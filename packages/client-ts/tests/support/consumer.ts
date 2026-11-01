// Packs the package (when no tarball exists yet) and installs that tarball into an isolated
// consumer project outside the repository; nothing here imports the package's source.

import { copyFileSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";

export const PACKAGE = join(import.meta.dir, "..", "..");
export const REPO = join(PACKAGE, "..", "..");
export const CONFORMANCE_BIN = process.env.OVIS_CONFORMANCE_BIN ?? join(REPO, "target", "debug", "conformance");
const VERSION = (JSON.parse(readFileSync(join(PACKAGE, "package.json"), "utf8")) as { version: string }).version;
export const TARBALL = join(PACKAGE, "pack", `ovis-terminal-client-${VERSION}.tgz`);
const CONSUMER_FILES = ["support.ts", "cases.ts", "types.ts"];
const PROCESS_BOUND_MS = 120_000;
const TSC = Bun.resolveSync("typescript/bin/tsc", PACKAGE);
const TYPE_ROOTS = dirname(dirname(Bun.resolveSync("@types/bun/package.json", PACKAGE)));

export interface Ran {
  readonly argv: readonly string[];
  readonly code: number;
  readonly stdout: string;
  readonly stderr: string;
}

export function scratch(prefix: string): string {
  return mkdtempSync(join(process.env.OVIS_FIXTURE_ROOT ?? tmpdir(), prefix));
}

/** Runs a child to completion within a hang guard and captures its output. */
export async function run(argv: readonly string[], cwd: string, env: Record<string, string> = {}): Promise<Ran> {
  const child = Bun.spawn([...argv], { cwd, env: { ...process.env, ...env }, stdout: "pipe", stderr: "pipe" });
  let timer: ReturnType<typeof setTimeout> | undefined;
  const guard = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error(`hang guard: ${argv.join(" ")} ran longer than ${PROCESS_BOUND_MS} ms`)), PROCESS_BOUND_MS);
  });
  const finished = Promise.all([child.exited, new Response(child.stdout).text(), new Response(child.stderr).text()]);
  try {
    const [code, stdout, stderr] = await Promise.race([finished, guard]);
    return { argv, code, stdout, stderr };
  } finally {
    clearTimeout(timer);
  }
}

/** A child that must exit 0; any other status fails with its argv, stdout and stderr. */
export async function succeed(argv: readonly string[], cwd: string, env: Record<string, string> = {}): Promise<Ran> {
  const ran = await run(argv, cwd, env);
  if (ran.code !== 0) {
    throw new Error(`${ran.argv.join(" ")} exited ${ran.code}\n--- stdout\n${ran.stdout}\n--- stderr\n${ran.stderr}`);
  }
  return ran;
}

/** The tarball `bun run pack` produced; built and packed first when it does not exist. */
export async function packed(): Promise<string> {
  if (!existsSync(TARBALL)) {
    await succeed([process.execPath, "run", "build"], PACKAGE);
    await succeed([process.execPath, "run", "pack"], PACKAGE);
  }
  return TARBALL;
}

export class Consumer {
  private constructor(readonly dir: string) {}

  /** A fresh project outside the repository whose only dependency is the tarball. */
  static async install(tarball: string): Promise<Consumer> {
    const dir = scratch("t63c-");
    writeFileSync(
      join(dir, "package.json"),
      JSON.stringify({ name: "t63-consumer", private: true, type: "module", dependencies: { "@ovis-terminal/client": `file:${tarball}` } }, null, 2),
    );
    writeFileSync(
      join(dir, "tsconfig.json"),
      JSON.stringify(
        {
          compilerOptions: {
            target: "ES2022",
            module: "ESNext",
            moduleResolution: "bundler",
            lib: ["ES2022"],
            types: ["bun"],
            typeRoots: [TYPE_ROOTS],
            strict: true,
            exactOptionalPropertyTypes: true,
            noUncheckedIndexedAccess: true,
            resolveJsonModule: true,
            skipLibCheck: true,
            noEmit: true,
          },
          include: CONSUMER_FILES,
        },
        null,
        2,
      ),
    );
    for (const file of CONSUMER_FILES) copyFileSync(join(PACKAGE, "tests", "consumer", file), join(dir, file));
    await succeed([process.execPath, "install"], dir);
    return new Consumer(dir);
  }

  /** Runs one case of cases.ts and returns the JSON result it printed. */
  async run(name: string): Promise<Record<string, unknown>> {
    const ran = await succeed([process.execPath, "cases.ts", name], this.dir, { OVIS_CONFORMANCE_BIN: CONFORMANCE_BIN });
    const last = ran.stdout.trim().split("\n").at(-1) ?? "";
    return JSON.parse(last) as Record<string, unknown>;
  }

  typecheck(): Promise<Ran> {
    return run([process.execPath, TSC, "-p", "tsconfig.json"], this.dir);
  }

  remove(): void {
    rmSync(this.dir, { recursive: true, force: true });
  }
}
