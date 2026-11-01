// Compiled with tsc against the installed declarations only. Each @ts-expect-error fails the
// compile if the packed types stop rejecting that misuse.

import { type Client, type ParamsOf, decimal } from "@ovis-terminal/client";
import type { OpRecord, StreamId, TerminalId } from "@ovis-terminal/client/protocol";
import document from "@ovis-terminal/client/openrpc.json";

export const offset: ParamsOf<"terminal.output">["from_offset"] = decimal(5n);

// @ts-expect-error a u64 offset is a canonical decimal string, never a JSON number
export const numeric: ParamsOf<"terminal.output">["from_offset"] = 5;

declare const terminal: TerminalId;
// @ts-expect-error identities of different kinds are not interchangeable
export const crossed: StreamId = terminal;

export const known: OpRecord["state"] = "committed";

// @ts-expect-error an additive output value is an explicit UnknownValue, not any string
export const guessed: OpRecord["state"] = "future_status";

export async function unknownMethod(client: Client): Promise<void> {
  // @ts-expect-error only published methods can be called through the typed path
  await client.call("terminal.focus_pane", {});
}

export const version: string = document.info.version;
