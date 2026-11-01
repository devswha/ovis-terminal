import { Draft, OutcomeUnknown } from "./drafts";
import { ClientError, RpcError } from "./errors";
import type { ErrorData, HelloResult, OpCancelResult, OpId, OpRecord } from "./generated/components";
import { methods, type ParamsOf, type ResultOf, type RpcMethod } from "./generated/methods";
import { protocolGeneration } from "./generated/stream";
import { checkNotification, checkParams, checkResult } from "./schema";
import { Transport } from "./transport";
import type { Frame } from "./wire/frames";

export interface ConnectOptions {
  readonly socketPath: string;
  readonly credential?: string;
  readonly framed?: boolean;
  readonly client?: { readonly name: string; readonly version: string };
}

export interface CallOptions {
  readonly signal?: AbortSignal;
  readonly deadlineMs?: number;
}

interface Pending {
  readonly method: string;
  readonly checked: boolean;
  readonly params: Readonly<Record<string, unknown>>;
  readonly resolve: (result: unknown) => void;
  readonly reject: (error: Error) => void;
  readonly cleanup: () => void;
}

type Listener = (params: unknown) => void;

export interface Connection {
  readonly epoch: number;
  readonly hello: HelloResult;
}

/** One persistent multiplexed connection with correlation, deadlines and cancellation. */
export class Client {
  private transport: Transport | undefined;
  private readonly pending = new Map<string, Pending>();
  private readonly listeners = new Map<string, Set<Listener>>();
  private readonly frameListeners = new Set<(frame: Frame) => void>();
  private readonly disconnectListeners = new Set<() => void>();
  private readonly retained: Draft[] = [];
  private nextId = 1;
  private current: Connection | undefined;
  readonly violations: { readonly method: string; readonly path: string; readonly reason: string }[] = [];
  lateReplies = 0;

  private constructor(private readonly options: ConnectOptions) {}

  static async connect(options: ConnectOptions): Promise<Client> {
    const client = new Client(options);
    await client.reconnect();
    return client;
  }

  get connection(): Connection {
    if (this.current === undefined) throw new ClientError("not_connected", "no hello has completed");
    return this.current;
  }

  get framed(): boolean {
    return this.transport?.framed ?? false;
  }

  /** Mutations whose replies were lost and that still wait for an explicit decision. */
  drafts(): readonly Draft[] {
    return this.retained.filter((draft) => draft.decision === "pending");
  }

  /** Opens a fresh connection and repeats hello; never resends a request of the old one. */
  async reconnect(): Promise<Connection> {
    this.transport = await Transport.connect(this.options.socketPath, {
      record: (record) => this.record(record),
      frame: (frame) => this.frameListeners.forEach((listener) => listener(frame)),
      closed: (error) => this.closed(error),
    });
    const params = {
      client: this.options.client ?? { name: "@ovis-terminal/client", version: "0.1.0" },
      generation: protocolGeneration,
      credential: this.options.credential ?? null,
      framed: this.options.framed ?? false,
    };
    const hello = await this.call("hello", params as ParamsOf<"hello">);
    this.current = { epoch: (this.current?.epoch ?? 0) + 1, hello };
    return this.current;
  }

  call<M extends RpcMethod>(method: M, params: ParamsOf<M>, options: CallOptions = {}): Promise<ResultOf<M>> {
    const violation = checkParams(method, params);
    if (violation !== undefined) {
      const kind = violation.path === "method" ? "method_not_found" : "invalid_params";
      return Promise.reject(new ClientError(kind, `${violation.path} ${violation.reason}`, { fieldPath: violation.path }));
    }
    return this.send(method, params as Record<string, unknown>, { ...options, checked: true }) as Promise<ResultOf<M>>;
  }

  /**
   * Sends and returns the raw reply with no schema check in either direction, so a
   * conformance test can observe what a server does with input the typed path refuses.
   */
  unchecked(method: string, params: unknown): Promise<unknown> {
    return this.send(method, params as Record<string, unknown>, { checked: false });
  }

  on(method: string, listener: Listener): () => void {
    const set = this.listeners.get(method) ?? new Set();
    set.add(listener);
    this.listeners.set(method, set);
    return () => set.delete(listener);
  }

  onFrame(listener: (frame: Frame) => void): () => void {
    this.frameListeners.add(listener);
    return () => this.frameListeners.delete(listener);
  }

  onDisconnect(listener: () => void): () => void {
    this.disconnectListeners.add(listener);
    return () => this.disconnectListeners.delete(listener);
  }

