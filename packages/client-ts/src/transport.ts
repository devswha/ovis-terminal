import type { Socket } from "bun";

import { ClientError } from "./errors";
import { CONTROL_CHANNEL, type Frame, FrameDecoder, encodeFrame } from "./wire/frames";
import { LineDecoder } from "./wire/lines";

const NEWLINE = 0x0a;
const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { fatal: true });

export interface Sink {
  record(record: unknown): void;
  frame(frame: Frame): void;
  closed(error: ClientError | undefined): void;
}

/** One persistent Unix-socket connection: NDJSON, upgraded in place to framed mode. */
export class Transport {
  private mode: "lines" | "framed" = "lines";
  private upgradeAfter: string | undefined;
  private readonly lines = new LineDecoder();
  private readonly frames = new FrameDecoder();
  private queued: Uint8Array[] = [];
  private failure: ClientError | undefined;
  private open = true;

  private constructor(
    private readonly socket: Socket<undefined>,
    private readonly sink: Sink,
  ) {}

  static async connect(path: string, sink: Sink): Promise<Transport> {
    let transport: Transport | undefined;
    const socket = await Bun.connect({
      unix: path,
      socket: {
        data: (_, chunk) => transport?.receive(chunk),
        drain: () => transport?.flush(),
        close: () => transport?.finish(),
        error: (_, error) => transport?.fail(new ClientError("disconnected", error.message, { cause: error })),
      },
    });
    transport = new Transport(socket, sink);
    return transport;
  }

  get framed(): boolean {
    return this.mode === "framed";
  }

  /** The NDJSON reply carrying `id` is the last line; framed bytes follow it. */
  upgradeAfterReply(id: string): void {
    this.upgradeAfter = id;
  }

  sendRecord(record: unknown): void {
    const bytes = encoder.encode(JSON.stringify(record));
    if (this.mode === "framed") {
      this.write(encodeFrame({ channel: CONTROL_CHANNEL, kind: "data", payload: bytes }));
    } else {
      const line = new Uint8Array(bytes.byteLength + 1);
      line.set(bytes);
      line[bytes.byteLength] = NEWLINE;
      this.write(line);
    }
  }

  sendFrame(frame: Frame): void {
    this.write(encodeFrame(frame));
  }

  /** Closes immediately without flushing; models a transport that vanished. */
  sever(): void {
    this.socket.terminate();
  }

  close(): void {
    this.socket.end();
  }

  private write(bytes: Uint8Array): void {
    if (!this.open) {
      throw new ClientError("not_connected", "the connection is closed");
    }
    this.queued.push(bytes);
    this.flush();
  }

  private flush(): void {
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

  private receive(chunk: Uint8Array): void {
    try {
      let rest = chunk;
      while (rest.byteLength > 0 && this.mode === "lines") {
        const end = rest.indexOf(NEWLINE);
        if (end === -1) {
          this.lines.push(rest);
          return;
        }
        for (const line of this.lines.push(rest.subarray(0, end + 1))) {
          this.line(line);
        }
        rest = rest.subarray(end + 1);
      }
      for (const frame of rest.byteLength > 0 ? this.frames.push(rest) : []) {
        this.dispatchFrame(frame);
      }
    } catch (error) {
      this.fail(error instanceof ClientError ? error : new ClientError("invalid_response", String(error), { cause: error }));
    }
  }

  private line(line: string): void {
    const record = parse(line);
    this.sink.record(record);
    const id = (record as { id?: unknown } | null)?.id;
    if (this.upgradeAfter !== undefined && id === this.upgradeAfter) {
      this.upgradeAfter = undefined;
      this.mode = "framed";
    }
  }

  private dispatchFrame(frame: Frame): void {
    if (frame.channel === CONTROL_CHANNEL && frame.kind === "data") {
      this.sink.record(parse(decoder.decode(frame.payload)));
    } else {
      this.sink.frame(frame);
    }
  }

  private fail(error: ClientError): void {
    this.failure ??= error;
    this.socket.terminate();
  }

  private finish(): void {
    if (!this.open) return;
    this.open = false;
    if (this.failure === undefined && this.mode === "framed") {
      try {
        this.frames.finish();
      } catch (error) {
        this.failure = error as ClientError;
      }
    }
    this.sink.closed(this.failure);
  }
}

function parse(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch (error) {
    throw new ClientError("invalid_response", "the peer sent a record that is not JSON", { cause: error });
  }
}
