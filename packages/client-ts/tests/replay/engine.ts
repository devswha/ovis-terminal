// The Bun half of the transport-neutral replay (conformance/replay is the Rust half):
// symbol binding, pointer expectations, legacy translation and normalized receipts.
// Both halves read the same fixture files and must produce the same receipts.

export type Json = null | boolean | number | string | Json[] | { [key: string]: Json };

export interface Call {
  readonly conn: string;
  readonly id: string;
  readonly method: string;
  readonly params: Json;
  readonly inject?: boolean;
  readonly expect: Record<string, Json>;
}

export interface EventStep {
  readonly conn: string;
  readonly method: string;
  readonly expect: Record<string, Json>;
}

export type Step = { readonly call: Call } | { readonly legacy: Call } | { readonly event: EventStep };

export interface Case {
  readonly format: string;
  readonly id: string;
  readonly server: "assembled" | "protocol";
  readonly kind: "happy" | "failure";
  readonly steps: readonly Step[];
}

export interface Translation {
  readonly method: string;
  readonly into: string;
  readonly constant: Record<string, Json>;
  readonly fields: Record<string, { readonly to: string; readonly values?: Record<string, string> }>;
}

export interface ReceiptLine {
  readonly step: string;
  readonly method: string;
  readonly outcome: "result" | "error" | "event";
  readonly kind: string | null;
  readonly field_path: string | null;
}

export interface Check {
  readonly check: string;
  readonly expected: Json;
  readonly actual: Json;
  readonly passed: boolean;
}

/** `$op:` labels become fresh UUIDv7s, `$value:` captured values, `$bind:` server values. */
export class Bindings {
  private readonly ops = new Map<string, string>();
  private readonly values = new Map<string, Json>();

  constructor(private readonly server: Record<string, Json>) {}

  capture(name: string, value: Json): void {
    this.values.set(name, value);
  }

  resolve(value: Json): Json {
    if (typeof value === "string") return this.symbol(value);
    if (Array.isArray(value)) return value.map((item) => this.resolve(item));
    if (value !== null && typeof value === "object") {
      return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, this.resolve(item)]));
    }
    return value;
  }

  private symbol(text: string): Json {
    if (text.startsWith("$op:")) {
      const label = text.slice(4);
      const existing = this.ops.get(label);
      if (existing !== undefined) return existing;
      const fresh = Bun.randomUUIDv7();
      this.ops.set(label, fresh);
      return fresh;
    }
    for (const [prefix, table] of [["$value:", this.values], ["$bind:", new Map(Object.entries(this.server))]] as const) {
      if (!text.startsWith(prefix)) continue;
      const bound = table.get(text.slice(prefix.length));
      if (bound === undefined) throw new Error(`symbol_unbound: ${text}`);
      return bound;
    }
    return text;
  }
}

/** RFC 6901 lookup; `undefined` when absent. */
function pointer(value: Json, path: string): Json | undefined {
  if (path === "") return value;
  let current: Json | undefined = value;
  for (const raw of path.slice(1).split("/")) {
    const key = raw.replaceAll("~1", "/").replaceAll("~0", "~");
    if (Array.isArray(current)) current = current[Number(key)];
    else if (current !== null && typeof current === "object") current = Object.hasOwn(current, key) ? current[key] : undefined;
    else return undefined;
  }
  return current;
}

/** Structural equality that ignores object key order, as Rust's `Value` comparison does. */
export function same(left: Json | undefined, right: Json | undefined): boolean {
  if (Array.isArray(left) && Array.isArray(right)) return left.length === right.length && left.every((item, at) => same(item, right[at]));
  if (left !== null && right !== null && typeof left === "object" && typeof right === "object" && !Array.isArray(left) && !Array.isArray(right)) {
    const keys = Object.keys(left);
    return keys.length === Object.keys(right).length && keys.every((key) => Object.hasOwn(right, key) && same(left[key], right[key]));
  }
  return left === right;
}

function operator(matcher: Json): [string, Json] | undefined {
  if (matcher === null || typeof matcher !== "object" || Array.isArray(matcher)) return undefined;
  const entries = Object.entries(matcher);
  const first = entries[0];
  return entries.length === 1 && first !== undefined && first[0].startsWith("$") ? first : undefined;
}

/** Every pointer of `expect` checked against `subject`; captures are recorded. */
export function check(expect: Record<string, Json>, scope: string, subject: Json, bindings: Bindings): Check[] {
  return Object.keys(expect)
    .sort()
    .map((path) => {
      const matcher = expect[path] as Json;
      const actual = pointer(subject, path);
      const op = operator(matcher);
      const list = Array.isArray(actual) ? actual : undefined;
      let expected: Json;
      let passed: boolean;
      if (op === undefined) {
        expected = bindings.resolve(matcher);
        passed = actual !== undefined && same(actual, expected);
      } else if (op[0] === "$present") {
        expected = matcher;
        passed = (actual !== undefined) === op[1];
      } else if (op[0] === "$capture") {
        expected = matcher;
        passed = actual !== undefined;
        if (actual !== undefined) bindings.capture(String(op[1]), actual);
      } else if (op[0] === "$contains" || op[0] === "$excludes") {
        const items = bindings.resolve(op[1]) as Json[];
        expected = { [op[0]]: items };
        passed = list !== undefined && items.every((item) => list.some((value) => same(value, item)) === (op[0] === "$contains"));
      } else {
        throw new Error(`unknown matcher ${op[0]}`);
      }
      return { check: `${scope}${path}`, expected, actual: actual ?? null, passed };
    });
}

/** The published request of a legacy call; unknown fields stay at the top level. */
export function translate(map: Record<string, Translation>, step: string, method: string, params: Json): [string, Json] {
  const translation = map[method];
  if (translation === undefined) throw new Error(`legacy_untranslated: ${method}`);
  if (params === null || typeof params !== "object" || Array.isArray(params)) throw new Error(`legacy_untranslated: ${step}`);
  const inner: Record<string, Json> = { ...translation.constant };
  const outer: Record<string, Json> = { op_id: `$op:${step}` };
  for (const [key, value] of Object.entries(params)) {
    const field = translation.fields[key];
    if (field === undefined) {
      outer[key] = value;
      continue;
    }
    const renamed = typeof value === "string" ? field.values?.[value] : undefined;
    inner[field.to] = renamed ?? value;
  }
  outer[translation.into] = inner;
  return [translation.method, outer];
}

function text(value: Json, path: string): string | null {
  const found = pointer(value, path);
  return typeof found === "string" ? found : null;
}

export function replyLine(step: string, method: string, reply: Json): ReceiptLine {
  const failed = pointer(reply, "/error") !== undefined;
  return { step, method, outcome: failed ? "error" : "result", kind: text(reply, "/error/data/kind"), field_path: text(reply, "/error/data/field_path") };
}

export function eventLine(method: string, record: Json): ReceiptLine {
  return { step: `event:${method}`, method, outcome: "event", kind: text(record, "/params/type"), field_path: null };
}
