// Schema-true records a scripted peer answers with; the peer checks each against the schema.

import { limitDefaults } from "../../src/generated/stream";

export const ABOVE_2_53 = "9007199254740993";
export const LIFETIME = "lifetime:0191f1a2-0000-7000-8000-000000000029";

export function helloResult(): Record<string, unknown> {
  return {
    machine_id: "machine:peer",
    runtime_lifetime: LIFETIME,
    server_boot_id: "boot:1",
    generation: "1",
    version: "0.1.0",
    methods: ["attach.open", "attach.credit", "events.subscribe", "input.send", "op.get", "op.cancel"],
    capabilities: { transports: ["unix"], stream_modes: ["channel", "plain"], framed: true },
    limits: limitDefaults,
    server_time_ms: "1",
    endpoint_kind: "unix",
  };
}

export function lease(id: string, epoch: string): Record<string, unknown> {
  return { lease_id: id, epoch, role: "interact", holder: "client:t29" };
}

export function attachReady(stream: string, channel: number | null, start: string): Record<string, unknown> {
  return {
    outcome: "ready",
    credit_initial: limitDefaults.credit_initial_bytes,
    stream_id: stream,
    channel,
    lease: lease(`lease:${stream}`, "1"),
    first_paint: {
      output_offset: start,
      stream_offset: start,
      revision: "1",
      geometry: { cols: 80, rows: 24 },
      modes: { enabled: [], kitty_keyboard: 0, revision: "1" },
      cursor: { row: 0, col: 0, visible: true },
      history_lines: "0",
    },
  };
}

export function inputResult(opId: string): Record<string, unknown> {
  return {
    receipt: {
      op_id: opId,
      queued_seq: ABOVE_2_53,
      admitted_bytes: "3",
      output_cursor: { offset: "18446744073709551615" },
      mode_revision: "2",
    },
    status_at_send: { reported: false, records: {}, turn: "0", finish_generation: "0" },
  };
}

export function opRecord(opId: string, state: string, seq: string): Record<string, unknown> {
  return { op_id: opId, method: "input.send", state, updated_seq: seq };
}

export function subscribed(id: string, missed: string | null): Record<string, unknown> {
  return {
    subscription_id: id,
    snapshot_seq: ABOVE_2_53,
    replay_floor: "1",
    queue_events: 10_000,
    queue_bytes: limitDefaults.event_replay_bytes,
    heartbeat_ms: limitDefaults.heartbeat_ms,
    missed,
    snapshot: missed === null ? null : [],
  };
}

export function envelope(subscription: string, seq: string, type: string, data: unknown): Record<string, unknown> {
  return {
    seq,
    lifetime: LIFETIME,
    boot_id: "boot:1",
    machine_id: "machine:peer",
    subscription_id: subscription,
    ts_ms: "1",
    type,
    target: {},
    data,
  };
}
