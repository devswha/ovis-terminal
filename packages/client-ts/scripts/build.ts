// `bun run build`: regenerates the protocol modules from the Rust-emitted OpenRPC document,
// refuses a stale src/generated, then writes dist/ (ESM bundles, declarations, OpenRPC JSON).

import { copyFileSync, readFileSync, rmSync } from "node:fs";
import { join } from "node:path";

import { generate } from "./codegen/modules";
import { PackageError, exitWith } from "./package/error";

const PACKAGE = join(import.meta.dir, "..");
const SCHEMA = join(PACKAGE, "..", "..", "schema", "ovis-terminal.openrpc.json");
const GENERATED = join(PACKAGE, "src", "generated");
const DIST = join(PACKAGE, "dist");
const ENTRIES = ["index", "protocol"] as const;

function requireFreshGenerated(): number {
  const want = generate(readFileSync(SCHEMA, "utf8"));
  const stale = [...want].filter(([path, body]) => {
    try {
      return readFileSync(join(GENERATED, path), "utf8") !== body;
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") return true;
      throw error;
    }
  });
  if (stale.length > 0) {
    throw new PackageError("stale_generated", `src/generated differs from the schema: ${stale.map(([path]) => path).join(", ")}`);
  }
  return want.size;
}

async function bundle(): Promise<void> {
  const built = await Bun.build({
    entrypoints: ENTRIES.map((entry) => join(PACKAGE, "src", `${entry}.ts`)),
    outdir: DIST,
    target: "bun",
    format: "esm",
    splitting: false,
    minify: false,
    sourcemap: "none",
    naming: "[name].[ext]",
    throw: false,
  });
  if (!built.success) {
    throw new PackageError("bundle_failed", built.logs.map((log) => String(log)).join("\n"));
  }
}

async function declarations(): Promise<void> {
  const tsc = Bun.resolveSync("typescript/bin/tsc", PACKAGE);
  const child = Bun.spawn([process.execPath, tsc, "-p", join(PACKAGE, "tsconfig.build.json")], { cwd: PACKAGE, stdout: "pipe", stderr: "pipe" });
  const [code, stdout, stderr] = await Promise.all([child.exited, new Response(child.stdout).text(), new Response(child.stderr).text()]);
  if (code !== 0) {
    throw new PackageError("declarations_failed", `${stdout}${stderr}`);
  }
}

await exitWith(async () => {
  const generated = requireFreshGenerated();
  rmSync(DIST, { recursive: true, force: true });
  await bundle();
  await declarations();
  copyFileSync(join(GENERATED, "openrpc.json"), join(DIST, "openrpc.json"));
  return { built: "dist", generated_files: generated, entries: ENTRIES.map((entry) => `dist/${entry}.js`) };
});
