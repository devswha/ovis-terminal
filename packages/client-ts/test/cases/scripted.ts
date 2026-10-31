// Client behavior on routes the fixture does not serve yet, against a schema-checked peer.

import { expect } from "bun:test";

import { asId, isKnown } from "../../src/brand";
import { Client } from "../../src/client";
import { type ClientId, type TerminalId, TransportKindValues } from "../../src/generated/components";
import { newOpId, sendText } from "../../src/input";
import { type Stream, Streams } from "../../src/streams";
import { Subscription } from "../../src/subscriptions";
import { once, within } from "../support/bounded";
import { Peer, type PeerConn } from "../support/peer";
import { ABOVE_2_53, LIFETIME, attachReady, envelope, inputResult, opRecord, subscribed } from "../support/records";

export const target = { terminal_id: asId<TerminalId>("terminal:7") };
export const above = BigInt(ABOVE_2_53);

export interface Session {
  readonly peer: Peer;
  readonly client: Client;
  readonly conn: PeerConn;
  readonly streams: Streams;
}

export async function session(framed: boolean, hello: Record<string, unknown> = {}): Promise<Session> {
  const peer = Peer.start();
  const connecting = Client.connect({ socketPath: peer.path, framed });
  const conn = await peer.accept();
  await conn.hello(hello);
  const client = await within(connecting, "the hello reply");
  return { peer, client, conn, streams: new Streams(client) };
}

export async function attach(s: Session, stream: string, channel: number | null): Promise<Stream> {
  const opening = s.streams.open({ op_id: newOpId(), target, role: "interact", client_id: asId<ClientId>("client:t29"), stream_mode: channel === null ? "plain" : "channel" });
  const request = await s.conn.requests.next("attach.open");
  s.conn.reply(request, attachReady(stream, channel, ABOVE_2_53));
  const attached = await within(opening, "the attach.open reply");
  expect(attached.result.outcome).toBe("ready");
  return attached.stream as Stream;
}

export async function control(s: Session, stream: string, kinds: readonly string[]): Promise<void> {
  let seen = 0;
  const all = once<void>((deliver) => s.client.on("attach.control", () => ++seen === kinds.length && deliver()));
  for (const kind of kinds) s.conn.notify("attach.control", { stream_id: stream, kind, stream_offset: ABOVE_2_53 });
  await within(all, `control records ${kinds.join(", ")}`);
}

export async function reconnect(s: Session): Promise<PeerConn> {
  const reconnecting = s.client.reconnect();
  const conn = await s.peer.accept();
  await conn.hello();
  await within(reconnecting, "the second hello");
  return conn;
}

/** Framed stream: bytes above 2^53 are offset exactly, credit is a CREDIT frame bound to the stream's channel. */
export async function framedStreamAndUnknownEnums(): Promise<void> {
  const s = await session(true, { endpoint_kind: "quic_from_a_newer_runtime", field_from_a_newer_runtime: { any: "shape" } });
  try {
    const kind = s.client.connection.hello.endpoint_kind;
    expect<unknown>([kind, isKnown(TransportKindValues, kind)]).toEqual(["quic_from_a_newer_runtime", false]);
    const stream = await attach(s, "stream:a", 5);
    s.conn.frame({ channel: 5, kind: "data", payload: new TextEncoder().encode("hello") });
    await control(s, "stream:a", ["input_ready", "kind_from_a_newer_runtime"]);
    expect(stream.delivered).toBe(above + 5n);
    expect(new TextDecoder().decode(stream.bytes())).toBe("hello");
    expect<unknown>([stream.inputReady, stream.controls.map((record) => record.kind)]).toEqual([true, ["input_ready", "kind_from_a_newer_runtime"]]);
    await stream.ack(above + 5n);
    const credit = await s.conn.frames.next("the CREDIT frame");
    expect([credit.channel, credit.kind]).toEqual([5, "credit"]);
    expect(new DataView(credit.payload.buffer, credit.payload.byteOffset).getBigUint64(0, true)).toBe(above + 5n);
    expect(s.client.violations).toEqual([]);
  } finally {
    s.client.close();
    s.peer.stop();
  }
}

