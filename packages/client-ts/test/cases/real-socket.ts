// Conversations with the real Rust server fixture over its Unix socket.

import { expect } from "bun:test";

import { asId } from "../../src/brand";
import { Client } from "../../src/client";
import { U64_MAX, decimal, toBigInt } from "../../src/decimal";
import { OutcomeUnknown } from "../../src/drafts";
import { ClientError, RpcError } from "../../src/errors";
import type { DocName, TerminalId } from "../../src/generated/components";
import document from "../../src/generated/openrpc.json";
import { newOpId } from "../../src/input";
import { once, rejection, within } from "../support/bounded";
import { Fixture } from "../support/fixture";
import { ABOVE_2_53 } from "../support/records";

const HELD = "$/fixture/held";
const doc = (name: string) => asId<DocName>(name);
const terminal1 = { terminal_id: asId<TerminalId>("terminal:1") };

async function withFixture(body: (fixture: Fixture, connect: (framed: boolean) => Promise<Client>) => Promise<void>): Promise<Record<string, unknown>> {
  const fixture = await Fixture.start();
  const clients: Client[] = [];
  let report: Record<string, unknown> = {};
  try {
    await body(fixture, async (framed) => {
      const client = await Client.connect({ socketPath: fixture.ready.socket, credential: fixture.ready.bearer, framed });
      clients.push(client);
      return client;
    });
  } finally {
    for (const client of clients) client.close();
    report = { ...(await fixture.stop()) };
  }
  return report;
}

/** One framed connection carries concurrent correlated calls; u64 values stay exact. */
export async function persistentConversation(): Promise<void> {
  const report = await withFixture(async (_, connect) => {
    const client = await connect(true);
    expect(client.framed).toBe(true);
    expect(String(client.connection.hello.limits.max_frame_bytes)).toBe("16777216");
    const wide = { offset: decimal(U64_MAX), above_2_53: ABOVE_2_53, kind: "kind_from_a_newer_runtime" };
    const opId = newOpId();
    const [set, page, discovered] = await within(
      Promise.all([
        client.unchecked("doc.set", { op_id: opId, name: doc("wide"), value: wide, version: decimal(0) }),
        client.call("terminal.output", { target: terminal1, from_offset: decimal(U64_MAX), limit_bytes: decimal(4) }),
        client.call("rpc.discover", {}),
      ]),
      "three pipelined replies",
    );
    expect((set as { value: unknown }).value).toEqual(wide);
    expect(toBigInt(page.from_offset)).toBe(64n * 1024n * 1024n);
    expect(discovered).toEqual(document);
    const read = (await client.unchecked("doc.get", { name: doc("wide") })) as { value: typeof wide };
    expect(read.value).toEqual(wide);
    expect(toBigInt(read.value.offset)).toBe(U64_MAX);
    const op = await client.call("op.get", { op_id: opId });
    expect([op.op_id, op.state, op.method] as string[]).toEqual([opId, "committed", "doc.set"]);
  });
  expect(report).toEqual({ accepted: "1", failures: {}, panicked: "0" });
}

/** Aborting a held mutation sends op.cancel; both requests settle on the same connection. */
export async function cancellation(): Promise<void> {
  await withFixture(async (_, connect) => {
    const client = await connect(false);
    const opId = newOpId();
    const held = once<unknown>((deliver) => client.on(HELD, deliver));
    const abort = new AbortController();
    const call = client.call("doc.set", { op_id: opId, name: doc("held"), value: "never", version: decimal(0) }, { signal: abort.signal });
    expect(client.lateReplies).toBe(0);
    await within(held, "the held notification");
    abort.abort();
    const error = await within(rejection(call), "the cancelled reply");
    expect(error).toBeInstanceOf(RpcError);
    expect((error as RpcError).serverKind).toBe("cancelled");
    expect((await client.call("op.get", { op_id: opId })).state).toBe("cancelled");
    expect(((await client.unchecked("doc.get", { name: doc("held") })) as { value: unknown }).value).toBeNull();
  });
}

