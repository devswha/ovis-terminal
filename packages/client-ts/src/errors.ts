import type { ErrorData } from "./generated/components";

/** Failures the client itself detects; each has a stable kind and recovery hint. */
export const clientErrorHints = {
  invalid_decimal: "send u64 values as canonical decimal strings",
  invalid_params: "correct the request parameters; nothing was sent",
  method_not_found: "call a method the generated catalog publishes",
  invalid_response: "the peer sent a record outside the published schema; reconnect",
  frame_too_large: "the peer exceeded the 16 MiB frame bound; reconnect",
  unknown_kind: "the peer sent a frame kind no generation defines; reconnect",
  truncated_frame: "the stream ended inside a frame; reconnect",
  line_too_large: "the peer exceeded the frame bound in one NDJSON line; reconnect",
  not_connected: "connect or reconnect before calling",
  disconnected: "reconnect; requests without an op_id may be retried",
  outcome_unknown: "inspect the retained draft and decide explicitly to retry or discard it",
  deadline_exceeded: "inspect the operation by op_id before deciding to retry",
  cancelled: "the operation was cancelled before it committed",
  stream_closed: "this stream identity is closed; acknowledge on the stream that replaced it",
  impossible_ack: "acknowledge only offsets this stream has delivered",
  input_not_ready: "wait for input_ready on a live interact stream",
  draft_settled: "this draft already has an explicit decision",
  rpc_error: "the server refused the request; see its kind and hint",
  unsupported_transport: "this build has no client transport for the platform's endpoint; use the CLI connect bridge",
} as const;

export type ClientErrorKind = keyof typeof clientErrorHints;

export class ClientError extends Error {
  readonly kind: ClientErrorKind;
  readonly hint: string;
  readonly fieldPath: string | undefined;

  constructor(kind: ClientErrorKind, message: string, options?: { cause?: unknown; fieldPath?: string | undefined }) {
    super(message, options?.cause === undefined ? undefined : { cause: options.cause });
    this.name = "ClientError";
    this.kind = kind;
    this.hint = clientErrorHints[kind];
    this.fieldPath = options?.fieldPath;
  }
}

/** A JSON-RPC error reply; `data.kind` is kept even when this build does not know it. */
export class RpcError extends ClientError {
  readonly code: number;
  readonly data: ErrorData;

  constructor(code: number, message: string, data: ErrorData) {
    super("rpc_error", message, { fieldPath: data.field_path });
    this.name = "RpcError";
    this.code = code;
    this.data = data;
  }

  get serverKind(): string {
    return this.data.kind;
  }
}
