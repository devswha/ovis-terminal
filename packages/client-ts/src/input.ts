import type { Client } from "./client";
import { ClientError } from "./errors";
import type { Expect, InputReceipt, OpId, OpRecord, StatusSnapshot, Submit } from "./generated/components";
import type { Stream } from "./streams";
import type { Subscription } from "./subscriptions";

export interface TextInput {
  readonly text: string;
  readonly submit: Submit;
  readonly expect?: Expect | null;
}

/**
 * What `input.send` returns: admission into the terminal's queue only. PTY delivery, agent
 * acceptance and turn facts are separate updates, read from the operation record or its events.
 */
export class Admission {
  constructor(
    private readonly client: Client,
    readonly receipt: InputReceipt,
    readonly statusAtSend: StatusSnapshot,
  ) {}

  get opId(): OpId {
    return this.receipt.op_id;
  }

  /** The operation record as committed now; its evidence carries write delivery facts. */
  inspect(): Promise<OpRecord> {
    return this.client.call("op.get", { op_id: this.opId });
  }

  /** The next committed op.* event about this admission on `subscription`. */
  async update(subscription: Subscription): Promise<OpRecord> {
    for (;;) {
      const event = await subscription.next();
      const record = event.data as Partial<OpRecord> | null;
      if (event.type.startsWith("op.") && record?.op_id === this.opId) return record as OpRecord;
    }
  }
}

/** A fresh client-minted operation identity (UUIDv7); reuse it only to retry the same request. */
export function newOpId(): OpId {
  return Bun.randomUUIDv7() as OpId;
}

/** Sends literal text on an interact stream that is input-ready; refuses locally otherwise. */
export async function sendText(client: Client, stream: Stream, input: TextInput): Promise<Admission> {
  if (!stream.inputReady) {
    throw new ClientError("input_not_ready", `stream ${stream.id} is not input-ready${stream.closed === undefined ? "" : ` (${stream.closed})`}`);
  }
  const result = await client.call("input.send", {
    op_id: newOpId(),
    target: stream.target,
    lease_id: stream.lease?.lease_id ?? null,
    text: input.text,
    submit: input.submit,
    expect: input.expect ?? null,
  });
  return new Admission(client, result.receipt, result.status_at_send);
}
