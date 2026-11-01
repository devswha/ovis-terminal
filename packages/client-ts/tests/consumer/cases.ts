// Consumer cases, run as `bun cases.ts <name>` inside the isolated project that installed the
// packed tarball. Each prints one JSON result line; tests/consumer.test.ts asserts on it.

import {
  Client,
  ClientError,
  FrameDecoder,
  OutcomeUnknown,
  RpcError,
  Streams,
  Subscription,
  U64_MAX,
  asId,
  creditPayload,
  decimal,
  encodeFrame,
  endpointTransport,
  isKnown,
  newOpId,
  type Frame,
} from "@ovis-terminal/client";
import document from "@ovis-terminal/client/openrpc.json";
import { type ClientId, type DocName, type OpId, OpStateValues, type TerminalId, type TerminalName, schemaVersion } from "@ovis-terminal/client/protocol";

import { Fixture, LIFETIME, Peer, type PeerConn, Relay, attachResult, envelope, once, rejection, subscribed, within } from "./support";

const EXPIRED_OP = "01890000-0000-7000-8000-000000000001" as OpId;
const run = { program: { argv: ["agent"] }, geometry: { cols: 80, rows: 24 } };
const doc = (name: string) => asId<DocName>(name);

async function withFixture<T>(body: (fixture: Fixture) => Promise<T>): Promise<T & { report: Record<string, unknown> }> {
  const fixture = await Fixture.start();
  let outcome: T;
  let report: Record<string, unknown>;
  try {
    outcome = await body(fixture);
  } finally {
    report = await fixture.stop();
  }
  return { ...outcome, report };
}

async function scripted(framed: boolean): Promise<{ peer: Peer; client: Client; conn: PeerConn }> {
  const peer = Peer.start();
  const connecting = Client.connect({ socketPath: peer.path, framed });
  const conn = await peer.accept();
  await conn.hello();
  return { peer, client: await within(connecting, "the scripted hello"), conn };
}

/** Pipelined calls of every kind on one framed connection to the real server. */
async function concurrentRpc() {
  return withFixture(async (fixture) => {
    const client = await Client.connect({ socketPath: fixture.ready.socket, credential: fixture.ready.bearer, framed: true });
    try {
      const value = { status: "future_status", offset: decimal(U64_MAX), above_2_53: "9007199254740993" };
      const opId = newOpId();
      const [set, page, discovered, ran] = await within(
        Promise.all([
          client.unchecked("doc.set", { op_id: opId, name: doc("pkg"), value, version: decimal(0) }),
          client.call("terminal.output", { target: { terminal_id: asId<TerminalId>("terminal:1") }, from_offset: decimal(U64_MAX), limit_bytes: decimal(4) }),
          client.call("rpc.discover", {}),
          client.call("terminal.run", { ...run, op_id: newOpId(), name: asId<TerminalName>("pkg") }),
        ]),
        "four pipelined replies",
      );
      const read = (await client.unchecked("doc.get", { name: doc("pkg") })) as { value: unknown };
      const op = await client.call("op.get", { op_id: opId });
      return {
        framed: client.framed,
        set_value: (set as { value: unknown }).value,
        read_value: read.value,
        page_from: page.from_offset,
        discover_is_export: JSON.stringify(discovered) === JSON.stringify(document),
        schema_version: schemaVersion,
        document_version: document.info.version,
        run_state: ran.terminal.state,
        op_state: op.state,
      };
    } finally {
      client.close();
    }
  });
}

/** An expired operation id is a typed refusal that reaches the server exactly once. */
async function opExpired() {
  return withFixture(async (fixture) => {
    const relay = Relay.start(fixture.ready.socket);
    const client = await Client.connect({ socketPath: relay.path, credential: fixture.ready.bearer });
    try {
      const error = await rejection(client.call("terminal.run", { ...run, op_id: EXPIRED_OP, name: asId<TerminalName>("expired") }));
      const typed = error as RpcError;
      await client.reconnect();
      await client.call("rpc.discover", {});
      return {
        rpc_error: error instanceof RpcError,
        outcome_unknown: error instanceof OutcomeUnknown,
        server_kind: typed.serverKind,
        op_id: typed.data.op_id,
        retryable: typed.data.retryable,
        drafts: client.drafts().length,
        sent: relay.count("terminal.run", EXPIRED_OP),
      };
    } finally {
      client.close();
      relay.stop();
    }
  });
}

