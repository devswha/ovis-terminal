// `bun scripts/generate.ts` writes src/generated/; `--check` exits 1 when it is stale.

import { mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";

import { generate } from "./codegen/modules";
import { GenerateError } from "./codegen/render";

const PACKAGE = join(import.meta.dir, "..");
const SCHEMA = join(PACKAGE, "..", "..", "schema", "ovis-terminal.openrpc.json");
const OUT = join(PACKAGE, "src", "generated");

function existing(dir: string): Map<string, string> {
  const found = new Map<string, string>();
  const walk = (at: string) => {
    for (const entry of readdirSync(at, { withFileTypes: true })) {
      const path = join(at, entry.name);
      if (entry.isDirectory()) {
        walk(path);
      } else {
        found.set(relative(dir, path), readFileSync(path, "utf8"));
      }
    }
  };
  try {
    walk(dir);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") {
      throw error;
    }
  }
  return found;
}

function stale(want: Map<string, string>, have: Map<string, string>): string[] {
  const paths = new Set([...want.keys(), ...have.keys()]);
  return [...paths].sort().filter((path) => want.get(path) !== have.get(path));
}

function main(check: boolean): number {
  const want = generate(readFileSync(SCHEMA, "utf8"));
  const differing = stale(want, existing(OUT));
  if (check) {
    if (differing.length === 0) {
      process.stdout.write(`${JSON.stringify({ generated: "fresh", files: want.size })}\n`);
      return 0;
    }
    const error = new GenerateError("stale_output", `src/generated differs from the schema: ${differing.join(", ")}`);
    process.stderr.write(`${JSON.stringify({ kind: error.kind, hint: error.hint, files: differing })}\n`);
    return 1;
  }
  rmSync(OUT, { recursive: true, force: true });
  for (const [path, body] of want) {
    mkdirSync(dirname(join(OUT, path)), { recursive: true });
    writeFileSync(join(OUT, path), body);
  }
  process.stdout.write(`${JSON.stringify({ generated: "written", files: want.size, changed: differing.length })}\n`);
  return 0;
}

process.exit(main(process.argv.includes("--check")));
