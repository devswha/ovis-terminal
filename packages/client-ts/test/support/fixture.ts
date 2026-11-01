// The real protocol fixture: the server crate, dispatcher, authority and SQLite store that
// the Rust conformance runner serves, started from its prebuilt binary on a private socket.

import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { within } from "./bounded";

export const REPO = join(import.meta.dir, "..", "..", "..", "..");
export const CONFORMANCE_BIN = process.env.OVIS_CONFORMANCE_BIN ?? join(REPO, "target", "debug", "conformance");
export const OVIS_TERMINAL_BIN = process.env.OVIS_TERMINAL_BIN ?? join(REPO, "target", "debug", "ovis-terminal");

export interface Ready {
  readonly socket: string;
  readonly bearer: string;
  readonly observer: string;
}

export interface ServeReport {
  readonly accepted: string;
  readonly failures: Readonly<Record<string, string>>;
  readonly panicked: string;
}

export async function requireBinary(path: string): Promise<void> {
  if (!(await Bun.file(path).exists())) {
    throw new Error(`${path} is missing: run \`cargo build --locked -p ovis-terminal-cli -p ovis-terminal-conformance\` first`);
  }
}

export class Fixture {
  private constructor(
    private readonly child: Bun.Subprocess<"pipe", "pipe", "pipe">,
    private readonly lines: AsyncIterator<string>,
    private readonly root: string,
    readonly ready: Ready,
  ) {}

  static async start(): Promise<Fixture> {
    await requireBinary(CONFORMANCE_BIN);
    const root = mkdtempSync(join(tmpdir(), "t29-"));
    const child = Bun.spawn([CONFORMANCE_BIN, "fixture", "--root", root], { stdin: "pipe", stdout: "pipe", stderr: "pipe" });
    const lines = readLines(child.stdout);
    const first = await within(lines.next(), "the fixture ready line");
    if (first.done === true) {
      throw new Error(`the fixture exited before it was ready: ${await new Response(child.stderr).text()}`);
    }
    const ready = (JSON.parse(first.value) as { ready: Ready }).ready;
    return new Fixture(child, lines, root, ready);
  }

  /** Closes the fixture's stdin, which stops it, and returns its serve report. */
  async stop(): Promise<ServeReport> {
    this.child.stdin.end();
    const next = await within(this.lines.next(), "the fixture serve report");
    const code = await within(this.child.exited, "the fixture exit");
    rmSync(this.root, { recursive: true, force: true });
    if (next.done === true || code !== 0) {
      throw new Error(`fixture stopped with ${code}: ${await new Response(this.child.stderr).text()}`);
    }
    return (JSON.parse(next.value) as { report: ServeReport }).report;
  }
}

async function* readLines(stream: ReadableStream<Uint8Array>): AsyncGenerator<string> {
  const decoder = new TextDecoder();
  let buffered = "";
  for await (const chunk of stream) {
    buffered += decoder.decode(chunk, { stream: true });
    for (let end = buffered.indexOf("\n"); end !== -1; end = buffered.indexOf("\n")) {
      yield buffered.slice(0, end);
      buffered = buffered.slice(end + 1);
    }
  }
}
