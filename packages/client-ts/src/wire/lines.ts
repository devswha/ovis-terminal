import { ClientError } from "../errors";
import { MAX_PAYLOAD_BYTES } from "./frames";

const NEWLINE = 0x0a;

/** Splits NDJSON bytes into complete UTF-8 lines regardless of read boundaries. */
export class LineDecoder {
  private pending: Uint8Array[] = [];
  private pendingBytes = 0;
  private readonly text = new TextDecoder("utf-8", { fatal: true });

  push(input: Uint8Array): string[] {
    const lines: string[] = [];
    let start = 0;
    for (let index = input.indexOf(NEWLINE); index !== -1; index = input.indexOf(NEWLINE, start)) {
      this.keep(input.subarray(start, index));
      lines.push(this.take());
      start = index + 1;
    }
    this.keep(input.subarray(start));
    return lines;
  }

  private drain(): Uint8Array {
    const rest = this.concat();
    this.pending = [];
    this.pendingBytes = 0;
    return rest;
  }

  private keep(part: Uint8Array): void {
    if (part.byteLength === 0) return;
    this.pendingBytes += part.byteLength;
    if (this.pendingBytes > MAX_PAYLOAD_BYTES) {
      throw new ClientError("line_too_large", `an NDJSON line exceeds ${MAX_PAYLOAD_BYTES} bytes`);
    }
    this.pending.push(part.slice());
  }

  private take(): string {
    return this.text.decode(this.drain());
  }

  private concat(): Uint8Array {
    const out = new Uint8Array(this.pendingBytes);
    let at = 0;
    for (const part of this.pending) {
      out.set(part, at);
      at += part.byteLength;
    }
    return out;
  }
}
