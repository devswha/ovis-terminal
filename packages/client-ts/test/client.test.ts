import { describe, test } from "bun:test";

import { everySplit, rustReplayMatches } from "./cases/fixtures";
import { cancellation, clientDeadline, lostReplyAfterAdmission, persistentConversation, strictRejections } from "./cases/real-socket";
import { admissionSeparateFromDelivery, cursorResume, framedStreamAndUnknownEnums } from "./cases/scripted";
import { lostInputReply, missedCursor, numericCounterReply, staleStreamAck } from "./cases/scripted-failure";

const SOCKET_BOUND_MS = 60_000;
const RUNNER_BOUND_MS = 180_000;

describe("happy", () => {
  test("happy: shared wire fixtures decode as declared at every split and re-encode byte-exactly", everySplit);
  test("happy: the Rust runner replayed the same fixture bytes through the real binary (sha256 match)", rustReplayMatches, RUNNER_BOUND_MS);
  test("happy: one persistent real-socket connection carries pipelined calls with exact u64 values", persistentConversation, SOCKET_BOUND_MS);
  test("happy: aborting a held mutation cancels it before commit on the same connection", cancellation, SOCKET_BOUND_MS);
  test("happy: framed stream offsets above 2^53, channel-bound credit and unknown output enums survive", framedStreamAndUnknownEnums);
  test("happy: input admission is separate from its later delivery update; plain credit stays exact", admissionSeparateFromDelivery);
  test("happy: reconnect resumes after the exact retained cursor and drops a replayed duplicate", cursorResume);
});

describe("failure", () => {
  test("failure: shared strict rejections are typed identically by the local check and the real server", strictRejections, SOCKET_BOUND_MS);
  test("failure: a reply lost after durable accept leaves a draft that is never resent", lostReplyAfterAdmission, SOCKET_BOUND_MS);
  test("failure: a client deadline reports deadline_exceeded with an inspectable draft", clientDeadline, SOCKET_BOUND_MS);
  test("failure: lost reply after input admission is outcome_unknown, revokes input-ready, never resent", lostInputReply);
  test("failure: an old stream's ack cannot grant credit to the new stream", staleStreamAck);
  test("failure: an unreplayable cursor is reported missed", missedCursor);
  test("failure: a u64 counter sent as a JSON number is invalid_response", numericCounterReply);
});
