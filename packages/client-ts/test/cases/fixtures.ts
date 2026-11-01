// The language-neutral wire fixtures, replayed unchanged by the Bun decoder and compared
// byte-for-byte with what the Rust conformance runner sent over the real socket.

import { expect } from "bun:test";
import { createHash } from "node:crypto";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { ClientError } from "../../src/errors";
import { type Frame, FrameDecoder, encodeFrame } from "../../src/wire/frames";
import { within } from "../support/bounded";
import { CONFORMANCE_BIN, OVIS_TERMINAL_BIN, REPO, requireBinary } from "../support/fixture";

export interface WireEntry {
  readonly name: string;
  readonly hex: string;
  readonly decodes?: readonly { readonly channel: string; readonly kind: string; readonly payload_hex: string }[];
  readonly error?: string;
}

export const FRAMES_PATH = join(REPO, "conformance", "fixtures", "wire", "frames.json");

export async function wireFixtures(): Promise<readonly WireEntry[]> {
  const file = (await Bun.file(FRAMES_PATH).json()) as { format: string; frames: WireEntry[] };
  expect(file.format).toBe("ovis-wire-fixture/1");
  return file.frames;
}

const bytesOf = (hex: string) => Uint8Array.from(Buffer.from(hex, "hex"));
const shown = (frame: Frame) => ({ channel: String(frame.channel), kind: frame.kind, payload_hex: Buffer.from(frame.payload).toString("hex") });
const sha256 = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");

/** What the decoder makes of `bytes` cut at `cuts`, in the shape the fixture declares. */
export function decode(bytes: Uint8Array, cuts: readonly number[]): unknown {
  const decoder = new FrameDecoder();
  const frames: unknown[] = [];
  let start = 0;
  try {
    for (const end of [...cuts, bytes.byteLength]) {
      frames.push(...decoder.push(bytes.subarray(start, Math.max(end, start))).map(shown));
      start = Math.max(end, start);
    }
    decoder.finish();
  } catch (error) {
    return { error: (error as ClientError).kind };
  }
  return { frames };
}

export const declared = (entry: WireEntry) => (entry.error === undefined ? { frames: entry.decodes } : { error: entry.error });

/** Every entry decodes as declared at every single split and byte by byte; valid ones re-encode to the same bytes. */
export async function everySplit(): Promise<void> {
  const entries = await wireFixtures();
  expect(entries.length).toBeGreaterThanOrEqual(9);
  for (const entry of entries) {
    const bytes = bytesOf(entry.hex);
    for (let cut = 0; cut <= bytes.byteLength; cut += 1) {
      expect<unknown>([entry.name, cut, decode(bytes, [cut])]).toEqual([entry.name, cut, declared(entry)]);
    }
    const bytewise = Array.from({ length: Math.max(bytes.byteLength - 1, 0) }, (_, index) => index + 1);
    expect<unknown>([entry.name, decode(bytes, bytewise)]).toEqual([entry.name, declared(entry)]);
    if (entry.error === undefined) {
      const frames = new FrameDecoder().push(bytes);
      expect(Buffer.concat(frames.map(encodeFrame)).toString("hex")).toBe(entry.hex);
    }
  }
}

interface Transcript {
  readonly direction: string;
  readonly unit: string;
  readonly bytes: string;
  readonly sha256: string;
  readonly text: string | null;
}

/** The Rust runner replays the same fixture over the real binary; its recorded hashes equal Bun's. */
export async function rustReplayMatches(): Promise<void> {
  await requireBinary(CONFORMANCE_BIN);
  await requireBinary(OVIS_TERMINAL_BIN);
  const dir = mkdtempSync(join(tmpdir(), "t29e-"));
  try {
    const evidence = join(dir, "task-29.json");
    const argv = [CONFORMANCE_BIN, "--binary", OVIS_TERMINAL_BIN, "--scenario", "part-a/task-29", "--isolated", "--case", "happy", "--evidence", evidence];
    const run = Bun.spawn(argv, { cwd: REPO, stdout: "pipe", stderr: "pipe" });
    const code = await within(run.exited, "the conformance runner", 120_000);
    if (code !== 0) {
      const written = (await Bun.file(evidence).exists()) ? JSON.stringify((await Bun.file(evidence).json()).cases?.map((c: { failure: unknown }) => c.failure)) : "no evidence";
      throw new Error(`${argv.join(" ")} exited ${code}: ${await new Response(run.stdout).text()} ${await new Response(run.stderr).text()} failures=${written}`);
    }
    const report = (await Bun.file(evidence).json()) as {
      result: string;
      cases: { descriptors: { source: string }[]; connections: { name: string; transcript: Transcript[] }[] }[];
    };
    expect(report.result).toBe("passed");
    const replayed = report.cases[0]?.descriptors.map((d) => d.source).filter((source) => source.startsWith("fixtures/wire/frames.json#")) ?? [];
    expect(replayed).toEqual(["fixtures/wire/frames.json#control-request", "fixtures/wire/frames.json#coalesced-requests"]);
    const transcript = report.cases[0]?.connections.find((conn) => conn.name === "bridge")?.transcript ?? [];
    const sent = transcript.filter((record) => record.direction === "sent");
    const replies = transcript.filter((record) => record.direction === "received").map((record) => JSON.parse(record.text ?? "null") as { id?: string });
    const entries = await wireFixtures();
    for (const source of replayed) {
      const entry = entries.find((candidate) => `fixtures/wire/frames.json#${candidate.name}` === source) as WireEntry;
      const bytes = bytesOf(entry.hex);
      expect(sent.some((record) => record.sha256 === sha256(bytes) && record.bytes === String(bytes.byteLength))).toBe(true);
      for (const frame of new FrameDecoder().push(bytes)) {
        const id = (JSON.parse(new TextDecoder().decode(frame.payload)) as { id: string }).id;
        expect(replies.filter((reply) => reply.id === id)).toHaveLength(1);
      }
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}