/** A held mutation whose reply is lost is a draft; reconnecting resends nothing. */
async function lostMutationReply() {
  return withFixture(async (fixture) => {
    const relay = Relay.start(fixture.ready.socket);
    const client = await Client.connect({ socketPath: relay.path, credential: fixture.ready.bearer });
    try {
      const opId = newOpId();
      const held = once<unknown>((deliver) => client.on("$/fixture/held", deliver));
      const call = client.call("doc.set", { op_id: opId, name: doc("held"), value: "once", version: decimal(0) });
      await within(held, "durable accept of the held write");
      client.sever();
      const error = await within(rejection(call), "the lost reply");
      const draft = (error as OutcomeUnknown).draft;
      await client.reconnect();
      const pending = client.drafts().map((entry) => entry.opId);
      const inspected = await draft.inspect();
      const sentBeforeDecision = relay.count("doc.set", opId);
      const discarded = await draft.discard();
      const retryAfterDiscard = (() => {
        try {
          void draft.retry();
          return "retried";
        } catch (refused) {
          return (refused as ClientError).kind;
        }
      })();
      return {
        kind: (error as ClientError).kind,
        draft_op: draft.opId,
        op_id: opId,
        pending,
        inspected_op: inspected.op_id,
        sent_before_decision: sentBeforeDecision,
        sent_after_decision: relay.count("doc.set", opId),
        discarded: discarded.outcome,
        retry_after_discard: retryAfterDiscard,
        drafts_after: client.drafts().length,
        epoch: client.connection.epoch,
      };
    } finally {
      client.close();
      relay.stop();
    }
  });
}

/** Output enum values a newer runtime adds are kept as received; known fields are untouched. */
async function futureStatus() {
  const { peer, client, conn } = await scripted(false);
  try {
    const opId = newOpId();
    const sent = { op_id: opId, method: "input.send", state: "future_status", updated_seq: decimal(U64_MAX) };
    const reading = client.call("op.get", { op_id: opId });
    conn.reply(await conn.requests.next("op.get"), sent);
    const record = await within(reading, "the op.get reply");
    return {
      received: record,
      sent,
      known: isKnown(OpStateValues, record.state),
      updated_is_max: BigInt(record.updated_seq) === U64_MAX,
      violations: client.violations.length,
    };
  } finally {
    client.close();
    peer.stop();
  }
}

/** Typed attach outcomes, isolated streams and per-stream credit on one framed connection. */
async function attachAndCredit() {
  const { peer, client, conn } = await scripted(true);
  try {
    const streams = new Streams(client);
    const open = async (terminal: string, result: Record<string, unknown>) => {
      const opening = streams.open({
        op_id: newOpId(),
        client_id: asId<ClientId>("client:t63"),
        role: "observe",
        stream_mode: "channel",
        target: { terminal_id: asId<TerminalId>(terminal) },
      });
      conn.reply(await conn.requests.next("attach.open"), result);
      return within(opening, "the attach.open reply");
    };
    const first = await open("terminal:1", attachResult("ready", "stream:1", 3));
    const second = await open("terminal:2", attachResult("ready", "stream:2", 5));
    const refused = await open("terminal:3", attachResult("refused", null, null));
    const future = await open("terminal:4", attachResult("future_outcome", null, null));
    const one = first.stream;
    const two = second.stream;
    if (one === undefined || two === undefined) throw new Error("ready attaches carry streams");
    const delivered = once<Frame>((deliver) => client.onFrame((frame) => frame.channel === 5 && deliver(frame)));
    conn.write(encodeFrame({ channel: 3, kind: "data", payload: new TextEncoder().encode("hello") }));
    conn.write(encodeFrame({ channel: 5, kind: "data", payload: new TextEncoder().encode("xy") }));
    await within(delivered, "the second stream's data");
    await one.ack(5n);
    const credit = await conn.frames.next("the first stream's credit");
    const impossible = await rejection(one.ack(6n));
    const closedFrame = once<Frame>((deliver) => client.onFrame((frame) => frame.kind === "close" && deliver(frame)));
    conn.write(encodeFrame({ channel: 3, kind: "close", payload: new TextEncoder().encode(JSON.stringify({ reason: "released" })) }));
    await within(closedFrame, "the first stream's close");
    const stale = await rejection(one.ack(5n));
    await two.ack(2n);
    const next = await conn.frames.next("the next credit frame");
    return {
      outcomes: [first.result.outcome, second.result.outcome, refused.result.outcome, future.result.outcome],
      refused_stream: refused.stream === undefined,
      refused_reason: refused.result.reason,
      bytes: [new TextDecoder().decode(one.bytes()), new TextDecoder().decode(two.bytes())],
      credit: { channel: credit.channel, kind: credit.kind, payload: [...credit.payload], expected: [...creditPayload(5n)] },
      impossible: (impossible as ClientError).kind,
      closed: one.closed,
      stale: (stale as ClientError).kind,
      next: { channel: next.channel, payload: [...next.payload], expected: [...creditPayload(2n)] },
    };
  } finally {
    client.close();
    peer.stop();
  }
}

