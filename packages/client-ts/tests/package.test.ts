import { afterAll, beforeAll, expect, test } from "bun:test";
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync } from "node:fs";
import { join } from "node:path";

import { PACKAGE, REPO, packed, run, scratch, succeed } from "./support/consumer";

const BUILD_BOUND_MS = 180_000;
let tarball = "";
const scratchDirs: string[] = [];

const entries = async () => (await succeed(["tar", "tzf", tarball], PACKAGE)).stdout.trim().split("\n");
const member = async (path: string) => (await succeed(["tar", "xzf", tarball, "-O", `package/${path}`], PACKAGE)).stdout;

beforeAll(async () => {
  tarball = await packed();
}, BUILD_BOUND_MS);

afterAll(() => {
  for (const dir of scratchDirs) rmSync(dir, { recursive: true, force: true });
});

test("the tarball carries only built output, the license, the readme and the manifest", async () => {
  const listed = await entries();
  const outside = listed.filter((path) => !path.startsWith("package/dist/") && !["package/package.json", "package/LICENSE", "package/README.md"].includes(path));
  expect(outside).toEqual([]);
  for (const required of ["dist/index.js", "dist/protocol.js", "dist/types/index.d.ts", "dist/types/protocol.d.ts", "dist/openrpc.json"]) {
    expect([required, listed.includes(`package/${required}`)]).toEqual([required, true]);
  }
  expect(listed.filter((path) => path.endsWith(".ts") && !path.endsWith(".d.ts"))).toEqual([]);
});

test("the packed manifest's exports, types, files, engines and license resolve inside the tarball", async () => {
  const listed = new Set(await entries());
  const manifest = JSON.parse(await member("package.json")) as {
    name: string;
    license: string;
    type: string;
    private: boolean;
    engines: Record<string, string>;
    files: string[];
    main: string;
    types: string;
    exports: Record<string, string | Record<string, string>>;
  };
  expect([manifest.name, manifest.license, manifest.type, manifest.engines.bun]).toEqual(["@ovis-terminal/client", "MIT", "module", ">=1.4.3"]);
  expect(manifest.private).toBe(true);
  expect(manifest.files.sort()).toEqual(["LICENSE", "README.md", "dist"]);
  expect(Object.keys(manifest.exports).sort()).toEqual([".", "./openrpc.json", "./package.json", "./protocol"]);
  const targets = [manifest.main, manifest.types, ...Object.values(manifest.exports).flatMap((target) => (typeof target === "string" ? [target] : Object.values(target)))];
  const missing = targets.filter((target) => !listed.has(`package/${target.replace(/^\.\//, "")}`));
  expect(missing).toEqual([]);
});

test("the exported OpenRPC document is the Rust-emitted schema byte for byte", async () => {
  expect(await member("dist/openrpc.json")).toBe(readFileSync(join(REPO, "schema", "ovis-terminal.openrpc.json"), "utf8"));
  expect(await member("LICENSE")).toBe(readFileSync(join(REPO, "LICENSE"), "utf8"));
});

test(
  "building and packing again reproduces the tarball byte for byte",
  async () => {
    const into = scratch("t63k-");
    scratchDirs.push(into);
    await succeed([process.execPath, "run", "build"], PACKAGE);
    const repacked = JSON.parse((await succeed([process.execPath, "run", "pack", "--destination", into], PACKAGE)).stdout.trim().split("\n").at(-1) ?? "") as {
      tarball: string;
      sha256: string;
    };
    const original = new Bun.CryptoHasher("sha256").update(readFileSync(tarball)).digest("hex");
    expect(repacked.tarball).not.toBe(tarball);
    expect(repacked.sha256).toBe(original);
  },
  BUILD_BOUND_MS,
);

test(
  "build refuses a schema the committed generated modules do not match, writing nothing",
  async () => {
    const root = scratch("t63s-");
    scratchDirs.push(root);
    const copy = join(root, "packages", "client-ts");
    mkdirSync(join(root, "schema"), { recursive: true });
    cpSync(join(PACKAGE, "src"), join(copy, "src"), { recursive: true });
    cpSync(join(PACKAGE, "scripts"), join(copy, "scripts"), { recursive: true });
    const schema = JSON.parse(readFileSync(join(REPO, "schema", "ovis-terminal.openrpc.json"), "utf8")) as { info: { version: string } };
    schema.info.version = `${schema.info.version}-drifted`;
    await Bun.write(join(root, "schema", "ovis-terminal.openrpc.json"), JSON.stringify(schema));
    const ran = await run([process.execPath, join(copy, "scripts", "build.ts")], copy);
    expect([ran.code, (JSON.parse(ran.stderr.trim()) as { kind: string }).kind]).toEqual([1, "stale_generated"]);
    expect(existsSync(join(copy, "dist"))).toBe(false);
  },
  BUILD_BOUND_MS,
);
