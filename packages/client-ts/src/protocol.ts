// The `@ovis-terminal/client/protocol` entry: generated protocol types and catalogs only,
// with no connection code, for consumers that need the contract without the Bun client.

export type { Decimal, Id, Open, UnknownValue } from "./brand";
export { asId, isKnown } from "./brand";
export * from "./generated/components";
export { errorKinds, type KnownErrorKind } from "./generated/errors";
export { eventsNames, type Events } from "./generated/events";
export { methods, type Methods, type ParamsOf, type ResultOf, type RpcMethod } from "./generated/methods";
export { notificationsNames, type Notifications } from "./generated/notifications";
export { compatibility, limitDefaults, protocolGeneration, schemaVersion, streamContract } from "./generated/stream";