/** Reconnect resumes after the retained cursor and drops a replayed duplicate. */
async function cursorReplay() {
  const { peer, client, conn } = await scripted(false);
  try {
    const subscribing = Subscription.open(client, {}, null);
    conn.reply(await conn.requests.next("events.subscribe"), subscribed("sub:1"));
    const subscription = await within(subscribing, "the subscription");
    conn.notify("event", "EventEnvelope", envelope("sub:1", "5", "terminal.output"));
    conn.notify("event", "EventEnvelope", envelope("sub:1", "6", "future.kind"));
    const before = [(await within(subscription.next(), "seq 5")).seq, (await within(subscription.next(), "seq 6")).seq];
    const gone = once<unknown>((deliver) => client.onDisconnect(deliver));
    conn.vanish();
    await within(gone, "the disconnect");
    const detached = subscription.state;
    const reconnecting = client.reconnect();
    const again = await peer.accept();
    await again.hello();
    await within(reconnecting, "the reconnect");
    const resuming = subscription.resume();
    const resubscribe = await again.requests.next("the resubscribe");
    again.reply(resubscribe, subscribed("sub:2"));
    await within(resuming, "the resume");
    again.notify("event", "EventEnvelope", envelope("sub:2", "6", "future.kind"));
    again.notify("event", "EventEnvelope", envelope("sub:2", "7", "terminal.output"));
    const after = (await within(subscription.next(), "seq 7")).seq;
    return {
      before,
      detached,
      resubscribe: resubscribe.params,
      lifetime: LIFETIME,
      after,
      stale_dropped: subscription.staleDropped,
      state: subscription.state,
    };
  } finally {
    client.close();
    peer.stop();
  }
}

/** A frame no generation defines ends the connection once with a typed decoder error. */
async function malformedFrame() {
  const { peer, client, conn } = await scripted(true);
  try {
    const disconnects: (string | undefined)[] = [];
    client.onDisconnect((failure) => disconnects.push(failure?.kind));
    const opId = newOpId();
    const reading = client.call("op.get", { op_id: opId });
    const writing = client.call("doc.set", { op_id: opId, name: doc("x"), value: 1, version: decimal(0) });
    const read = await conn.requests.next("op.get");
    const write = await conn.requests.next("doc.set");
    const bad = new Uint8Array([0, 0, 0, 0, 0x7f, 0, 0, 0, 0]);
    const valid = conn.encode({ jsonrpc: "2.0", id: read.id, result: { op_id: opId, method: "doc.set", state: "committed", updated_seq: "1" } });
    conn.write(Buffer.concat([bad, valid]));
    const readError = (await within(rejection(reading), "the read rejection")) as ClientError;
    const writeError = (await within(rejection(writing), "the write rejection")) as OutcomeUnknown;
    const failureWhileDown = client.failure?.kind ?? null;
    const reconnecting = client.reconnect();
    const again = await peer.accept();
    await again.hello();
    await within(reconnecting, "the reconnect");
    const probing = client.call("op.get", { op_id: opId });
    const firstAfter = await again.requests.next("the first request after reconnect");
    again.reply(firstAfter, { op_id: opId, method: "doc.set", state: "outcome_unknown", updated_seq: "2" });
    await within(probing, "the probe reply");
    return {
      read_kind: readError.kind,
      read_cause: (readError.cause as ClientError | undefined)?.kind,
      write_kind: writeError.kind,
      write_method: write.method,
      draft_op: writeError.draft.opId,
      disconnects: [...disconnects],
      failure_while_down: failureWhileDown,
      failure_after_reconnect: client.failure?.kind ?? null,
      first_after_reconnect: firstAfter.method,
    };
  } finally {
    client.close();
    peer.stop();
  }
}

