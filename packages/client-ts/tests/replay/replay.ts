// Runs in an isolated project that installed the packed tarball: replays every case of the
// Ovis consumer fixture set through @ovis-terminal/client against a fresh server per case
// and prints one JSON line with the pins it computed and each case's receipt and failures.

import { createHash } from "node:crypto";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { Client, RpcError } from "@ovis-terminal/client";

import { Bindings, type Case, type Check, type Json, type ReceiptLine, type Step, type Translation, check, eventLine, replyLine, translate } from "./engine";

const BOUND_MS = 20_000;
const env = (name: string): string => {
  const value = process.env[name];
  if (value === undefined) throw new Error(`${name} is required`);
  return value;
};
const FIXTURES = env("OVIS_REPLAY_FIXTURES");
const REPO = env("OVIS_REPLAY_REPO");

interface Pin {
  readonly path: string;
  readonly sha256: string;
}

interface Manifest {
  readonly generation: string;
  readonly schema: Pin;
  readonly sources: readonly Pin[];
  readonly files: readonly Pin[];
  readonly servers: Record<string, { readonly bindings: Record<string, Json> }>;
}

const readJson = <T>(path: string): T => JSON.parse(readFileSync(path, "utf8")) as T;
const digest = (path: string): string => createHash("sha256").update(readFileSync(path)).digest("hex");

function within<T>(promise: Promise<T>, what: string): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const guard = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error(`hang guard: ${what} did not happen within ${BOUND_MS} ms`)), BOUND_MS);
  });
  return Promise.race([promise, guard]).finally(() => clearTimeout(timer));
}

/** A fresh server in a fresh root, announced by its first stdout line, stopped by closing stdin. */
class Server {
  private constructor(
    private readonly child: Bun.Subprocess<"pipe", "pipe", "pipe">,
    private readonly root: string,
    readonly socket: string,
    readonly bearer: string,
  ) {}

  static async start(kind: Case["server"]): Promise<Server> {
    // Short Unix socket paths: the macOS per-user temp directory exceeds the 104-byte limit.
    const base = process.platform === "win32" ? tmpdir() : (process.env.OVIS_FIXTURE_ROOT ?? "/tmp");
    const root = mkdtempSync(join(base, "t64b-"));
    const argv =
      kind === "assembled"
        ? [env("OVIS_TERMINAL_BIN"), "serve", "--stdin-lifetime", "--root", join(root, "srv")]
        : [env("OVIS_CONFORMANCE_BIN"), "fixture", "--root", root];
    const child = Bun.spawn(argv, { cwd: root, stdin: "pipe", stdout: "pipe", stderr: "pipe" });
    const reader = child.stdout.getReader();
    let buffered = "";
    while (!buffered.includes("\n")) {
      const chunk = await within(reader.read(), "the server ready line");
      if (chunk.done) throw new Error(`${argv.join(" ")} exited before it was ready: ${await new Response(child.stderr).text()}`);
      buffered += new TextDecoder().decode(chunk.value);
    }
    reader.releaseLock();
    const ready = (JSON.parse(buffered.slice(0, buffered.indexOf("\n"))) as { ready: { socket: string; bearer: string } }).ready;
    return new Server(child, root, ready.socket, ready.bearer);
  }

  async stop(): Promise<void> {
    this.child.stdin.end();
    await within(this.child.exited, "the server exit");
    rmSync(this.root, { recursive: true, force: true });
  }
}

/** One persistent client per connection label, every notification queued from the start. */
class Transport {
  private readonly clients = new Map<string, { client: Client; queued: Json[]; waiter?: () => void }>();

  constructor(
    private readonly server: Server,
    private readonly methods: readonly string[],
  ) {}

