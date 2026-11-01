// Typed refusals on routes the fixture does not serve yet, against a schema-checked peer.

import { expect } from "bun:test";

import { OutcomeUnknown } from "../../src/drafts";
import { ClientError } from "../../src/errors";
import { sendText } from "../../src/input";
import { Subscription } from "../../src/subscriptions";
import { once, rejection, within } from "../support/bounded";
import { ABOVE_2_53, envelope, inputResult, opRecord, subscribed } from "../support/records";
import { above, attach, control, reconnect, session } from "./scripted";

const kindOf = (error: unknown) => (error as ClientError).kind;

/** A reply lost after input admission: outcome_unknown, input-ready revoked, never resent. */
export async function lostInputReply(): Promise<void> {
  const s = await session(false);
  try {
    const stream = await attach(s, "stream:i", null);
    await control(s, "stream:i", ["input_ready"]);
    const sending = sendText(s.client, stream, { text: "rm -rf build", submit: "enter" });
    const admitted = await s.conn.requests.next("input.send");
    s.conn.vanish();
    const error = await within(rejection(sending), "the lost input reply");
    expect(error).toBeInstanceOf(OutcomeUnknown);
    const draft = (error as OutcomeUnknown).draft;
    expect<unknown>([kindOf(error), draft.opId, draft.params]).toEqual(["outcome_unknown", admitted.params.op_id, admitted.params]);
    expect<unknown>([stream.inputReady, stream.closed]).toEqual([false, "disconnected"]);
    expect(kindOf(await rejection(sendText(s.client, stream, { text: "again", submit: "enter" })))).toBe("input_not_ready");
    const conn = await reconnect(s);
    expect(stream.inputReady).toBe(false);
    expect(s.client.drafts()).toEqual([draft]);
    const inspecting = draft.inspect();
    const first = await conn.requests.next("the first request after reconnect");
    expect<unknown>([first.method, first.params]).toEqual(["op.get", { op_id: admitted.params.op_id }]);
    conn.reply(first, opRecord(draft.opId, "outcome_unknown", "5"));
    expect((await inspecting).state).toBe("outcome_unknown");
    const retrying = draft.retry();
    const again = await conn.requests.next("the explicit retry");
    expect<unknown>([again.method, again.params]).toEqual(["input.send", admitted.params]);
    conn.reply(again, inputResult(draft.opId));
    await within(retrying, "the retried admission");
    expect<unknown>([draft.decision, s.client.drafts()]).toEqual(["retried", []]);
    expect(() => draft.discard()).toThrow(ClientError);
    expect(conn.requests.pending).toBe(0);
  } finally {
    s.client.close();
    s.peer.stop();
  }
}

/** An acknowledgement on a closed stream grants nothing; the new stream's credit is its own. */
export async function staleStreamAck(): Promise<void> {
  const s = await session(true);
  try {
    const old = await attach(s, "stream:old", 5);
    s.conn.frame({ channel: 5, kind: "data", payload: new Uint8Array([1, 2, 3, 4]) });
    const closed = once<void>((deliver) => s.client.onFrame((frame) => frame.kind === "close" && deliver()));
    s.conn.frame({ channel: 5, kind: "close", payload: new TextEncoder().encode('{"reason":"taken_over"}') });
    await within(closed, "the CLOSE frame");
    expect<unknown>([old.closed, old.delivered]).toEqual(["taken_over", above + 4n]);
    const fresh = await attach(s, "stream:new", 6);
    expect(kindOf(await rejection(old.ack(above + 4n)))).toBe("stream_closed");
    s.conn.frame({ channel: 6, kind: "data", payload: new Uint8Array([9, 9, 9]) });
    await control(s, "stream:new", ["input_ready"]);
    expect(kindOf(await rejection(fresh.ack(above + 4n)))).toBe("impossible_ack");
    await fresh.ack(above + 3n);
    expect(kindOf(await rejection(fresh.ack(above + 1n)))).toBe("impossible_ack");
    const credit = await s.conn.frames.next("the only CREDIT frame");
    expect<unknown>([credit.channel, credit.kind]).toEqual([6, "credit"]);
    expect(new DataView(credit.payload.buffer, credit.payload.byteOffset).getBigUint64(0, true)).toBe(above + 3n);
    expect(s.conn.frames.pending).toBe(0);
  } finally {
    s.client.close();
    s.peer.stop();
  }
}

/** A cursor the server can no longer replay is reported as missed, never silently restarted. */
export async function missedCursor(): Promise<void> {
  const s = await session(false);
  try {
    const opening = Subscription.open(s.client, {});
    s.conn.reply(await s.conn.requests.next("events.subscribe"), subscribed("sub:1", null));
    const subscription = await opening;
    s.conn.notify("event", envelope("sub:1", ABOVE_2_53, "terminal.updated", {}));
    await within(subscription.next(), "the first event");
    s.conn.vanish();
    await within(s.conn.gone, "the dropped connection");
    const conn = await reconnect(s);
    const resuming = subscription.resume();
    conn.reply(await conn.requests.next("events.subscribe"), subscribed("sub:2", "cursor_expired"));
    await within(resuming, "the refused resume");
    expect<unknown>([subscription.state, subscription.missedReport?.reason, subscription.cursor]).toEqual(["missed", "cursor_expired", null]);
  } finally {
    s.client.close();
    s.peer.stop();
  }
}

/** A u64 counter sent as a JSON number is not a reply this client accepts. */
export async function numericCounterReply(): Promise<void> {
  const s = await session(false);
  try {
    const stream = await attach(s, "stream:n", null);
    await control(s, "stream:n", ["input_ready"]);
    const sending = sendText(s.client, stream, { text: "x", submit: "none" });
    const request = await s.conn.requests.next("input.send");
    const result = inputResult(String(request.params.op_id)) as { receipt: Record<string, unknown> };
    s.conn.send({ jsonrpc: "2.0", id: request.id, result: { ...result, receipt: { ...result.receipt, queued_seq: 9007199254740993 } } });
    const error = (await within(rejection(sending), "the refused reply")) as ClientError;
    expect<unknown>([error.kind, error.fieldPath]).toEqual(["invalid_response", "result.receipt.queued_seq"]);
  } finally {
    s.client.close();
    s.peer.stop();
  }
}
