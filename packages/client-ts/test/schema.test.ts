import { expect, test } from "bun:test";

import { checkComponent, checkParams, checkResult } from "../src/schema";

test("outputs keep added fields and added values at open-enum sites", () => {
  const control = { stream_id: "s", kind: "kind_from_a_newer_runtime", stream_offset: "9007199254740993", added_field: [1] };
  expect(checkComponent("AttachControl", control, "output")).toBeUndefined();
  expect(checkResult("op.get", { op_id: "o", method: "doc.set", state: "state_from_a_newer_runtime", updated_seq: "1" })).toBeUndefined();
});

test("outputs still refuse a missing required field, a numeric counter and an unknown closed value", () => {
  expect(checkComponent("AttachControl", { stream_id: "s", kind: "gap" }, "output")?.path).toBe("AttachControl.stream_offset");
  expect(checkComponent("AttachControl", { stream_id: "s", kind: "gap", stream_offset: 3 }, "output")?.path).toBe("AttachControl.stream_offset");
  expect(checkComponent("StreamMode", "mode_from_a_newer_runtime", "output")?.path).toBe("StreamMode");
});

test("inputs are closed: an added field or an open-enum value outside the catalog is refused", () => {
  const send = { op_id: "o", target: { terminal_id: "terminal:1" }, text: "x", submit: "none" };
  expect(checkParams("input.send", send)).toBeUndefined();
  expect(checkParams("input.send", { ...send, extra: true })?.path).toBe("params.extra");
  expect(checkParams("input.send", { ...send, submit: "tab" })?.path).toBe("params.submit");
  expect(checkParams("attach.open", { op_id: "o", target: { name: "n" }, role: "admin", client_id: "c", stream_mode: "plain" })?.path).toBe("params.role");
  expect(checkParams("pane.split", {})?.path).toBe("method");
});