  private async conn(label: string): Promise<{ client: Client; queued: Json[]; waiter?: () => void }> {
    const existing = this.clients.get(label);
    if (existing !== undefined) return existing;
    const client = await Client.connect({ socketPath: this.server.socket, credential: this.server.bearer, client: { name: `ovis-replay-${label}`, version: "1" } });
    const entry: { client: Client; queued: Json[]; waiter?: () => void } = { client, queued: [] };
    for (const method of this.methods) {
      client.on(method, (params) => {
        entry.queued.push({ method, params: params as Json });
        entry.waiter?.();
      });
    }
    this.clients.set(label, entry);
    return entry;
  }

  async call(label: string, method: string, params: Json): Promise<Json> {
    const { client } = await this.conn(label);
    try {
      return { result: (await within(client.unchecked(method, params), `the reply to ${method}`)) as Json };
    } catch (error) {
      if (!(error instanceof RpcError)) throw error;
      return { error: { code: error.code, message: error.message, data: error.data as unknown as Json } };
    }
  }

  async event(label: string, method: string): Promise<Json> {
    const entry = await this.conn(label);
    for (;;) {
      const at = entry.queued.findIndex((record) => (record as { method: string }).method === method);
      if (at !== -1) {
        const [record] = entry.queued.splice(0, at + 1).slice(-1);
        return record as Json;
      }
      await within(new Promise<void>((resolve) => (entry.waiter = resolve)), `a ${method} notification`);
    }
  }

  close(): void {
    for (const { client } of this.clients.values()) client.close();
  }
}

async function replayCase(kase: Case, legacy: Record<string, Translation>, bindings: Bindings): Promise<{ receipt: ReceiptLine[]; failed: Check[] }> {
  const server = await Server.start(kase.server);
  const methods = [...new Set(kase.steps.flatMap((step: Step) => ("event" in step ? [step.event.method] : [])))];
  const transport = new Transport(server, methods);
  const receipt: ReceiptLine[] = [];
  const failed: Check[] = [];
  try {
    for (const step of kase.steps) {
      let checks: Check[];
      if ("event" in step) {
        const record = await transport.event(step.event.conn, step.event.method);
        receipt.push(eventLine(step.event.method, record));
        checks = check(step.event.expect, `event:${step.event.method}`, record, bindings);
      } else {
        const call = "call" in step ? step.call : step.legacy;
        const [method, params] = "call" in step ? [call.method, call.params] : translate(legacy, call.id, call.method, call.params);
        const reply = await transport.call(call.conn, method, bindings.resolve(params));
        receipt.push(replyLine(call.id, method, reply));
        checks = check(call.expect, call.id, reply, bindings);
      }
      failed.push(...checks.filter((one) => !one.passed).map((one) => ({ ...one, check: `${kase.id}/${one.check}` })));
      if (failed.length > 0) break;
    }
  } finally {
    transport.close();
    await server.stop();
  }
  return { receipt, failed };
}

const manifest = readJson<Manifest>(join(FIXTURES, "manifest.json"));
const pins = [
  ...[manifest.schema, ...manifest.sources].map((pin) => ({ path: pin.path, expected: pin.sha256, actual: digest(join(REPO, pin.path)) })),
  ...manifest.files.map((pin) => ({ path: pin.path, expected: pin.sha256, actual: digest(join(FIXTURES, pin.path)) })),
];
const legacy = readJson<{ methods: Record<string, Translation> }>(join(FIXTURES, "legacy.json")).methods;
const schemaVersion = readJson<{ info: { version: string } }>(join(REPO, manifest.schema.path)).info.version;
const cases: Record<string, { receipt: ReceiptLine[]; failed: Check[] }> = {};
for (const pin of manifest.files.filter((file) => file.path.startsWith("cases/"))) {
  const kase = readJson<Case>(join(FIXTURES, pin.path));
  const server = manifest.servers[kase.server]?.bindings ?? {};
  cases[kase.id] = await replayCase(kase, legacy, new Bindings({ ...server, schema_version: schemaVersion, generation: manifest.generation }));
}
console.log(JSON.stringify({ generation: manifest.generation, pins, cases }));
