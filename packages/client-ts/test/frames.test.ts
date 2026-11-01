import { expect, test } from "bun:test";

import { U64_MAX } from "../src/decimal";
import { type Frame, FrameDecoder, creditPayload, encodeFrame } from "../src/wire/frames";
import { declared, decode, wireFixtures } from "./cases/fixtures";

// Seeded xorshift so every run explores the same segmentations; no retry can turn it green.
function generator(seed: number): () => number {
  let state = seed >>> 0 || 1;
  return () => {
    state ^= state << 13;
    state ^= state >>> 17;
    state ^= state << 5;
    return (state >>> 0) / 2 ** 32;
  };
}

function randomCuts(next: () => number, length: number): number[] {
  const count = Math.floor(next() * 32);
  return Array.from({ length: count }, () => Math.floor(next() * (length + 1))).sort((a, b) => a - b);
}

test("coalesced valid fixtures decode identically under 2000 seeded segmentations", async () => {
  const valid = (await wireFixtures()).filter((entry) => entry.error === undefined);
  const bytes = Buffer.concat(valid.map((entry) => Buffer.from(entry.hex, "hex")));
  const want = { frames: valid.flatMap((entry) => entry.decodes ?? []) };
  const next = generator(29);
  for (let round = 0; round < 2000; round += 1) {
    const cuts = randomCuts(next, bytes.byteLength);
    expect<unknown>([cuts, decode(bytes, cuts)]).toEqual([cuts, want]);
  }
});

test("every invalid fixture appended after valid frames fails with its typed kind under any split", async () => {
  const entries = await wireFixtures();
  const prefix = Buffer.from(entries.find((entry) => entry.name === "control-request")?.hex ?? "", "hex");
  const next = generator(53);
  for (const entry of entries.filter((candidate) => candidate.error !== undefined)) {
    const bytes = Buffer.concat([prefix, Buffer.from(entry.hex, "hex")]);
    for (let round = 0; round < 200; round += 1) {
      const result = decode(bytes, randomCuts(next, bytes.byteLength)) as { error?: string };
      expect([entry.name, result.error]).toEqual([entry.name, declared(entry).error]);
    }
  }
});

test("random frames round-trip, and a CREDIT payload carries u64::MAX exactly", () => {
  const next = generator(7);
  const kinds = ["data", "credit", "close"] as const;
  const frames: Frame[] = Array.from({ length: 300 }, () => ({
    channel: Math.floor(next() * 2 ** 32),
    kind: kinds[Math.floor(next() * 3)] ?? "data",
    payload: Uint8Array.from({ length: Math.floor(next() * 64) }, () => Math.floor(next() * 256)),
  }));
  const bytes = Buffer.concat(frames.map(encodeFrame));
  const decoded = new FrameDecoder().push(bytes);
  expect(decoded.map((frame) => [frame.channel, frame.kind, Buffer.from(frame.payload).toString("hex")])).toEqual(
    frames.map((frame) => [frame.channel, frame.kind, Buffer.from(frame.payload).toString("hex")]),
  );
  const credit = creditPayload(U64_MAX);
  expect(Buffer.from(credit).toString("hex")).toBe("ffffffffffffffff");
  expect(new DataView(creditPayload((1n << 53n) + 1n).buffer).getBigUint64(0, true)).toBe((1n << 53n) + 1n);
});
