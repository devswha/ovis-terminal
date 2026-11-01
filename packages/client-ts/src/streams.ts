import type { Client } from "./client";
import { decimal } from "./decimal";
import { ClientError } from "./errors";
import type {
  AttachControl,
  AttachData,
  AttachOpenParams,
  AttachOpenResult,
  LeaseRecord,
  StreamId,
  Target,
} from "./generated/components";
import { limitDefaults } from "./generated/stream";
import { creditPayload, type Frame } from "./wire/frames";

const text = new TextDecoder();

export interface Chunk {
  readonly offset: bigint;
  readonly bytes: Uint8Array;
}

export interface StreamIdentity {
  readonly id: StreamId;
  readonly channel: number | undefined;
  readonly target: Target;
  readonly lease: LeaseRecord | undefined;
  readonly start: bigint;
}

/** `attach.open`'s typed outcome; `stream` is present only when the server opened one. */
export interface Attached {
  readonly result: AttachOpenResult;
  readonly stream: Stream | undefined;
}

/**
 * One attach stream. Its identity (stream id and framed channel) is never reused, so an
 * acknowledgement on a closed stream is refused locally and cannot credit any other stream.
 */
export class Stream {
  private deliveredEnd: bigint;
  private acknowledgedNow: bigint;
  private closedReason: string | undefined;
  private ready = false;
  readonly chunks: Chunk[] = [];
  readonly controls: AttachControl[] = [];

  readonly id: StreamId;
  readonly channel: number | undefined;
  readonly target: Target;
  readonly lease: LeaseRecord | undefined;

  constructor(
    private readonly client: Client,
    identity: StreamIdentity,
  ) {
    ({ id: this.id, channel: this.channel, target: this.target, lease: this.lease } = identity);
    this.deliveredEnd = identity.start;
    this.acknowledgedNow = identity.start;
  }

  get delivered(): bigint {
    return this.deliveredEnd;
  }

  get acknowledged(): bigint {
    return this.acknowledgedNow;
  }

  get closed(): string | undefined {
    return this.closedReason;
  }

  /** Input is ready only between `input_ready` and revocation, close or disconnect. */
  get inputReady(): boolean {
    return this.ready && this.closedReason === undefined;
  }

  /** Grants credit up to `offset`, a cumulative filtered-stream offset this stream delivered. */
  async ack(offset: bigint): Promise<void> {
    if (this.closedReason !== undefined) {
      throw new ClientError("stream_closed", `stream ${this.id} closed (${this.closedReason}); its acknowledgement grants nothing`);
    }
    if (offset > this.deliveredEnd || offset < this.acknowledgedNow) {
      throw new ClientError("impossible_ack", `offset ${offset} is outside ${this.acknowledgedNow}..${this.deliveredEnd}`);
    }
    this.acknowledgedNow = offset;
    if (this.channel !== undefined && this.client.framed) {
      this.client.sendFrame({ channel: this.channel, kind: "credit", payload: creditPayload(offset) });
      return;
    }
    await this.client.call("attach.credit", { stream_id: this.id, acknowledged: decimal(offset) });
  }

  bytes(): Uint8Array {
    return Buffer.concat(this.chunks.map((chunk) => chunk.bytes));
  }

  data(offset: bigint, bytes: Uint8Array): void {
    if (this.closedReason !== undefined) return;
    this.chunks.push({ offset, bytes });
    this.deliveredEnd = offset + BigInt(bytes.byteLength);
  }

  control(record: AttachControl): void {
    this.controls.push(record);
    switch (record.kind) {
      case "input_ready":
        this.ready = true;
        break;
      case "input_revoked":
        this.ready = false;
        break;
      case "close":
        this.end(record.close?.reason ?? "close");
        break;
      default:
        break;
    }
  }

  end(reason: string): void {
    this.ready = false;
    this.closedReason ??= reason;
  }
}

/** Routes stream bytes and control records by stream identity on one client. */
export class Streams {
  private readonly byId = new Map<string, Stream>();
  private readonly byChannel = new Map<number, Stream>();
  private readonly early = new Map<string, { readonly bytes: number; readonly apply: (stream: Stream) => void }[]>();
  private earlyBytes = 0;

  constructor(private readonly client: Client) {
    client.on("attach.data", (params) => {
      const record = params as AttachData;
      const bytes = Buffer.from(record.data_b64, "base64");
      this.route(`s:${record.stream_id}`, bytes.byteLength, (stream) => stream.data(BigInt(record.offset), bytes));
    });
    client.on("attach.control", (params) => {
      const record = params as AttachControl;
      this.route(`s:${record.stream_id}`, 0, (stream) => stream.control(record));
    });
    client.onFrame((frame) => this.route(`c:${frame.channel}`, frame.payload.byteLength, (stream) => this.frame(stream, frame)));
    client.onDisconnect(() => {
      for (const stream of this.byId.values()) stream.end("disconnected");
    });
  }

  async open(params: AttachOpenParams): Promise<Attached> {
    const result = await this.client.call("attach.open", params);
    if (result.stream_id === undefined || result.stream_id === null) return { result, stream: undefined };
    const start = BigInt(result.first_paint?.stream_offset ?? "0");
    const channel = result.channel ?? undefined;
    const stream = new Stream(this.client, { id: result.stream_id, channel, target: params.target, lease: result.lease ?? undefined, start });
    this.byId.set(stream.id, stream);
    const keys = [`s:${stream.id}`];
    if (channel !== undefined) {
      this.byChannel.set(channel, stream);
      keys.push(`c:${channel}`);
    }
    for (const key of keys) {
      for (const waiting of this.early.get(key) ?? []) {
        this.earlyBytes -= waiting.bytes;
        waiting.apply(stream);
      }
      this.early.delete(key);
    }
    return { result, stream };
  }

  /** Applies a record to its stream; records that beat the `attach.open` reply wait for it, within the credit bound. */
  private route(key: string, bytes: number, apply: (stream: Stream) => void): void {
    const known = key.startsWith("c:") ? this.byChannel.get(Number(key.slice(2))) : this.byId.get(key.slice(2));
    if (known !== undefined) {
      apply(known);
      return;
    }
    if (this.earlyBytes + bytes > Number(limitDefaults.credit_max_bytes)) {
      this.client.violations.push({ method: key, path: key, reason: "stream bytes for an unknown stream exceed the credit bound" });
      return;
    }
    this.earlyBytes += bytes;
    this.early.set(key, [...(this.early.get(key) ?? []), { bytes, apply }]);
  }

  private frame(stream: Stream, frame: Frame): void {
    if (frame.kind === "data") {
      stream.data(stream.delivered, frame.payload);
    } else if (frame.kind === "close") {
      const reason = (JSON.parse(text.decode(frame.payload)) as { reason?: unknown }).reason;
      stream.end(typeof reason === "string" ? reason : "close");
    }
  }
}
