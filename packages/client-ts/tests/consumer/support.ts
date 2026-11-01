// Consumer-side helpers. This directory is copied into an isolated project that installs the
// packed tarball; everything here imports @ovis-terminal/client, never repository source.

import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import type { Socket } from "bun";
import { FrameDecoder, checkComponent, checkResult, encodeFrame, type Frame, limitDefaults } from "@ovis-terminal/client";

export const HANG_GUARD_MS = 20_000;
export const CONTROL_CHANNEL = 0;
const NEWLINE = 0x0a;

export function scratch(prefix: string): string {
  return mkdtempSync(join(process.env.OVIS_FIXTURE_ROOT ?? tmpdir(), prefix));
}

/** Bounds an event-driven wait; it never decides an outcome. */
export function within<T>(promise: Promise<T>, what: string): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const guard = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error(`hang guard: ${what} did not happen within ${HANG_GUARD_MS} ms`)), HANG_GUARD_MS);
  });
  return Promise.race([promise, guard]).finally(() => clearTimeout(timer));
}

/** Resolves with the first value `register` delivers; register before triggering the action. */
export function once<T>(register: (deliver: (value: T) => void) => () => void): Promise<T> {
  return new Promise((resolve) => {
    const stop = register((value) => {
      stop();
      resolve(value);
    });
  });
}

export async function rejection(promise: Promise<unknown>): Promise<unknown> {
  try {
    await promise;
  } catch (error) {
    return error;
  }
  throw new Error("expected a rejection, got a result");
}

class Queue<T> {
  private readonly items: T[] = [];
  private readonly waiters: ((item: T) => void)[] = [];

  push(item: T): void {
    const waiter = this.waiters.shift();
    if (waiter === undefined) this.items.push(item);
    else waiter(item);
  }

  next(what: string): Promise<T> {
    const item = this.items.shift();
    return item !== undefined ? Promise.resolve(item) : within(new Promise((resolve) => this.waiters.push(resolve)), what);
  }
}

class Lines {
  private buffered = Buffer.alloc(0);

  push(chunk: Uint8Array): string[] {
    this.buffered = Buffer.concat([this.buffered, chunk]);
    const lines: string[] = [];
    for (let end = this.buffered.indexOf(NEWLINE); end !== -1; end = this.buffered.indexOf(NEWLINE)) {
      lines.push(this.buffered.subarray(0, end).toString("utf8"));
      this.buffered = this.buffered.subarray(end + 1);
    }
    return lines;
  }
}

/** Writes with backpressure: bytes the socket did not take wait for its drain. */
class Writer {
  private queued: Uint8Array[] = [];

  constructor(private readonly socket: Socket<undefined>) {}

  write(bytes: Uint8Array): void {
    this.queued.push(bytes);
    this.flush();
  }

  flush(): void {
    while (this.queued.length > 0) {
      const head = this.queued[0] as Uint8Array;
      const written = this.socket.write(head);
      if (written < head.byteLength) {
        this.queued[0] = head.subarray(Math.max(written, 0));
        return;
      }
      this.queued.shift();
    }
  }
}

export interface Ready {
  readonly socket: string;
  readonly bearer: string;
}

/** The real Rust protocol fixture (server, dispatcher, authority, SQLite store) on a private socket. */
export class Fixture {
  private constructor(
    private readonly child: Bun.Subprocess<"pipe", "pipe", "pipe">,
    private readonly lines: AsyncIterator<string>,
    private readonly root: string,
    readonly ready: Ready,
  ) {}

  static async start(): Promise<Fixture> {
    const binary = process.env.OVIS_CONFORMANCE_BIN;
    if (binary === undefined) throw new Error("OVIS_CONFORMANCE_BIN names the conformance binary");
    const root = scratch("t63f-");
    const child = Bun.spawn([binary, "fixture", "--root", root], { stdin: "pipe", stdout: "pipe", stderr: "pipe" });
    const lines = readLines(child.stdout);
    const first = await within(lines.next(), "the fixture ready line");
    if (first.done === true) throw new Error(`the fixture exited before it was ready: ${await new Response(child.stderr).text()}`);
    return new Fixture(child, lines, root, (JSON.parse(first.value) as { ready: Ready }).ready);
  }

