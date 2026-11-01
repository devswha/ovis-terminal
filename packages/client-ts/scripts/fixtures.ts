// `bun run test:fixtures -- --fixtures <dir>`: the fixture replay test against <dir>.

import { parseArgs } from "node:util";
import { resolve } from "node:path";

const { values } = parseArgs({ args: Bun.argv.slice(2), options: { fixtures: { type: "string" } }, strict: true });
const env = values.fixtures === undefined ? process.env : { ...process.env, OVIS_REPLAY_FIXTURES: resolve(values.fixtures) };
const child = Bun.spawn([process.execPath, "test", "./tests/fixtures.test.ts"], { cwd: resolve(import.meta.dir, ".."), env, stdout: "inherit", stderr: "inherit" });
process.exit(await child.exited);