  sendFrame(frame: Frame): void {
    this.live().sendFrame(frame);
  }

  /** Drops the transport without a close handshake, as a vanished network would. */
  sever(): void {
    this.transport?.sever();
  }

  close(): void {
    this.transport?.close();
  }

  private live(): Transport {
    if (this.transport === undefined) throw new ClientError("not_connected", "the connection is closed");
    return this.transport;
  }

  private send(method: string, params: Record<string, unknown>, options: CallOptions & { readonly checked: boolean }): Promise<unknown> {
    const transport = this.live();
    const id = `c${this.nextId++}`;
    return new Promise((resolve, reject) => {
      const timer = options.deadlineMs === undefined ? undefined : setTimeout(() => this.expire(id), options.deadlineMs);
      const abort = () => this.abort(id);
      options.signal?.addEventListener("abort", abort, { once: true });
      const cleanup = () => {
        clearTimeout(timer);
        options.signal?.removeEventListener("abort", abort);
      };
      this.pending.set(id, { method, checked: options.checked, params, resolve, reject, cleanup });
      if (method === "hello" && params.framed === true) transport.upgradeAfterReply(id);
      transport.sendRecord({ jsonrpc: "2.0", id, method, params });
    });
  }

  private settle(id: string): Pending | undefined {
    const entry = this.pending.get(id);
    this.pending.delete(id);
    entry?.cleanup();
    return entry;
  }

  private draftOf(entry: Pending): Draft | undefined {
    const opId = entry.params.op_id;
    if (methods[entry.method as RpcMethod]?.effect !== "mutation" || typeof opId !== "string") return undefined;
    const draft = new Draft(entry.method, entry.params, opId as OpId, {
      resend: (method, params) => this.send(method, params as Record<string, unknown>, { checked: entry.checked }),
      inspect: (op) => this.call("op.get", { op_id: op }) as Promise<OpRecord>,
      cancel: (op) => this.call("op.cancel", { op_id: op }) as Promise<OpCancelResult>,
    });
    this.retained.push(draft);
    return draft;
  }

  private expire(id: string): void {
    const entry = this.settle(id);
    if (entry === undefined) return;
    const draft = this.draftOf(entry);
    entry.reject(draft === undefined ? new ClientError("deadline_exceeded", `${entry.method} got no reply in time`) : new OutcomeUnknown("deadline_exceeded", draft));
  }

  private abort(id: string): void {
    const entry = this.pending.get(id);
    if (entry === undefined) return;
    const opId = entry.params.op_id;
    if (methods[entry.method as RpcMethod]?.effect === "mutation" && typeof opId === "string") {
      this.unchecked("op.cancel", { op_id: opId }).catch(() => undefined);
      return;
    }
    this.settle(id)?.reject(new ClientError("cancelled", `${entry.method} was abandoned before its reply`));
  }

  private record(record: unknown): void {
    const message = record as { id?: unknown; method?: unknown; params?: unknown; result?: unknown; error?: unknown };
    if (typeof message.method === "string") {
      this.notification(message.method, message.params);
      return;
    }
    const entry = typeof message.id === "string" ? this.settle(message.id) : undefined;
    if (entry === undefined) {
      this.lateReplies += 1;
      return;
    }
    if (message.error !== undefined) {
      const error = message.error as { code: number; message: string; data?: ErrorData };
      const data = error.data;
      entry.reject(data === undefined || typeof data.kind !== "string" ? new ClientError("invalid_response", "an error reply lacks data.kind") : new RpcError(error.code, error.message, data));
      return;
    }
    const violation = entry.checked ? checkResult(entry.method, message.result) : undefined;
    if (violation !== undefined) {
      entry.reject(new ClientError("invalid_response", `${violation.path} ${violation.reason}`, { fieldPath: violation.path }));
      return;
    }
    entry.resolve(message.result);
  }

  private notification(method: string, params: unknown): void {
    const violation = checkNotification(method, params);
    if (violation !== undefined) {
      this.violations.push({ method, ...violation });
      return;
    }
    this.listeners.get(method)?.forEach((listener) => listener(params));
  }

  private closed(error: ClientError | undefined): void {
    this.transport = undefined;
    for (const id of [...this.pending.keys()]) {
      const entry = this.settle(id);
      if (entry === undefined) continue;
      const draft = this.draftOf(entry);
      entry.reject(draft === undefined ? new ClientError("disconnected", `${entry.method} lost its connection`, { cause: error }) : new OutcomeUnknown("outcome_unknown", draft));
    }
    this.disconnectListeners.forEach((listener) => listener());
  }
}
