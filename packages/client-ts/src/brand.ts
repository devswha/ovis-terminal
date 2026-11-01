// Nominal wrappers the generated types use: an identity string, a u64 carried as a
// canonical decimal string, and an output enum value this build does not know.

declare const identity: unique symbol;
declare const decimal: unique symbol;
declare const unknownValue: unique symbol;

/** An opaque identity string; two identity kinds are never interchangeable. */
export type Id<Kind extends string> = string & { readonly [identity]: Kind };

/** A u64 counter, offset or sequence as a canonical decimal string, never a JSON number. */
export type Decimal<Kind extends string> = string & { readonly [decimal]: Kind };

/** An output enum value a newer runtime added; the original string is kept unchanged. */
export type UnknownValue = string & { readonly [unknownValue]: true };

/** An open output enum: every known value, or an additive value kept as received. */
export type Open<Known extends string> = Known | UnknownValue;

/** Names an identity the caller already holds, such as a terminal id from configuration. */
export function asId<Brand extends Id<string>>(value: string): Brand {
  return value as Brand;
}

/** Whether `value` is one of `known`; an additive value answers false and stays intact. */
export function isKnown<Known extends string>(
  known: readonly Known[],
  value: string,
): value is Known {
  return (known as readonly string[]).includes(value);
}
