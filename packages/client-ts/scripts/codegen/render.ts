// Maps the JSON Schema subset the OpenRPC document uses to TypeScript type expressions.

export interface Schema {
  readonly $ref?: string;
  readonly type?: string | readonly string[];
  readonly const?: unknown;
  readonly enum?: readonly unknown[];
  readonly oneOf?: readonly Schema[];
  readonly anyOf?: readonly Schema[];
  readonly properties?: Readonly<Record<string, Schema>>;
  readonly required?: readonly string[];
  readonly additionalProperties?: boolean | Schema;
  readonly items?: Schema;
  readonly pattern?: string;
  readonly description?: string;
  readonly "x-ovis-open-enum"?: boolean;
}

export const DECIMAL_PATTERN = "^(0|[1-9][0-9]{0,19})$";
const REF_PREFIX = "#/components/schemas/";

export class GenerateError extends Error {
  readonly kind: "unsupported_schema" | "name_collision" | "stale_output";
  readonly hint: string;

  constructor(kind: GenerateError["kind"], message: string) {
    super(message);
    this.name = "GenerateError";
    this.kind = kind;
    this.hint =
      kind === "stale_output" ? "run `bun run generate` and commit the result" : "extend the generator before publishing this schema";
  }
}

export function refName(ref: string): string {
  if (!ref.startsWith(REF_PREFIX)) {
    throw new GenerateError("unsupported_schema", `reference ${ref} is outside components.schemas`);
  }
  const name = ref.slice(REF_PREFIX.length);
  if (!/^[A-Z][A-Za-z0-9]*$/.test(name)) {
    throw new GenerateError("unsupported_schema", `component name ${name} is not a TypeScript identifier`);
  }
  return name;
}

export function literals(schema: Schema): readonly string[] | undefined {
  if (schema.enum !== undefined && schema.enum.every((value) => typeof value === "string")) {
    return schema.enum as readonly string[];
  }
  const branches = schema.oneOf;
  if (branches !== undefined && branches.length > 0 && branches.every((branch) => typeof branch.const === "string")) {
    return branches.map((branch) => branch.const as string);
  }
  return undefined;
}

function union(parts: readonly string[]): string {
  const unique = [...new Set(parts)];
  return unique.length === 1 ? (unique[0] ?? "never") : unique.map((part) => `(${part})`).join(" | ");
}

export class Renderer {
  constructor(private readonly qualifier: string) {}

  render(schema: Schema): string {
    const open = schema["x-ovis-open-enum"] === true;
    if (schema.$ref !== undefined) {
      const name = `${this.qualifier}${refName(schema.$ref)}`;
      return open ? `Open<${name}>` : name;
    }
    if (schema.const !== undefined) {
      return JSON.stringify(schema.const);
    }
    const closed = literals(schema);
    if (closed !== undefined) {
      const body = union(closed.map((value) => JSON.stringify(value)));
      return open ? `Open<${body}>` : body;
    }
    if (schema.oneOf !== undefined) {
      return union(schema.oneOf.map((branch) => this.render(branch)));
    }
    if (schema.anyOf !== undefined) {
      return union(schema.anyOf.map((branch) => this.render(branch)));
    }
    const types = typeof schema.type === "string" ? [schema.type] : (schema.type ?? []);
    if (types.length === 0) {
      return schema.properties === undefined ? "unknown" : this.object(schema);
    }
    return union(types.map((type) => this.typed(type, schema)));
  }

  private typed(type: string, schema: Schema): string {
    switch (type) {
      case "string":
        return schema.pattern === DECIMAL_PATTERN ? "Decimal<string>" : "string";
      case "integer":
      case "number":
        return "number";
      case "boolean":
        return "boolean";
      case "null":
        return "null";
      case "array":
        return `readonly (${schema.items === undefined ? "unknown" : this.render(schema.items)})[]`;
      case "object":
        return this.object(schema);
      default:
        throw new GenerateError("unsupported_schema", `JSON Schema type ${type} has no TypeScript mapping`);
    }
  }

  private object(schema: Schema): string {
    const required = new Set(schema.required ?? []);
    const fields = Object.entries(schema.properties ?? {}).map(([key, value]) => {
      const optional = required.has(key) ? "" : "?";
      return `readonly ${JSON.stringify(key)}${optional}: ${this.render(value)}`;
    });
    const extra = schema.additionalProperties;
    const record = typeof extra === "object" ? `Readonly<Record<string, ${this.render(extra)}>>` : undefined;
    if (fields.length === 0) {
      return record ?? "Readonly<Record<string, never>>";
    }
    const body = `{ ${fields.join("; ")} }`;
    return record === undefined ? body : `${body} & ${record}`;
  }
}
