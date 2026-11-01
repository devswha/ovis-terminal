# Graphics passthrough: caps and the no-decode contract

Plan todo 86. Closes GAP-154, delivers IDEAL-121. Todos 83 and 84 build the
passthrough on top of what this document fixes; they enforce these caps and
publish these notices.

## Why

Agents and tools draw images inline (Kitty graphics, iTerm2 inline images,
sixel), and Ovis renders them with `@xterm/addon-image`. herdr 0.9.5 attach
drops graphics, while herdr-core passes them byte-identical live and on
reattach. ovis-terminal must pass them through too, without letting one chunk
series exhaust memory and without becoming an image parser.

## The no-decode contract

1. The daemon never decodes, validates, transforms, rescales or re-encodes an
   image payload. It does not inspect PNG, RGB, sixel raster data or base64
   content, and it adds no image-decoding dependency.
2. Format detection is header-only. A sequence is classified by its
   introducer alone:

   | Protocol (`GraphicsProtocol`) | Sequence | One "sequence" for the caps |
   |---|---|---|
   | `kitty` | `ESC _ G <keys> ; <payload> ESC \` | the whole chunk series of one transmission: every `m=1` chunk through the final `m=0` chunk |
   | `iterm2` | `ESC ] 1337 ; File= <args> : <payload> BEL` or `... ESC \` | one OSC |
   | `sixel` | `ESC P <params> q <data> ESC \` | one DCS |

   Control keys such as Kitty's `a=`, `f=`, `i=`/`I=` and `m=` may be read
   from the header to find sequence boundaries and ids. The payload is
   measured, never interpreted.
3. A sequence is kept or dropped whole, by its raw byte count. A viewer
   receives either the exact bytes the program wrote or none of them, never a
   prefix.
4. Viewers render. The VT engine (`crates/ovis-terminal-vt`,
   `alacritty_terminal`) implements only the Kitty keyboard protocol
   (`engine.rs` sets `kitty_keyboard: true`; `engine/modes.rs` keeps
   `kitty_flags`) and keeps no image state, so screen snapshots and
   checkpoints carry no images. Graphics reach viewers from the raw output,
   live and on reattach, and from nowhere else.

## Caps

Both caps are runtime `Limits` (`crates/ovis-terminal-protocol/src/schema/limits.rs`,
`Limits::v1`). `hello` advertises them under `limits` as canonical decimal
strings, like every other 64-bit count. A JSON number is refused as
`invalid_params`, and `u64::MAX` round-trips.

| Limit | Default | Bytes on the wire | Bounds | Over the bound |
|---|---|---|---|---|
| `graphics_sequence_max_bytes` | 8 MiB | `"8388608"` | raw bytes of one sequence (see the table above), introducer and terminator included | the sequence is dropped whole; `graphics.dropped{reason: too_large}` |
| `graphics_terminal_store_max_bytes` | 32 MiB | `"33554432"` | raw graphics bytes one terminal retains for reattach replay | the oldest sequences are evicted first until the new one fits; `graphics.evicted{reason: store_full}` |

The sequence cap is inclusive: a sequence of exactly 8 MiB is admitted, and
one more byte is dropped (`GraphicsCaps::admit`).

## Notices

Both are events in the `x-events` catalog (`schema/event_catalog.rs`). Their
data types are in `crates/ovis-terminal-protocol/src/domain/graphics.rs`.
Every reason is an open enum, so a client keeps an unknown future reason
verbatim.

| Event type | Notice | Data | Fields |
|---|---|---|---|
| `graphics.dropped` | `graphics_dropped{too_large}` | `GraphicsDroppedData` | `protocol`, `reason` (`too_large`), `output_offset`, `sequence_bytes`, `limit_bytes` |
| `graphics.evicted` | `graphics_evicted{store_full}` | `GraphicsEvictedData` | `reason` (`store_full`), `evicted_sequences`, `evicted_bytes`, `retained_bytes`, `limit_bytes` |

A notice carries counts and offsets, never image bytes, so it fits every
event and receipt bound.

## Layer budget: every advertised bound must be reachable

Todo 37's lesson: a limit is advertised only if every layer that carries the
value can hold it. Image bytes travel by their own path, never through the
ops ledger or the document store.

| Layer | Its bound | Holds an 8 MiB sequence? | Rule |
|---|---|---|---|
| Ops ledger receipt | 16 KiB (standing decision) | no | Nothing image-sized is ever recorded as an operation receipt. Only the notices above, which are a few hundred bytes, are committed as events. |
| Store value (`StoreLimits::max_value_bytes`) | 1 MiB | no | Images are never a document or store value. The image store keeps its own entries, each sized to the sequence cap, and their sum is bounded by `graphics_terminal_store_max_bytes`. |
| Launch record (`terminals/records.rs` `Launch::MAX_ENCODED_BYTES`) | 8 KiB | no | Launch and terminal records carry no image data. |
| Raw output ring (`output_ring_bytes`) | 64 MiB | yes | The raw replay source (`streams/journal.rs`, scheduler history). The 32 MiB store fits inside it, so a retained sequence never outlives its own replay bytes. |
| Frame (`max_frame_bytes`) | 16 MiB | yes | Even plain-mode base64 of a whole sequence, 11,184,812 bytes, fits one frame. Tested in `every_layer_that_carries_a_sequence_can_hold_the_cap`. |
| Session retained budget (`session_retained_bytes`) | 4 GiB | yes | The per-terminal store is charged to it. |
| Escape-boundary hold (`streams/filter.rs` `HOLD_LIMIT`) | 4096 bytes | no, today | A held sequence longer than 4096 bytes is released unfiltered. Todo 83 must reuse the boundary tracker but hold graphics sequences up to `graphics_sequence_max_bytes`, so a sequence is never split across frames or released in part. |
| Per-stream backlog (`stream_backlog_bytes`) | 4 MiB | no, today | A whole sequence released at once into a live stream is backlog. Above 4 MiB the stream gaps and falls back to a checkpoint, which carries no images. Todo 83 must make the full sequence cap reachable on a live attach before it enforces it: either deliver an admitted sequence without charging it against the backlog bound as one burst, or raise `stream_backlog_bytes` to at least the sequence cap plus one chunk. That todo owns the choice and its test. |

## What todo 86 does not do

It adds no stream, VT or store code. Detection, holding, dropping, the image
store and replay on reattach belong to todos 83 and 84, which consume
`GraphicsCaps::advertised(&limits)`, `GraphicsCaps::admit` and
`GraphicsEvictedData::store_full`.