/** Each shared strict rejection: the local check and the server agree on kind and field path. */
export async function strictRejections(): Promise<void> {
  const fixture = (await Bun.file(`${import.meta.dir}/../../../../conformance/fixtures/protocol/strict-rejections.json`).json()) as {
    cases: { name: string; method: string; params: unknown; kind: string; field_path?: string }[];
  };
  expect(fixture.cases.length).toBeGreaterThanOrEqual(8);
  const report = await withFixture(async (_, connect) => {
    const client = await connect(false);
    for (const entry of fixture.cases) {
      const params = JSON.parse(JSON.stringify(entry.params).replaceAll(/"\$op:[a-z]+"/g, () => JSON.stringify(newOpId())));
      const server = (await rejection(client.unchecked(entry.method, params))) as RpcError;
      expect([entry.name, server.serverKind, server.data.field_path]).toEqual([entry.name, entry.kind, entry.field_path]);
      const local = await rejection((client.call as (m: string, p: unknown) => Promise<unknown>)(entry.method, params));
      const schemaLevel = entry.kind === "invalid_params" || entry.kind === "method_not_found";
      if (schemaLevel) {
        expect(local).toBeInstanceOf(ClientError);
        expect(local).not.toBeInstanceOf(RpcError);
        expect([(local as ClientError).kind, (local as ClientError).fieldPath]).toEqual([entry.kind, entry.field_path]);
      } else {
        expect((local as RpcError).serverKind).toBe(entry.kind);
      }
    }
    const numeric = await rejection(client.call("doc.set", { op_id: newOpId(), name: doc("n"), value: 1, version: 0 as never }));
    expect([(numeric as ClientError).kind, (numeric as ClientError).fieldPath]).toEqual(["invalid_params", "params.version"]);
    expect(((await client.unchecked("doc.get", { name: doc("n") })) as { value: unknown }).value).toBeNull();
  });
  expect(report.panicked).toBe("0");
}

/** A reply lost after durable accept leaves a draft; nothing is resent until the caller decides. */
export async function lostReplyAfterAdmission(): Promise<void> {
  await withFixture(async (_, connect) => {
    const client = await connect(false);
    const opId = newOpId();
    const held = once<unknown>((deliver) => client.on(HELD, deliver));
    const call = client.call("doc.set", { op_id: opId, name: doc("held"), value: "once", version: decimal(0) });
    await within(held, "durable accept of the held write");
    client.sever();
    const error = await within(rejection(call), "the lost reply");
    expect(error).toBeInstanceOf(OutcomeUnknown);
    const draft = (error as OutcomeUnknown).draft;
    expect([(error as OutcomeUnknown).kind, draft.opId, draft.decision]).toEqual(["outcome_unknown", opId, "pending"]);
    await client.reconnect();
    expect(client.connection.epoch).toBe(2);
    expect(client.drafts()).toEqual([draft]);
    const inspected = await draft.inspect();
    expect(inspected.op_id).toBe(opId);
    const discarded = await draft.discard();
    expect(["cancelled_before_commit", "cancel_pending", "already_committed"]).toContain(discarded.outcome);
    expect(client.drafts()).toEqual([]);
    expect(() => draft.retry()).toThrow(ClientError);
    const value = ((await client.unchecked("doc.get", { name: doc("held") })) as { value: unknown }).value;
    expect(value).toEqual(discarded.outcome === "already_committed" ? "once" : null);
  });
}

/** A client deadline settles the call with a draft while the operation stays inspectable. */
export async function clientDeadline(): Promise<void> {
  await withFixture(async (_, connect) => {
    const client = await connect(false);
    const opId = newOpId();
    const held = once<unknown>((deliver) => client.on(HELD, deliver));
    const call = client.call("doc.set", { op_id: opId, name: doc("held"), value: "late", version: decimal(0) }, { deadlineMs: 50 });
    const error = (await within(rejection(call), "the client deadline")) as OutcomeUnknown;
    expect([error.kind, error.draft.opId]).toEqual(["deadline_exceeded", opId]);
    await within(held, "the held notification");
    expect(["accepted", "running"]).toContain((await error.draft.inspect()).state);
    expect((await error.draft.discard()).outcome).toBe("cancelled_before_commit");
    expect(client.drafts()).toEqual([]);
    expect(((await client.unchecked("doc.get", { name: doc("held") })) as { value: unknown }).value).toBeNull();
  });
}
