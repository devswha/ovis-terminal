import { expect, test } from "bun:test";

import { U64_MAX, decimal, parseDecimal, toBigInt } from "../src/decimal";
import { ClientError } from "../src/errors";

const kind = (run: () => unknown) => {
  try {
    run();
  } catch (error) {
    return (error as ClientError).kind;
  }
  return "accepted";
};

test("u64::MAX and 2^53 + 1 round-trip exactly through decimal strings", () => {
  for (const value of [0n, 1n, (1n << 53n) + 1n, U64_MAX - 1n, U64_MAX]) {
    const text = decimal(value);
    expect<string>(text).toBe(value.toString());
    expect(toBigInt(parseDecimal(text, "value"))).toBe(value);
  }
  expect(JSON.parse(JSON.stringify({ seq: decimal(U64_MAX) })).seq).toBe("18446744073709551615");
});

test("numbers, non-canonical text and out-of-range values are refused with invalid_decimal", () => {
  for (const value of [9007199254740993, "01", "-1", "1.0", " 1", "18446744073709551616", "", null]) {
    expect([value, kind(() => parseDecimal(value, "seq"))]).toEqual([value, "invalid_decimal"]);
  }
  expect(kind(() => decimal(2 ** 53 + 2))).toBe("invalid_decimal");
  expect(kind(() => decimal(U64_MAX + 1n))).toBe("invalid_decimal");
  expect(kind(() => decimal(-1n))).toBe("invalid_decimal");
});