/** Plain stream and input: the admission receipt is separate from the later delivery update. */
export async function admissionSeparateFromDelivery(): Promise<void> {
  const s = await session(false);
  try {
    const subscribing = Subscription.open(s.client, { types: ["op.committed"] });
    s.conn.reply(await s.conn.requests.next("events.subscribe"), subscribed("sub:1", null));
    const subscription = await subscribing;
    const stream = await attach(s, "stream:p", null);
    await control(s, "stream:p", ["input_ready"]);
    const sending = sendText(s.client, stream, { text: "ls\n", submit: "none" });
    const request = await s.conn.requests.next("input.send");
    expect<unknown>([request.params.lease_id, request.params.target]).toEqual(["lease:stream:p", target]);
    s.conn.reply(request, inputResult(String(request.params.op_id)));
    const admission = await within(sending, "the admission receipt");
    expect<unknown>([admission.receipt.queued_seq, admission.receipt.output_cursor.offset]).toEqual([ABOVE_2_53, "18446744073709551615"]);
    const update = admission.update(subscription);
    s.conn.notify("event", envelope("sub:1", "9007199254740994", "op.committed", opRecord(admission.opId, "committed", "9007199254740994")));
    const delivered = await within(update, "the op.committed update");
    expect<unknown>([delivered.op_id, delivered.state]).toEqual([admission.opId, "committed"]);
    expect<unknown>(subscription.cursor).toEqual({ runtime_lifetime: LIFETIME, seq: "9007199254740994" });
    const arrived = once<unknown>((deliver) => s.client.on("attach.data", deliver));
    s.conn.notify("attach.data", { stream_id: "stream:p", offset: ABOVE_2_53, data_b64: Buffer.from("abc").toString("base64") });
    await within(arrived, "the attach.data record");
    expect(stream.delivered).toBe(above + 3n);
    const acking = stream.ack(above + 3n);
    const credit = await s.conn.requests.next("attach.credit");
    expect(credit.params).toEqual({ stream_id: "stream:p", acknowledged: "9007199254740996" });
    s.conn.reply(credit, { stream_id: "stream:p", stream_offset: "9007199254740996" });
    await within(acking, "the attach.credit reply");
  } finally {
    s.client.close();
    s.peer.stop();
  }
}

/** Reconnect resumes after the exact retained cursor and drops a replayed duplicate. */
export async function cursorResume(): Promise<void> {
  const s = await session(false);
  try {
    const opening = Subscription.open(s.client, {});
    s.conn.reply(await s.conn.requests.next("events.subscribe"), subscribed("sub:1", null));
    const subscription = await opening;
    for (const seq of [ABOVE_2_53, "9007199254740994"]) s.conn.notify("event", envelope("sub:1", seq, "terminal.updated", {}));
    expect<unknown>((await within(subscription.next(), "first event")).seq).toBe(ABOVE_2_53);
    expect<unknown>((await within(subscription.next(), "second event")).seq).toBe("9007199254740994");
    s.conn.vanish();
    await within(s.conn.gone, "the dropped connection");
    const conn = await reconnect(s);
    expect(subscription.state).toBe("detached");
    const resuming = subscription.resume();
    const request = await conn.requests.next("events.subscribe");
    expect(request.params.cursor).toEqual({ runtime_lifetime: LIFETIME, seq: "9007199254740994" });
    conn.reply(request, subscribed("sub:2", null));
    await within(resuming, "the resumed subscription");
    for (const seq of ["9007199254740994", "9007199254740995"]) conn.notify("event", envelope("sub:2", seq, "terminal.updated", {}));
    expect<unknown>((await within(subscription.next(), "the next new event")).seq).toBe("9007199254740995");
    expect([subscription.state, subscription.staleDropped]).toEqual(["live", 1]);
  } finally {
    s.client.close();
    s.peer.stop();
  }
}
