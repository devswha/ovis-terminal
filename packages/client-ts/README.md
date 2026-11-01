# @ovis-terminal/client

TypeScript types generated from the ovis-terminal OpenRPC document and a persistent Bun client.
It requires Bun >=1.4.3; the declarations reference `bun-types`.

## Entry points

| Import | Contents |
|---|---|
| `@ovis-terminal/client` | `Client` (one persistent connection: correlated calls, cancellation, deadlines), `Subscription` (cursor replay), `Streams` (typed attach outcomes, per-stream credit), drafts, wire codecs and the protocol types |
| `@ovis-terminal/client/protocol` | generated protocol types and catalogs only (methods, events, notifications, error kinds, limits, stream contract) |
| `@ovis-terminal/client/openrpc.json` | the OpenRPC document the types were generated from; `info.version` and `info["x-generation"]` version it |

## Delivery contract

- A mutation whose reply is lost, or whose client deadline passes, rejects with `OutcomeUnknown`.
  Its `draft` is retained and nothing is resent until the caller calls `draft.retry()` (same
  `op_id`) or `draft.discard()`.
- Server refusals are `RpcError` with the server's `data.kind`, hint, retryability and `op_id`.
- Output enum values added by a newer runtime of the same generation are kept as received.
- A malformed frame from the server ends the connection once; `client.failure` holds the
  typed decoder error.
- Unix sockets only: on Windows, `Client.connect` refuses with `unsupported_transport`.

## Building

```sh
bun run build   # checks src/generated against schema/, writes dist/
bun run pack    # writes pack/ovis-terminal-client-<version>.tgz and prints its sha256
```

The package is marked `private`: packing it does not publish it.
