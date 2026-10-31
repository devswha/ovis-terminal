import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { type Document, generate } from "../scripts/codegen/modules";
import { GenerateError, Renderer } from "../scripts/codegen/render";
import { methods } from "../src/generated/methods";

const SCHEMA = join(import.meta.dir, "..", "..", "..", "schema", "ovis-terminal.openrpc.json");
const source = readFileSync(SCHEMA, "utf8");
const document = JSON.parse(source) as Document;

const failure = (run: () => unknown) => {
  try {
    run();
  } catch (error) {
    return (error as GenerateError).kind;
  }
  return "generated";
};

test("every published method lands in exactly one namespace module and in the runtime catalog", () => {
  const files = generate(source);
  const names = document.methods.map((method) => method.name).sort();
  const declaredIn = names.map((name) => [...files].filter(([path, body]) => path.startsWith("methods/") && path !== "methods/index.ts" && body.includes(`readonly ${JSON.stringify(name)}:`)).map(([path]) => path));
  expect(declaredIn.every((paths, index) => paths.length === 1 && paths[0] === `methods/${names[index]?.split(".")[0]}.ts`)).toBe(true);
  expect(Object.keys(methods).sort()).toEqual(names);
  expect(files.get("openrpc.json")).toBe(source);
});

test("generation is deterministic and the committed output is that generation", () => {
  const first = generate(source);
  expect([...generate(source)]).toEqual([...first]);
  for (const [path, body] of first) {
    expect([path, readFileSync(join(import.meta.dir, "..", "src", "generated", path), "utf8") === body]).toEqual([path, true]);
  }
});

test("open-enum sites, decimals and identities render as distinct nominal types", () => {
  const body = generate(source).get("components.ts") ?? "";
  expect(body).toContain('export type ByteOffset = Decimal<"ByteOffset">;');
  expect(body).toContain('export type StreamId = Id<"StreamId">;');
  expect(body).toMatch(/"kind": Open<ControlKind>/);
  expect(new Renderer("").render({ $ref: "#/components/schemas/StreamMode" })).toBe("StreamMode");
});

test("an unsupported schema or a colliding name fails generation with a typed kind", () => {
  expect(failure(() => new Renderer("").render({ type: "tuple" }))).toBe("unsupported_schema");
  expect(failure(() => new Renderer("").render({ $ref: "#/definitions/Elsewhere" }))).toBe("unsupported_schema");
  const colliding = { ...document, components: { schemas: { ...document.components.schemas, ControlKindValues: { type: "string" } } } };
  expect(failure(() => generate(JSON.stringify(colliding)))).toBe("name_collision");
});