/** Frames decode identically at every split of the same bytes. */
function fragmentedFrames() {
  const frames: Frame[] = [
    { channel: 0, kind: "data", payload: new TextEncoder().encode(JSON.stringify({ jsonrpc: "2.0", id: "c1", result: {} })) },
    { channel: 3, kind: "data", payload: new Uint8Array([0, 1, 2, 0xff]) },
    { channel: 3, kind: "credit", payload: creditPayload(U64_MAX) },
    { channel: 5, kind: "close", payload: new TextEncoder().encode('{"reason":"released"}') },
  ];
  const bytes = Buffer.concat(frames.map(encodeFrame));
  const show = (decoded: Frame[]) => JSON.stringify(decoded.map((frame) => [frame.channel, frame.kind, [...frame.payload]]));
  const whole = show(new FrameDecoder().push(bytes));
  let splits = 0;
  let mismatches = 0;
  for (let cut = 1; cut < bytes.byteLength; cut += 1) {
    const decoder = new FrameDecoder();
    const parts = [...decoder.push(bytes.subarray(0, cut)), ...decoder.push(bytes.subarray(cut))];
    decoder.finish();
    splits += 1;
    if (show(parts) !== whole) mismatches += 1;
  }
  const bytewise = new FrameDecoder();
  const each = [...bytes].flatMap((byte) => bytewise.push(new Uint8Array([byte])));
  const truncated = new FrameDecoder();
  truncated.push(bytes.subarray(0, bytes.byteLength - 1));
  const finish = (() => {
    try {
      truncated.finish();
      return "finished";
    } catch (error) {
      return (error as ClientError).kind;
    }
  })();
  return { frames: frames.length, whole_matches: whole === show(frames), splits, mismatches, bytewise_matches: show(each) === whole, truncated: finish };
}

/** The platform transport decision: Unix sockets, and a typed refusal for Windows pipes. */
function transportChoice() {
  const windows = (() => {
    try {
      return endpointTransport("win32");
    } catch (error) {
      return (error as ClientError).kind;
    }
  })();
  return { linux: endpointTransport("linux"), darwin: endpointTransport("darwin"), windows };
}

/** Where every public entry point resolves from inside the consumer project. */
function resolution() {
  return {
    root: Bun.resolveSync("@ovis-terminal/client", import.meta.dir),
    protocol: Bun.resolveSync("@ovis-terminal/client/protocol", import.meta.dir),
    openrpc: Bun.resolveSync("@ovis-terminal/client/openrpc.json", import.meta.dir),
    manifest: Bun.resolveSync("@ovis-terminal/client/package.json", import.meta.dir),
    consumer: import.meta.dir,
  };
}

const cases: Record<string, () => unknown> = {
  concurrentRpc,
  opExpired,
  lostMutationReply,
  futureStatus,
  attachAndCredit,
  cursorReplay,
  malformedFrame,
  fragmentedFrames,
  transportChoice,
  resolution,
};

const chosen = cases[process.argv[2] ?? ""];
if (chosen === undefined) throw new Error(`unknown case ${process.argv[2]}; known: ${Object.keys(cases).join(", ")}`);
const result = await chosen();
process.stdout.write(`${JSON.stringify(result, (_, value) => (typeof value === "bigint" ? value.toString() : value))}\n`);
process.exit(0);
