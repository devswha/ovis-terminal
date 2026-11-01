// Checks records against the shipped OpenRPC document: inputs are closed and strict;
// outputs tolerate added fields and added values at open-enum sites.

import document from "./generated/openrpc.json";
import { ClientError } from "./errors";

interface Node {
  readonly $ref?: string;
  readonly type?: string | readonly string[];
  readonly const?: unknown;
  readonly enum?: readonly unknown[];
  readonly oneOf?: readonly Node[];
  readonly anyOf?: readonly Node[];
  readonly properties?: Readonly<Record<string, Node>>;
  readonly required?: readonly string[];
  readonly additionalProperties?: boolean | Node;
  readonly items?: Node;
  readonly pattern?: string;
  readonly format?: string;
  readonly minimum?: number;
  readonly maximum?: number;
  readonly "x-ovis-open-enum"?: boolean;
}

interface MethodEntry {
  readonly name: string;
  readonly "x-params": Node;
  readonly result: { readonly schema: Node };
}

export type Direction = "input" | "output";

export interface Violation {
  readonly path: string;
  readonly reason: string;
}

const schemas = (document.components.schemas as unknown) as Readonly<Record<string, Node>>;
const byMethod = new Map((document.methods as unknown as readonly MethodEntry[]).map((method) => [method.name, method]));
const notifications = new Map(
  (document["x-notifications"] as unknown as readonly { name: string; schema: Node }[]).map((entry) => [entry.name, entry.schema]),
);
const patterns = new Map<string, RegExp>();

function resolve(node: Node): Node {
  const ref = node.$ref;
  if (ref === undefined) {
    return node;
  }
  const target = schemas[ref.slice("#/components/schemas/".length)];
  if (target === undefined) {
    throw new ClientError("invalid_response", `schema reference ${ref} does not resolve`);
  }
  return target;
}

function typeOf(value: unknown): string {
  if (value === null) return "null";
  if (Array.isArray(value)) return "array";
  if (typeof value === "number") return Number.isInteger(value) ? "integer" : "number";
  return typeof value;
}

function typeMatches(declared: string, value: unknown): boolean {
  const actual = typeOf(value);
  return declared === actual || (declared === "number" && actual === "integer");
}

function branches(options: readonly Node[], value: unknown, at: Checker): Violation | undefined {
  const failures: Violation[] = [];
  for (const option of options) {
    const failure = at.check(option, value);
    if (failure === undefined) return undefined;
    failures.push(failure);
  }
  const deeper = failures.filter((failure) => failure.path !== at.path);
  const first = deeper[0];
  const shared = first !== undefined && deeper.every((failure) => failure.path === first.path);
  return shared ? first : { path: at.path, reason: "matches none of the allowed shapes" };
}

class Checker {
  constructor(
    readonly path: string,
    private readonly direction: Direction,
  ) {}

  private at(child: string): Checker {
    return new Checker(child, this.direction);
  }

  check(raw: Node, value: unknown): Violation | undefined {
    if (this.direction === "output" && raw["x-ovis-open-enum"] === true && typeof value === "string") {
      return undefined;
    }
    const node = resolve(raw);
    if (node !== raw) return this.check(node, value);
    if (node.const !== undefined) return value === node.const ? undefined : this.fail(`must be ${JSON.stringify(node.const)}`);
    if (node.enum !== undefined) return node.enum.includes(value) ? undefined : this.fail("is not an allowed value");
    if (node.oneOf !== undefined) return branches(node.oneOf, value, this);
    if (node.anyOf !== undefined) return branches(node.anyOf, value, this);
    const types = typeof node.type === "string" ? [node.type] : (node.type ?? []);
    if (types.length > 0 && !types.some((type) => typeMatches(type, value))) {
      return this.fail(`must be ${types.join(" or ")}`);
    }
    if (typeof value === "string") return this.string(node, value);
    if (typeof value === "number") return this.number(node, value);
    if (Array.isArray(value)) return this.array(node, value);
    if (typeof value === "object" && value !== null) return this.object(node, value as Record<string, unknown>);
    return undefined;
  }

  private fail(reason: string): Violation {
    return { path: this.path, reason };
  }

  private string(node: Node, value: string): Violation | undefined {
    if (node.pattern === undefined) return undefined;
    const pattern = patterns.get(node.pattern) ?? new RegExp(node.pattern, "u");
    patterns.set(node.pattern, pattern);
    return pattern.test(value) ? undefined : this.fail(`must match ${node.pattern}`);
  }

  private number(node: Node, value: number): Violation | undefined {
    const integer = node.format === "uint32" || node.format === "int32";
    if (integer && !Number.isInteger(value)) return this.fail("must be an integer");
    const minimum = node.minimum ?? (node.format === "uint32" ? 0 : -(2 ** 31));
    const maximum = node.maximum ?? (node.format === "uint32" ? 2 ** 32 - 1 : 2 ** 31 - 1);
    const bounded = integer || node.minimum !== undefined || node.maximum !== undefined;
    return bounded && (value < minimum || value > maximum) ? this.fail(`must be within ${minimum}..${maximum}`) : undefined;
  }

  private array(node: Node, value: readonly unknown[]): Violation | undefined {
    if (node.items === undefined) return undefined;
    for (const [index, item] of value.entries()) {
      const failure = this.at(`${this.path}[${index}]`).check(node.items, item);
      if (failure !== undefined) return failure;
    }
    return undefined;
  }

  private object(node: Node, value: Record<string, unknown>): Violation | undefined {
    const properties = node.properties ?? {};
    const extra = node.additionalProperties;
    for (const key of Object.keys(value)) {
      const known = Object.hasOwn(properties, key);
      if (!known && extra === false && this.direction === "input") return this.at(`${this.path}.${key}`).fail("is not a known field");
    }
    for (const key of node.required ?? []) {
      if (!Object.hasOwn(value, key)) return this.at(`${this.path}.${key}`).fail("is required");
    }
    for (const [key, item] of Object.entries(value)) {
      const property = properties[key] ?? (typeof extra === "object" ? extra : undefined);
      const failure = property === undefined ? undefined : this.at(`${this.path}.${key}`).check(property, item);
      if (failure !== undefined) return failure;
    }
    return undefined;
  }
}

export function knownMethod(method: string): boolean {
  return byMethod.has(method);
}

/** The first way `params` breaks the closed parameter schema of `method`, if any. */
export function checkParams(method: string, params: unknown): Violation | undefined {
  const entry = byMethod.get(method);
  if (entry === undefined) return { path: "method", reason: "is not a published method" };
  return new Checker("params", "input").check(entry["x-params"], params);
}

/** The first way a reply breaks the published result schema of `method`, if any. */
export function checkResult(method: string, result: unknown): Violation | undefined {
  const entry = byMethod.get(method);
  return entry === undefined ? undefined : new Checker("result", "output").check(entry.result.schema, result);
}

/** The first way a server notification breaks its published schema, if it is published. */
export function checkNotification(method: string, params: unknown): Violation | undefined {
  const schema = notifications.get(method);
  return schema === undefined ? undefined : new Checker("params", "output").check(schema, params);
}

/** The first way `value` breaks the named component in the given direction, if any. */
export function checkComponent(name: string, value: unknown, direction: Direction): Violation | undefined {
  return new Checker(name, direction).check({ $ref: `#/components/schemas/${name}` }, value);
}
