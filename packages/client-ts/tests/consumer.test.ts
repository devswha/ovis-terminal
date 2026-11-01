// The packed artifact, installed into a project outside the repository, against the real Rust
// protocol fixture and a schema-checked scripted peer. These tests never import ../src.

import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { existsSync, readFileSync, realpathSync } from "node:fs";
import { join, sep } from "node:path";

import { CONFORMANCE_BIN, Consumer, packed } from "./support/consumer";

const SETUP_BOUND_MS = 240_000;
const CASE_BOUND_MS = 120_000;
const U64_MAX = "18446744073709551615";
let consumer: Consumer | undefined;

const installed = (): Consumer => {
  if (consumer === undefined) throw new Error("the consumer project was not installed");
  return consumer;
};

beforeAll(async () => {
  if (!existsSync(CONFORMANCE_BIN)) {
    throw new Error(`${CONFORMANCE_BIN} is missing: run \`cargo build --locked -p ovis-terminal-cli -p ovis-terminal-conformance\` first`);
  }
  consumer = await Consumer.install(await packed());
}, SETUP_BOUND_MS);

afterAll(() => consumer?.remove());

describe("happy", () => {
  test("every entry point resolves to the installed tarball's dist, outside the repository", async () => {
    const reported = (await installed().run("resolution")) as Record<string, string>;
    const resolved = Object.fromEntries(Object.entries(reported).map(([key, path]) => [key, realpathSync(path)]));
    const root = realpathSync(join(installed().dir, "node_modules", "@ovis-terminal", "client")) + sep;
    expect(Object.entries(resolved).filter(([key, path]) => key !== "consumer" && !path.startsWith(root))).toEqual([]);
    expect([resolved.root, resolved.protocol, resolved.openrpc]).toEqual([`${root}dist${sep}index.js`, `${root}dist${sep}protocol.js`, `${root}dist${sep}openrpc.json`]);
    expect(existsSync(join(root, "src"))).toBe(false);
    expect(readFileSync(resolved.manifest ?? "", "utf8")).toContain('"name": "@ovis-terminal/client"');
  }, CASE_BOUND_MS);

  test("the consumer typechecks against the packed declarations and every misuse is rejected", async () => {
    const ran = await installed().typecheck();
    expect([ran.code, ran.stdout, ran.stderr]).toEqual([0, "", ""]);
  }, CASE_BOUND_MS);

  test("one framed connection to the real server carries pipelined calls; future_status and u64 stay exact", async () => {
    const result = await installed().run("concurrentRpc");
    const value = { status: "future_status", offset: U64_MAX, above_2_53: "9007199254740993" };
    expect(result.framed).toBe(true);
    expect([result.set_value, result.read_value]).toEqual([value, value]);
    expect(result.page_from).toBe("67108864");
    expect(result.discover_is_export).toBe(true);
    expect(result.schema_version).toBe(result.document_version);
    expect([result.run_state, result.op_state]).toEqual(["live", "committed"]);
    expect(result.report).toEqual({ accepted: "1", failures: {}, panicked: "0" });
  }, CASE_BOUND_MS);

  test("an additive output enum value is kept as received and the known fields are unchanged", async () => {
    const result = await installed().run("futureStatus");
    expect(result.received).toEqual(result.sent);
    expect([result.known, result.updated_is_max, result.violations]).toEqual([false, true, 0]);
  }, CASE_BOUND_MS);

  test("typed attach outcomes, isolated stream bytes and per-stream credit", async () => {
    const result = await installed().run("attachAndCredit");
    expect(result.outcomes).toEqual(["ready", "ready", "refused", "future_outcome"]);
    expect([result.refused_stream, result.refused_reason]).toEqual([true, "held by another client"]);
    expect(result.bytes).toEqual(["hello", "xy"]);
    const credit = result.credit as { channel: number; kind: string; payload: number[]; expected: number[] };
    expect([credit.channel, credit.kind, credit.payload]).toEqual([3, "credit", credit.expected]);
    expect(result.impossible).toBe("impossible_ack");
  }, CASE_BOUND_MS);

  test("frames decode identically at every split, bytewise, and a truncated tail is typed", async () => {
    const result = await installed().run("fragmentedFrames");
    expect(result).toEqual({ frames: 4, whole_matches: true, splits: result.splits, mismatches: 0, bytewise_matches: true, truncated: "truncated_frame" });
    expect(result.splits).toBeGreaterThan(80);
  }, CASE_BOUND_MS);
});

describe("failure", () => {
  test("an expired op_id is the server's typed op_expired, sent once and never resent", async () => {
    const result = await installed().run("opExpired");
    expect(result).toEqual({
      rpc_error: true,
      outcome_unknown: false,
      server_kind: "op_expired",
      op_id: "01890000-0000-7000-8000-000000000001",
      retryable: false,
      drafts: 0,
      sent: 1,
      report: { accepted: "2", failures: {}, panicked: "0" },
    });
  }, CASE_BOUND_MS);

  test("a lost mutation reply is an inspectable draft; reconnect resends nothing", async () => {
    const result = await installed().run("lostMutationReply");
    expect([result.kind, result.draft_op, result.inspected_op]).toEqual(["outcome_unknown", result.op_id, result.op_id]);
    expect(result.pending).toEqual([result.op_id]);
    expect([result.sent_before_decision, result.sent_after_decision]).toEqual([1, 1]);
    expect(["cancelled_before_commit", "cancel_pending", "already_committed"]).toContain(result.discarded as string);
    expect([result.retry_after_discard, result.drafts_after, result.epoch]).toEqual(["draft_settled", 0, 2]);
  }, CASE_BOUND_MS);

  test("stale state: a closed stream's ack grants nothing and the next credit belongs to the live stream", async () => {
    const result = await installed().run("attachAndCredit");
    expect([result.closed, result.stale]).toEqual(["released", "stream_closed"]);
    const next = result.next as { channel: number; payload: number[]; expected: number[] };
    expect([next.channel, next.payload]).toEqual([5, next.expected]);
  }, CASE_BOUND_MS);

  test("stale state: reconnect resumes after the retained cursor and drops the replayed duplicate", async () => {
    const result = await installed().run("cursorReplay");
    expect([result.before, result.detached, result.after, result.stale_dropped, result.state]).toEqual([["5", "6"], "detached", "7", 1, "live"]);
    expect(result.resubscribe).toEqual({ filter: {}, snapshot: false, cursor: { runtime_lifetime: result.lifetime, seq: "6" } });
  }, CASE_BOUND_MS);

  test("a malformed server frame fails the decoder once; the mutation becomes a draft and is not resent", async () => {
    const result = await installed().run("malformedFrame");
    expect(result).toEqual({
      read_kind: "disconnected",
      read_cause: "unknown_kind",
      write_kind: "outcome_unknown",
      write_method: "doc.set",
      draft_op: result.draft_op,
      disconnects: ["unknown_kind"],
      failure_while_down: "unknown_kind",
      failure_after_reconnect: null,
      first_after_reconnect: "op.get",
    });
  }, CASE_BOUND_MS);

  test("Windows named pipes are refused with a typed error; Unix platforms use sockets", async () => {
    expect(await installed().run("transportChoice")).toEqual({ linux: "unix", darwin: "unix", windows: "unsupported_transport" });
  }, CASE_BOUND_MS);
});
