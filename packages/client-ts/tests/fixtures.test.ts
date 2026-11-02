// The Ovis consumer fixtures replayed through the packed client, installed into a project
// outside the repository, against the real ovis-terminal binary and protocol fixture. The
// Rust runner replays the same files (`--scenario client/fixtures`); both must reproduce
// receipts.json, so their normalized receipts are equal.

import { afterAll, beforeAll, expect, test } from "bun:test";
import { copyFileSync, existsSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";

import { CONFORMANCE_BIN, PACKAGE, REPO, packed, run, scratch, succeed } from "./support/consumer";

const SETUP_BOUND_MS = 240_000;
const REPLAY_BOUND_MS = 180_000;
const FIXTURES = resolve(process.env.OVIS_REPLAY_FIXTURES ?? join(REPO, "conformance", "fixtures", "ovis"));
const TERMINAL_BIN = process.env.OVIS_TERMINAL_BIN ?? join(REPO, "target", "debug", "ovis-terminal");
const REPLAY_FILES = ["engine.ts", "replay.ts"];
const TYPE_ROOTS = dirname(dirname(Bun.resolveSync("@types/bun/package.json", PACKAGE)));

interface Replayed {
  readonly generation: string;
  readonly pins: readonly { path: string; expected: string; actual: string }[];
  readonly cases: Record<string, { receipt: unknown[]; failed: unknown[] }>;
}

let project = "";
let replayed: Replayed | undefined;

const result = (): Replayed => {
  if (replayed === undefined) throw new Error("the replay did not run");
  return replayed;
};

beforeAll(async () => {
  for (const binary of [TERMINAL_BIN, CONFORMANCE_BIN]) {
    if (!existsSync(binary)) throw new Error(`${binary} is missing: run \`cargo build --locked -p ovis-terminal-cli -p ovis-terminal-conformance\` first`);
  }
  const tarball = await packed();
  project = scratch("t64c-");
  writeFileSync(join(project, "package.json"), JSON.stringify({ name: "t64-replay", private: true, type: "module", dependencies: { "@ovis-terminal/client": `file:${tarball}` } }));
  writeFileSync(
    join(project, "tsconfig.json"),
    JSON.stringify({
      compilerOptions: { target: "ES2022", module: "ESNext", moduleResolution: "bundler", lib: ["ES2022"], types: ["bun"], typeRoots: [TYPE_ROOTS], strict: true, noUncheckedIndexedAccess: true, exactOptionalPropertyTypes: true, skipLibCheck: true, noEmit: true },
      include: REPLAY_FILES,
    }),
  );
  for (const file of REPLAY_FILES) copyFileSync(join(PACKAGE, "tests", "replay", file), join(project, file));
  await succeed([process.execPath, "install"], project);
  const ran = await succeed([process.execPath, "replay.ts"], project, {
    OVIS_REPLAY_FIXTURES: FIXTURES,
    OVIS_REPLAY_REPO: REPO,
    OVIS_TERMINAL_BIN: resolve(TERMINAL_BIN),
    OVIS_CONFORMANCE_BIN: resolve(CONFORMANCE_BIN),
  });
  replayed = JSON.parse(ran.stdout.trim().split("\n").at(-1) ?? "") as Replayed;
}, SETUP_BOUND_MS + REPLAY_BOUND_MS);

afterAll(() => {
  if (project !== "") rmSync(project, { recursive: true, force: true });
});

test("every pin the Bun client computes equals the manifest, the same hashes the Rust runner checks", () => {
  const pins = result().pins;
  expect(pins.filter((pin) => pin.actual !== pin.expected)).toEqual([]);
  const files = readdirSync(join(FIXTURES, "cases")).map((name) => `cases/${name}`).sort();
  expect(pins.map((pin) => pin.path).filter((path) => path.startsWith("cases/")).sort()).toEqual(files);
  expect(result().generation).toBe("1");
});

test("every case replays through the packed client with no failed expectation", () => {
  const failed = Object.fromEntries(Object.entries(result().cases).filter(([, outcome]) => outcome.failed.length > 0));
  expect(failed).toEqual({});
});

test("the Bun receipts equal receipts.json, which the Rust runner reproduces too", () => {
  const expected = (JSON.parse(readFileSync(join(FIXTURES, "receipts.json"), "utf8")) as { cases: Record<string, unknown[]> }).cases;
  const actual = Object.fromEntries(Object.entries(result().cases).map(([id, outcome]) => [id, outcome.receipt]));
  expect(Object.keys(actual).sort()).toEqual(Object.keys(expected).sort());
  expect(actual).toEqual(expected);
});

test("the replay sources typecheck against the packed declarations", async () => {
  const tsc = Bun.resolveSync("typescript/bin/tsc", PACKAGE);
  const ran = await run([process.execPath, tsc, "-p", "tsconfig.json"], project);
  expect([ran.code, ran.stdout, ran.stderr]).toEqual([0, "", ""]);
}, SETUP_BOUND_MS);
