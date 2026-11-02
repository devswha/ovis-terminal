// The runnable examples of docs/ovis-adapter.md replayed through the packed client, installed
// into a project outside the repository, against the real ovis-terminal binary and protocol
// fixture. The `json ovis-example` blocks are staged as an ovis-replay/1 fixture set (the
// legacy map, bindings and schema pins of conformance/fixtures/ovis) and run by the same
// tests/replay sources as fixtures.test.ts. The Rust runner replays the same blocks
// (`--scenario client/migration`). OVIS_GUIDE replays another guide.

import { afterAll, beforeAll, expect, test } from "bun:test";
import { copyFileSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { dirname, join, resolve } from "node:path";

import { CONFORMANCE_BIN, PACKAGE, REPO, packed, run, scratch, succeed } from "./support/consumer";

const SETUP_BOUND_MS = 240_000;
const REPLAY_BOUND_MS = 180_000;
const GUIDE = resolve(process.env.OVIS_GUIDE ?? join(REPO, "docs", "ovis-adapter.md"));
const FIXTURES = join(REPO, "conformance", "fixtures", "ovis");
const TERMINAL_BIN = process.env.OVIS_TERMINAL_BIN ?? join(REPO, "target", "debug", "ovis-terminal");
const REPLAY_FILES = ["engine.ts", "replay.ts"];
const TYPE_ROOTS = dirname(dirname(Bun.resolveSync("@types/bun/package.json", PACKAGE)));
const OPEN = "```json ovis-example";
const CLOSE = "```";

interface Replayed {
  readonly pins: readonly { path: string; expected: string; actual: string }[];
  readonly cases: Record<string, { receipt: unknown[]; failed: unknown[] }>;
}

/** Every `json ovis-example` block of the guide, parsed, in document order. */
function examples(text: string): { id: string; body: string }[] {
  const found: { id: string; body: string }[] = [];
  let body: string[] | undefined;
  for (const line of text.split("\n")) {
    if (body === undefined) {
      if (line.trim() === OPEN) body = [];
    } else if (line.trim() === CLOSE) {
      const source = body.join("\n");
      found.push({ id: (JSON.parse(source) as { id: string }).id, body: source });
      body = undefined;
    } else {
      body.push(line);
    }
  }
  if (body !== undefined) throw new Error(`${GUIDE}: an ovis-example block is never closed`);
  return found;
}

/** A fixture directory holding only the guide's examples, pinned by their own bytes. */
function stage(dir: string, found: readonly { id: string; body: string }[]): void {
  const manifest = JSON.parse(readFileSync(join(FIXTURES, "manifest.json"), "utf8")) as Record<string, unknown>;
  mkdirSync(join(dir, "cases"));
  const files = found.map(({ id, body }) => {
    const path = `cases/${id}.json`;
    writeFileSync(join(dir, path), body);
    return { path, sha256: createHash("sha256").update(body).digest("hex") };
  });
  writeFileSync(join(dir, "manifest.json"), JSON.stringify({ ...manifest, files }));
  copyFileSync(join(FIXTURES, "legacy.json"), join(dir, "legacy.json"));
}

const cited = examples(readFileSync(GUIDE, "utf8"));
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
  project = scratch("t65g-");
  const staged = join(project, "guide-fixtures");
  mkdirSync(staged);
  stage(staged, cited);
  writeFileSync(join(project, "package.json"), JSON.stringify({ name: "t65-guide", private: true, type: "module", dependencies: { "@ovis-terminal/client": `file:${tarball}` } }));
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
    OVIS_REPLAY_FIXTURES: staged,
    OVIS_REPLAY_REPO: REPO,
    OVIS_TERMINAL_BIN: resolve(TERMINAL_BIN),
    OVIS_CONFORMANCE_BIN: resolve(CONFORMANCE_BIN),
  });
  replayed = JSON.parse(ran.stdout.trim().split("\n").at(-1) ?? "") as Replayed;
}, SETUP_BOUND_MS + REPLAY_BOUND_MS);

afterAll(() => {
  if (project !== "") rmSync(project, { recursive: true, force: true });
});

test("every example the guide cites replays through the packed client", () => {
  expect(cited.length).toBeGreaterThan(0);
  expect(Object.keys(result().cases).sort()).toEqual(cited.map(({ id }) => id).sort());
});

test("no guide example has a failed expectation through the packed client", () => {
  const failed = Object.fromEntries(Object.entries(result().cases).filter(([, outcome]) => outcome.failed.length > 0));
  expect(failed).toEqual({});
});

test("the staged examples are the guide's bytes and the schema pin is the published schema", () => {
  expect(result().pins.filter((pin) => pin.actual !== pin.expected)).toEqual([]);
});

test("the replay sources typecheck against the packed declarations", async () => {
  const tsc = Bun.resolveSync("typescript/bin/tsc", PACKAGE);
  const ran = await run([process.execPath, tsc, "-p", "tsconfig.json"], project);
  expect([ran.code, ran.stdout, ran.stderr]).toEqual([0, "", ""]);
}, SETUP_BOUND_MS);
