// `bun run pack [--destination <dir>]`: packs the built package into one reproducible tarball
// (default destination pack/) and reports its path and sha256. It never publishes.

import { existsSync, mkdirSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";

import { PackageError, exitWith } from "./package/error";

const PACKAGE = join(import.meta.dir, "..");

function destination(argv: readonly string[]): string {
  const at = argv.indexOf("--destination");
  const chosen = at === -1 ? undefined : argv[at + 1];
  return chosen === undefined ? join(PACKAGE, "pack") : resolve(chosen);
}

await exitWith(async () => {
  for (const built of ["dist/index.js", "dist/protocol.js", "dist/types/index.d.ts", "dist/openrpc.json"]) {
    if (!existsSync(join(PACKAGE, built))) throw new PackageError("not_built", `${built} is missing`);
  }
  const into = destination(process.argv);
  mkdirSync(into, { recursive: true });
  const child = Bun.spawn([process.execPath, "pm", "pack", "--destination", into, "--ignore-scripts", "--quiet"], { cwd: PACKAGE, stdout: "pipe", stderr: "pipe" });
  const [code, stdout, stderr] = await Promise.all([child.exited, new Response(child.stdout).text(), new Response(child.stderr).text()]);
  const tarball = stdout.trim().split("\n").at(-1) ?? "";
  if (code !== 0 || !existsSync(tarball)) {
    throw new PackageError("pack_failed", `bun pm pack exited ${code}: ${stdout}${stderr}`);
  }
  const sha256 = new Bun.CryptoHasher("sha256").update(readFileSync(tarball)).digest("hex");
  return { tarball, sha256 };
});
