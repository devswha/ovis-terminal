// A scripted protocol peer for routes the fixture does not serve yet (attach, events,
// input receipts). Every record it sends is first checked against the published schema,
// so a script cannot hand the client a shape the runtime would never produce.

import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import type { Socket } from "bun";

import { checkNotification, checkResult } from "../../src/schema";
import { CONTROL_CHANNEL, type Frame, FrameDecoder, encodeFrame } from "../../src/wire/frames";
import { LineDecoder } from "../../src/wire/lines";
import { within } from "./bounded";
import { helloResult } from "./records";

export interface Request {
  readonly id: string;
  readonly method: string;
  readonly params: Record<string, unknown>;
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

  get pending(): number {
    return this.items.length;
  }
}

export class PeerConn {
  framed = false;
  private upgradePending = false;
  private readonly lines = new LineDecoder();
  private readonly decoder = new FrameDecoder();
  readonly requests = new Queue<Request>();
  readonly frames = new Queue<Frame>();
  readonly gone: Promise<void>;
  private markGone: () => void = () => undefined;

  constructor(private readonly socket: Socket<undefined>) {
    this.gone = new Promise((resolve) => {
      this.markGone = resolve;
    });
  }

  receive(chunk: Uint8Array): void {
    if (!this.framed) {
      for (const line of this.lines.push(chunk)) this.requests.push(JSON.parse(line) as Request);
      return;
    }
    for (const frame of this.decoder.push(chunk)) {
      if (frame.channel === CONTROL_CHANNEL && frame.kind === "data") {
        this.requests.push(JSON.parse(new TextDecoder().decode(frame.payload)) as Request);
      } else {
        this.frames.push(frame);
      }
    }
  }

  closed(): void {
    this.markGone();
  }

  /** Answers the next request, which must be hello, and upgrades when it asked to. */
  async hello(overrides: Record<string, unknown> = {}): Promise<Request> {
    const request = await this.requests.next("hello");
    if (request.method !== "hello") throw new Error(`expected hello, got ${request.method}`);
    this.upgradePending = request.params.framed === true;
    this.reply(request, { ...helloResult(), ...overrides });
    return request;
  }

  reply(request: Request, result: unknown): void {
    const violation = checkResult(request.method, result);
    if (violation !== undefined) throw new Error(`scripted ${request.method} result breaks the schema: ${violation.path} ${violation.reason}`);
    this.send({ jsonrpc: "2.0", id: request.id, result });
  }

  /** Sends a record as-is; used to inject a record outside the schema on purpose. */
  send(record: unknown): void {
    const bytes = new TextEncoder().encode(JSON.stringify(record));
    if (this.framed) {
      this.socket.write(encodeFrame({ channel: CONTROL_CHANNEL, kind: "data", payload: bytes }));
    } else {
      this.socket.write(Buffer.concat([bytes, Buffer.from("\n")]));
    }
    if (this.upgradePending) {
      this.upgradePending = false;
      this.framed = true;
    }
  }

  notify(method: string, params: unknown): void {
    const violation = checkNotification(method, params);
    if (violation !== undefined) throw new Error(`scripted ${method} breaks the schema: ${violation.path} ${violation.reason}`);
    this.send({ jsonrpc: "2.0", method, params });
  }

  frame(frame: Frame): void {
    this.socket.write(encodeFrame(frame));
  }

  /** Drops the connection with no reply and no close handshake. */
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
    const dir = mkdtempSync(join(tmpdir(), "t29p-"));
    const path = join(dir, "s.sock");
    let peer: Peer | undefined;
    const listener = Bun.listen<undefined>({
      unix: path,
      socket: {
        open: (socket) => peer?.open(socket),
        data: (socket, chunk) => peer?.conns.get(socket)?.receive(chunk),
        close: (socket) => peer?.conns.get(socket)?.closed(),
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
