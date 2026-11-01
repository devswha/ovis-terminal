import { ClientError } from "../errors";
import { streamContract } from "../generated/stream";

export const HEADER_BYTES = streamContract.header_bytes;
export const MAX_PAYLOAD_BYTES = streamContract.max_payload_bytes;
export const CONTROL_CHANNEL = streamContract.control_channel;

export type FrameKind = "data" | "credit" | "close";

const KIND_VALUES: ReadonlyMap<number, FrameKind> = new Map(
  streamContract.kinds.map((kind) => [kind.value, kind.name.toLowerCase() as FrameKind]),
);
const KIND_BYTES: ReadonlyMap<FrameKind, number> = new Map([...KIND_VALUES].map(([value, name]) => [name, value]));

export interface Frame {
  readonly channel: number;
  readonly kind: FrameKind;
  readonly payload: Uint8Array;
}

export function encodeFrame(frame: Frame): Uint8Array {
  if (frame.payload.byteLength > MAX_PAYLOAD_BYTES) {
    throw new ClientError("frame_too_large", `a ${frame.payload.byteLength}-byte payload exceeds the frame bound`);
  }
  const bytes = new Uint8Array(HEADER_BYTES + frame.payload.byteLength);
  const view = new DataView(bytes.buffer);
  view.setUint32(0, frame.channel, true);
  view.setUint8(4, KIND_BYTES.get(frame.kind) ?? 0);
  view.setUint32(5, frame.payload.byteLength, true);
  bytes.set(frame.payload, HEADER_BYTES);
  return bytes;
}

/** A CREDIT payload: the cumulative acknowledged stream offset as eight little-endian bytes. */
export function creditPayload(offset: bigint): Uint8Array {
  const payload = new Uint8Array(8);
  new DataView(payload.buffer).setBigUint64(0, offset, true);
  return payload;
}

/** Incremental decoder: identical frames for every fragmentation of the same bytes. */
export class FrameDecoder {
  private buffered = new Uint8Array(0);

  push(input: Uint8Array): Frame[] {
    const joined = new Uint8Array(this.buffered.byteLength + input.byteLength);
    joined.set(this.buffered, 0);
    joined.set(input, this.buffered.byteLength);
    const frames: Frame[] = [];
    let start = 0;
    while (joined.byteLength - start >= HEADER_BYTES) {
      const view = new DataView(joined.buffer, joined.byteOffset + start, HEADER_BYTES);
      const rawKind = view.getUint8(4);
      const kind = KIND_VALUES.get(rawKind);
      if (kind === undefined) {
        throw new ClientError("unknown_kind", `frame kind ${rawKind} is not defined`);
      }
      const length = view.getUint32(5, true);
      if (length > MAX_PAYLOAD_BYTES) {
        throw new ClientError("frame_too_large", `a frame header declares ${length} payload bytes`);
      }
      const end = start + HEADER_BYTES + length;
      if (joined.byteLength < end) {
        break;
      }
      frames.push({ channel: view.getUint32(0, true), kind, payload: joined.slice(start + HEADER_BYTES, end) });
      start = end;
    }
    this.buffered = joined.slice(start);
    return frames;
  }

  /** Throws `truncated_frame` when the stream ended inside a header or payload. */
  finish(): void {
    if (this.buffered.byteLength > 0) {
      throw new ClientError("truncated_frame", `${this.buffered.byteLength} bytes of an incomplete frame remain`);
    }
  }
}
