import type { Decimal } from "./brand";
import { ClientError } from "./errors";

const CANONICAL = /^(0|[1-9][0-9]{0,19})$/;
export const U64_MAX = (1n << 64n) - 1n;

/** Parses a canonical u64 decimal string exactly; a JSON number or any other form is refused. */
export function parseDecimal<Kind extends string>(value: unknown, path: string): Decimal<Kind> {
  if (typeof value !== "string" || !CANONICAL.test(value) || BigInt(value) > U64_MAX) {
    throw new ClientError("invalid_decimal", `${path} is not a canonical u64 decimal string`);
  }
  return value as Decimal<Kind>;
}

/** The exact value of a decimal string; never rounded through a double. */
export function toBigInt(value: Decimal<string>): bigint {
  return BigInt(value);
}

/** Formats a u64 as its canonical decimal string. */
export function decimal<Kind extends string>(value: bigint | number): Decimal<Kind> {
  const exact = typeof value === "number" ? numberToBigInt(value) : value;
  if (exact < 0n || exact > U64_MAX) {
    throw new ClientError("invalid_decimal", `${exact} is outside the u64 range`);
  }
  return exact.toString() as Decimal<Kind>;
}

function numberToBigInt(value: number): bigint {
  if (!Number.isSafeInteger(value)) {
    throw new ClientError("invalid_decimal", `${value} is not an exactly representable integer`);
  }
  return BigInt(value);
}