  /** Closes the fixture's stdin, which stops it, and returns its serve report. */
  async stop(): Promise<Record<string, unknown>> {
    this.child.stdin.end();
    const next = await within(this.lines.next(), "the fixture serve report");
    const code = await within(this.child.exited, "the fixture exit");
    rmSync(this.root, { recursive: true, force: true });
    if (next.done === true || code !== 0) throw new Error(`fixture stopped with ${code}: ${await new Response(this.child.stderr).text()}`);
    return (JSON.parse(next.value) as { report: Record<string, unknown> }).report;
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

export interface Sent {
  readonly method?: string;
  readonly params?: Record<string, unknown>;
}

/**
 * A byte-transparent Unix-socket relay in front of the fixture that records every NDJSON
 * request each client sends, so a test can count what actually reached the server.
 */
export class Relay {
  readonly sent: Sent[] = [];
  private readonly pairs = new Map<Socket<undefined>, { upstream: Writer; downstream: Writer; lines: Lines; server: Socket<undefined> | undefined }>();

  private constructor(
    private readonly listener: Bun.UnixSocketListener<undefined>,
    private readonly dir: string,
    readonly path: string,
  ) {}

  static start(target: string): Relay {
    const dir = scratch("t63r-");
    const path = join(dir, "r.sock");
    let relay: Relay | undefined;
    const listener = Bun.listen<undefined>({
      unix: path,
      socket: {
        open: (client) => relay?.open(client, target),
        data: (client, chunk) => relay?.upstream(client, chunk),
        drain: (client) => relay?.pairs.get(client)?.downstream.flush(),
        close: (client) => {
          relay?.pairs.get(client)?.server?.end();
        },
      },
    });
    relay = new Relay(listener, dir, path);
    return relay;
  }

  count(method: string, opId: string): number {
    return this.sent.filter((record) => record.method === method && record.params?.op_id === opId).length;
  }

  stop(): void {
    this.listener.stop(true);
    rmSync(this.dir, { recursive: true, force: true });
  }

  private open(client: Socket<undefined>, target: string): void {
    client.pause();
    const pair = { upstream: undefined as unknown as Writer, downstream: new Writer(client), lines: new Lines(), server: undefined as Socket<undefined> | undefined };
    this.pairs.set(client, pair);
    void Bun.connect<undefined>({
      unix: target,
      socket: {
        data: (_, chunk) => pair.downstream.write(chunk),
        drain: () => pair.upstream.flush(),
        close: () => {
          client.end();
        },
      },
    }).then((server) => {
      pair.server = server;
      pair.upstream = new Writer(server);
      client.resume();
    });
  }

  private upstream(client: Socket<undefined>, chunk: Uint8Array): void {
    const pair = this.pairs.get(client);
    if (pair === undefined) return;
    for (const line of pair.lines.push(chunk)) this.sent.push(JSON.parse(line) as Sent);
    pair.upstream.write(chunk);
  }
}

export interface Request {
  readonly id: string;
  readonly method: string;
  readonly params: Record<string, unknown>;
}

/** One client connection to a scripted peer; every record it answers is schema-checked first. */
export class PeerConn {
  private framed = false;
  private upgradePending = false;
  private readonly lines = new Lines();
  private readonly decoder = new FrameDecoder();
  readonly requests = new Queue<Request>();
  readonly frames = new Queue<Frame>();

  constructor(private readonly socket: Socket<undefined>) {}

  receive(chunk: Uint8Array): void {
    if (!this.framed) {
      for (const line of this.lines.push(chunk)) this.requests.push(JSON.parse(line) as Request);
      return;
    }
    for (const frame of this.decoder.push(chunk)) {
      if (frame.channel === CONTROL_CHANNEL && frame.kind === "data") this.requests.push(JSON.parse(new TextDecoder().decode(frame.payload)) as Request);
      else this.frames.push(frame);
    }
  }

  async hello(): Promise<Request> {
    const request = await this.requests.next("hello");
    if (request.method !== "hello") throw new Error(`expected hello, got ${request.method}`);
    this.upgradePending = request.params.framed === true;
    this.reply(request, helloResult());
    return request;
  }

  reply(request: Request, result: unknown): void {
    const violation = checkResult(request.method, result);
    if (violation !== undefined) throw new Error(`scripted ${request.method} breaks the schema: ${violation.path} ${violation.reason}`);
    this.write(this.encode({ jsonrpc: "2.0", id: request.id, result }));
  }

  /** Sends a notification whose params are first checked against the named schema component. */
  notify(method: string, component: string, params: unknown): void {
    const violation = checkComponent(component, params, "output");
    if (violation !== undefined) throw new Error(`scripted ${method} breaks the schema: ${violation.path} ${violation.reason}`);
    this.write(this.encode({ jsonrpc: "2.0", method, params }));
  }

  encode(record: unknown): Uint8Array {
    const bytes = new TextEncoder().encode(JSON.stringify(record));
    if (this.framed) return encodeFrame({ channel: CONTROL_CHANNEL, kind: "data", payload: bytes });
    return Buffer.concat([bytes, Buffer.from("\n")]);
  }

  write(bytes: Uint8Array): void {
    this.socket.write(bytes);
    if (this.upgradePending) {
      this.upgradePending = false;
      this.framed = true;
    }
  }

  vanish(): void {
    this.socket.terminate();
  }
}

export class Peer {
  private readonly accepted = new Queue<PeerConn>();
  private readonly conns = new Map<Socket<undefined>, PeerConn>();

  private constructor(
    private readonly listener: Bun.UnixSocketListener<undefined>,
    private readonly dir: string,
    readonly path: string,
  ) {}

  static start(): Peer {
    const dir = scratch("t63p-");
    const path = join(dir, "p.sock");
    let peer: Peer | undefined;
    const listener = Bun.listen<undefined>({
      unix: path,
      socket: {
        open: (socket) => peer?.open(socket),
        data: (socket, chunk) => peer?.conns.get(socket)?.receive(chunk),
      },
    });
    peer = new Peer(listener, dir, path);
    return peer;
  }

  accept(): Promise<PeerConn> {
    return this.accepted.next("a client connection");
  }

  stop(): void {
    this.listener.stop(true);
    rmSync(this.dir, { recursive: true, force: true });
  }

  private open(socket: Socket<undefined>): void {
    const conn = new PeerConn(socket);
    this.conns.set(socket, conn);
    this.accepted.push(conn);
  }
}

export const LIFETIME = "lifetime:0191f1a2-0000-7000-8000-000000000063";

export function helloResult(): Record<string, unknown> {
  return {
    machine_id: "machine:peer",
    runtime_lifetime: LIFETIME,
    server_boot_id: "boot:1",
    generation: "1",
    version: "0.1.0",
    methods: ["attach.open", "attach.credit", "events.subscribe", "op.get"],
    capabilities: { transports: ["unix"], stream_modes: ["channel", "plain"], framed: true },
    limits: limitDefaults,
    server_time_ms: "1",
    endpoint_kind: "unix",
  };
}

export function attachResult(outcome: string, stream: string | null, channel: number | null): Record<string, unknown> {
  const opened = stream === null ? {} : {
    stream_id: stream,
    channel,
    lease: { lease_id: `lease:${stream}`, epoch: "1", role: "observe", holder: "client:t63" },
    first_paint: {
      output_offset: "0",
      stream_offset: "0",
      revision: "1",
      geometry: { cols: 80, rows: 24 },
      modes: { enabled: [], kitty_keyboard: 0, revision: "1" },
      cursor: { row: 0, col: 0, visible: true },
      history_lines: "0",
    },
  };
  const refused = stream === null ? { reason: "held by another client" } : {};
  return { outcome, credit_initial: limitDefaults.credit_initial_bytes, ...opened, ...refused };
}

export function subscribed(id: string): Record<string, unknown> {
  return {
    subscription_id: id,
    snapshot_seq: "4",
    replay_floor: "1",
    queue_events: 10_000,
    queue_bytes: limitDefaults.event_replay_bytes,
    heartbeat_ms: limitDefaults.heartbeat_ms,
    missed: null,
    snapshot: null,
  };
}

export function envelope(subscription: string, seq: string, type: string): Record<string, unknown> {
  return {
    seq,
    lifetime: LIFETIME,
    boot_id: "boot:1",
    machine_id: "machine:peer",
    subscription_id: subscription,
    ts_ms: "1",
    type,
    target: {},
    data: { seq },
  };
}
