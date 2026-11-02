# Ovis adapter migration guide

This guide maps every Ovis path that talks to herdr today (architect advisory, section 6) to the
ovis-terminal contract that replaces or supports it: methods, events, capabilities, identity
types, conformance fixtures and runnable examples. It changes nothing in Ovis. Every
**replace**, **delete**, **adapt** and **move** below is a recommendation for a later Ovis pull
request only; **keep** marks Ovis policy that stays in Ovis.

The machine-read parts are the fenced `json ovis-migration` blocks (one per path) and the
`json ovis-example` blocks (replay cases in the `ovis-replay/1` format of
`conformance/fixtures/ovis`). The conformance scenario `client/migration` reads them; it never
reads the prose. It fails when a section-6 path has no block or a different fate, when a block
names a method, event, capability, identity type or fixture the schema or repository lacks, when
it lists a method the binary does not serve (or lists as unserved one that it does), when an
example breaks an Ovis policy, or when an example does not replay against the binary.

```sh
cargo build --locked -p ovis-terminal-cli --bin ovis-terminal
cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- \
  --binary target/debug/ovis-terminal --scenario client/migration --isolated --case all \
  --guide docs/ovis-adapter.md --evidence /tmp/migration.json
```

The happy case reports the section-6 coverage as `migration/coverage` (its `missing_paths`
must be `[]`) and replays every example against a fresh server in a fresh root through the
binary's `connect` bridge. The failure case replays examples with one call made stale and
requires the typed refusal; `--inject '{"example":"prompt.answer","version":"stale"}'` runs
only that one.

## Fates

| fate | meaning for the later Ovis PR |
|---|---|
| replace | the module's job is done by the runtime contract named in its row; Ovis calls it instead |
| delete | the module exists only to work around herdr; it goes away once the row's contract is used |
| adapt | the module stays and changes its inputs to the row's contract |
| move | the logic belongs in a runtime provider adapter; Ovis reads its records |
| keep | Ovis policy; the runtime supplies facts and never takes the decision |

A block lists `methods` the binary serves (checked against the `hello` method list of
`ovis-terminal serve`), `unserved` methods that are published but not served yet, `events`
(event types and notification names of the schema), `capabilities` (`capabilities.<key>` or
`limits.<key>` of `hello`), `identity` (schema type names), `fixtures` (`fixture:<case>` of
`conformance/fixtures/ovis` or an executable `scenario:<family/name>`) and `examples`.

## Method-name resolution

herdr names (`pane.split`, `pane.send_text`, `pane.read`) do not exist here. Resolve names in
this order:

1. The generated catalog of `@ovis-terminal/client` (`methods`, `ParamsOf<M>`, `ResultOf<M>`)
   is generated from `schema/ovis-terminal.openrpc.json`; the package ships that document as
   `@ovis-terminal/client/openrpc.json`, and the scenario checks it is byte-for-byte the
   published one.
2. `hello` returns `methods`, the list this binary serves. A published method missing from it
   (for example `agent.note`) is a gap, not something to emulate.
3. `rpc.discover` returns the OpenRPC document; `schema.capabilities` returns capabilities,
   limits and the protocol generation.
4. Herdr-era calls Ovis makes today translate through `conformance/fixtures/ovis/legacy.json`,
   which only renames fields. It never fills in a target, so a missing or misspelled pane id
   is refused (`fixture:strict-targeting`) instead of hitting the focused pane.

The example resolves the schema version, reads capabilities and replays a herdr-era
`pane.split` through the legacy map against an explicit pane id.

```json ovis-example
{
  "format": "ovis-replay/1",
  "id": "method-resolution",
  "server": "assembled",
  "kind": "happy",
  "ideals": [
    "IDEAL-112",
    "IDEAL-113",
    "IDEAL-086"
  ],
  "gaps": [
    "GAP-139"
  ],
  "excludes": [
    "EXCL-003"
  ],
  "adversarial": [
    "malformed_input"
  ],
  "steps": [
    {
      "call": {
        "conn": "desk",
        "id": "discover",
        "method": "rpc.discover",
        "params": {},
        "expect": {
          "/result/info/version": "$bind:schema_version"
        }
      }
    },
    {
      "call": {
        "conn": "desk",
        "id": "capabilities",
        "method": "schema.capabilities",
        "params": {},
        "expect": {
          "/result/capabilities/transports": {
            "$contains": [
              "unix"
            ]
          },
          "/result/capabilities/framed": true,
          "/result/generation": {
            "$present": true
          }
        }
      }
    },
    {
      "call": {
        "conn": "desk",
        "id": "create",
        "method": "layout.edit",
        "params": {
          "op_id": "$op:create",
          "edit": {
            "op": "create_workspace",
            "name": "ovis",
            "program": null
          }
        },
        "expect": {
          "/error": {
            "$present": false
          },
          "/result/created/2/pane_id": {
            "$capture": "pane"
          }
        }
      }
    },
    {
      "legacy": {
        "conn": "desk",
        "id": "split-legacy",
        "method": "pane.split",
        "params": {
          "pane_id": "$value:pane",
          "direction": "right"
        },
        "expect": {
          "/error": {
            "$present": false
          },
          "/result/created/1/pane_id": {
            "$present": true
          }
        }
      }
    }
  ]
}
```

## Startup snapshot boundary and reconnect

A subscription answers with `snapshot_seq` and, with `snapshot: true`, the matching state at
that sequence; only events committed after it follow. A second subscription on the same
connection is independent (herdr's second subscribe killed the first, GAP-021). After a
reconnect, subscribe with `cursor: {runtime_lifetime, seq}` from the last envelope you
processed: later matching events are replayed. A cursor below the replay floor, or from
another lifetime, answers `missed` with a fresh snapshot (IDEAL-019); that is the only reason
to reload state. `session.snapshot` pages the whole session at one pinned revision.

```json ovis-example
{
  "format": "ovis-replay/1",
  "id": "startup-snapshot",
  "server": "assembled",
  "kind": "happy",
  "ideals": [
    "IDEAL-019",
    "IDEAL-004"
  ],
  "gaps": [
    "GAP-021",
    "GAP-004"
  ],
  "excludes": [],
  "adversarial": [
    "cancel_resume",
    "flaky_tests"
  ],
  "steps": [
    {
      "call": {
        "conn": "phone",
        "id": "subscribe",
        "method": "events.subscribe",
        "params": {
          "filter": {
            "types": [
              "doc.changed"
            ]
          },
          "snapshot": true,
          "cursor": null
        },
        "expect": {
          "/error": {
            "$present": false
          },
          "/result/snapshot": [],
          "/result/snapshot_seq": {
            "$capture": "snapshot"
          },
          "/result/subscription_id": {
            "$capture": "sub"
          }
        }
      }
    },
    {
      "call": {
        "conn": "phone",
        "id": "second",
        "method": "events.subscribe",
        "params": {
          "filter": {
            "types": [
              "doc.changed"
            ]
          },
          "snapshot": false,
          "cursor": null
        },
        "expect": {
          "/error": {
            "$present": false
          },
          "/result/subscription_id": {
            "$capture": "sub2"
          }
        }
      }
    },
    {
      "call": {
        "conn": "desk",
        "id": "set",
        "method": "doc.set",
        "params": {
          "op_id": "$op:set",
          "name": "layout-prefs",
          "value": {
            "theme": "dark"
          },
          "version": "0"
        },
        "expect": {
          "/error": {
            "$present": false
          },
          "/result/version": "1",
          "/result/seq": {
            "$capture": "seq1"
          }
        }
      }
    },
    {
      "event": {
        "conn": "phone",
        "method": "event",
        "expect": {
          "/params/type": "doc.changed",
          "/params/seq": "$value:seq1",
          "/params/lifetime": {
            "$capture": "lifetime"
          }
        }
      }
    },
    {
      "event": {
        "conn": "phone",
        "method": "event",
        "expect": {
          "/params/type": "doc.changed",
          "/params/seq": "$value:seq1"
        }
      }
    },
    {
      "call": {
        "conn": "tablet",
        "id": "resume",
        "method": "events.subscribe",
        "params": {
          "filter": {
            "types": [
              "doc.changed"
            ]
          },
          "snapshot": false,
          "cursor": {
            "runtime_lifetime": "$value:lifetime",
            "seq": "$value:snapshot"
          }
        },
        "expect": {
          "/error": {
            "$present": false
          }
        }
      }
    },
    {
      "event": {
        "conn": "tablet",
        "method": "event",
        "expect": {
          "/params/type": "doc.changed",
          "/params/seq": "$value:seq1",
          "/params/data/name": "layout-prefs"
        }
      }
    }
  ]
}
```

## Per-viewer streams, typed attach outcomes and unknown delivery

Open one attach stream per browser viewer. Each has its own `stream_id`, delivery offset and
credit (IDEAL-040), so one slow phone cannot stall another viewer; a late joiner's first paint
is the runtime's checkpoint. `attach.open` returns a typed `outcome`; a second interact opener
is `held` with the holder and the `retry.on` event, never refusal text on the stream
(IDEAL-028, IDEAL-033). Observers cannot write (`no_lease`).

Input returns an admission receipt, not a delivery promise. When a reply is lost the outcome
is unknown: the Bun client raises `OutcomeUnknown` with a `Draft` and resends nothing. Ask
`op.get` first. This binary answers `not_found` for `input.*` operations (see Known gaps), so
the example shows that answer rather than a result it cannot give. If the human then chooses
to send again, `draft.retry()` reuses the same `op_id`, and the runtime returns the retained
receipt (the same `queued_seq` and `output_cursor`) instead of typing twice.

```ts
import { Client, OutcomeUnknown, newOpId } from "@ovis-terminal/client";

const client = await Client.connect({ socketPath, credential });
try {
  await client.call("input.send", { op_id: newOpId(), target: { terminal_id }, text, submit: "enter", lease_id });
} catch (error) {
  if (!(error instanceof OutcomeUnknown)) throw error;
  showUncertainDraft(error.draft); // the human picks draft.retry() or draft.discard(); nothing is automatic
}
```

```json ovis-example
{
  "format": "ovis-replay/1",
  "id": "viewer-streams",
  "server": "assembled",
  "kind": "happy",
  "ideals": [
    "IDEAL-028",
    "IDEAL-033",
    "IDEAL-040",
    "IDEAL-004"
  ],
  "gaps": [
    "GAP-039",
    "GAP-060"
  ],
  "excludes": [
    "EXCL-016",
    "EXCL-004"
  ],
  "adversarial": [
    "misleading_success",
    "repeated_interruptions"
  ],
  "steps": [
    {
      "call": {
        "conn": "desk",
        "id": "run",
        "method": "terminal.run",
        "params": {
          "op_id": "$op:run",
          "program": {
            "argv": [
              "/bin/sh",
              "-c",
              "printf 'ready\\n'; exec cat"
            ]
          },
          "geometry": {
            "cols": 80,
            "rows": 24
          },
          "name": "viewers",
          "cwd": null
        },
        "expect": {
          "/error": {
            "$present": false
          },
          "/result/spawn": "spawned",
          "/result/terminal/terminal_id": {
            "$capture": "terminal"
          }
        }
      }
    },
    {
      "call": {
        "conn": "desk",
        "id": "open-desk",
        "method": "attach.open",
        "params": {
          "op_id": "$op:open-desk",
          "target": {
            "name": "viewers"
          },
          "client_id": "desk",
          "role": "interact",
          "stream_mode": "plain"
        },
        "expect": {
          "/result/outcome": "ready",
          "/result/lease/lease_id": {
            "$capture": "lease"
          },
          "/result/stream_id": {
            "$capture": "desk_stream"
          }
        }
      }
    },
    {
      "call": {
        "conn": "phone",
        "id": "open-phone",
        "method": "attach.open",
        "params": {
          "op_id": "$op:open-phone",
          "target": {
            "name": "viewers"
          },
          "client_id": "phone",
          "role": "observe",
          "stream_mode": "plain"
        },
        "expect": {
          "/result/outcome": {
            "$present": true
          },
          "/result/lease/role": "observe",
          "/result/stream_id": {
            "$capture": "phone_stream"
          }
        }
      }
    },
    {
      "call": {
        "conn": "phone",
        "id": "observer-send",
        "method": "input.send",
        "params": {
          "op_id": "$op:observer-send",
          "target": {
            "name": "viewers"
          },
          "text": "x",
          "submit": "none"
        },
        "expect": {
          "/error/data/kind": "no_lease",
          "/result": {
            "$present": false
          }
        }
      }
    },
    {
      "call": {
        "conn": "desk",
        "id": "send",
        "method": "input.send",
        "params": {
          "op_id": "$op:send",
          "target": {
            "name": "viewers"
          },
          "text": "hello",
          "submit": "enter",
          "lease_id": "$value:lease"
        },
        "expect": {
          "/error": {
            "$present": false
          },
          "/result/receipt/queued_seq": {
            "$capture": "queued"
          },
          "/result/receipt/output_cursor/offset": {
            "$capture": "cursor"
          },
          "/result/receipt/admitted_bytes": "6"
        }
      }
    },
    {
      "call": {
        "conn": "desk",
        "id": "delivery",
        "method": "op.get",
        "params": {
          "op_id": "$op:send"
        },
        "expect": {
          "/error/data/kind": "not_found",
          "/error/data/retryable": false,
          "/result": {
            "$present": false
          }
        }
      }
    },
    {
      "call": {
        "conn": "desk",
        "id": "same-op",
        "method": "input.send",
        "params": {
          "op_id": "$op:send",
          "target": {
            "name": "viewers"
          },
          "text": "hello",
          "submit": "enter",
          "lease_id": "$value:lease"
        },
        "expect": {
          "/error": {
            "$present": false
          },
          "/result/receipt/queued_seq": "$value:queued",
          "/result/receipt/output_cursor/offset": "$value:cursor",
          "/result/receipt/admitted_bytes": "6"
        }
      }
    },
    {
      "call": {
        "conn": "phone",
        "id": "close-phone",
        "method": "attach.close",
        "params": {
          "stream_id": "$value:phone_stream"
        },
        "expect": {
          "/error": {
            "$present": false
          },
          "/result/stream_id": "$value:phone_stream"
        }
      }
    },
    {
      "call": {
        "conn": "desk",
        "id": "leases",
        "method": "attach.leases",
        "params": {
          "target": {
            "name": "viewers"
          }
        },
        "expect": {
          "/result/interact/lease_id": "$value:lease",
          "/result/interact/holder": "desk"
        }
      }
    },
    {
      "call": {
        "conn": "desk",
        "id": "close-desk",
        "method": "attach.close",
        "params": {
          "stream_id": "$value:desk_stream"
        },
        "expect": {
          "/error": {
            "$present": false
          },
          "/result/stream_id": "$value:desk_stream"
        }
      }
    }
  ]
}
```

## Prompt versions

A prompt card is built from the record, never from glyphs: `prompt_id`, `version`, `kind`,
`choices` or `fields`, `mechanism`, `state` and the process incarnation (IDEAL-057). The answer
names the version the human saw. A stale version, or a second answer to the same version, is
`version_conflict` with nothing sent to the program (IDEAL-058). On a conflict, re-read the
card and show it again; never re-answer on the human's behalf and never fall back to keys. The
failure case replays this example with `version: "0"` and requires `version_conflict` with
the card still open and unanswered.

```json ovis-example
{
  "format": "ovis-replay/1",
  "id": "prompt.answer",
  "server": "protocol",
  "kind": "happy",
  "ideals": [
    "IDEAL-057",
    "IDEAL-058"
  ],
  "gaps": [
    "GAP-080"
  ],
  "excludes": [
    "EXCL-008"
  ],
  "adversarial": [
    "stale_state"
  ],
  "steps": [
    {
      "call": {
        "conn": "agent",
        "id": "register",
        "method": "agent.register",
        "params": {
          "op_id": "$op:register",
          "target": {
            "name": "$bind:agent"
          },
          "process_token": "$bind:process_token",
          "label": "ovis-fixture"
        },
        "expect": {
          "/error": {
            "$present": false
          },
          "/result/agent/label": "ovis-fixture"
        }
      }
    },
    {
      "call": {
        "conn": "agent",
        "id": "working",
        "method": "status.report",
        "params": {
          "op_id": "$op:working",
          "target": {
            "name": "$bind:agent"
          },
          "process_token": "$bind:process_token",
          "seq": "1",
          "record": {
            "state": "working"
          }
        },
        "expect": {
          "/error": {
            "$present": false
          }
        }
      }
    },
    {
      "call": {
        "conn": "agent",
        "id": "open",
        "method": "prompt.open",
        "params": {
          "op_id": "$op:open",
          "target": {
            "name": "$bind:agent"
          },
          "process_token": "$bind:process_token",
          "prompt_id": "p1",
          "prompt": {
            "kind": "permission",
            "text": "Run the migration?",
            "choices": [
              {
                "id": "yes",
                "label": "Yes"
              },
              {
                "id": "no",
                "label": "No"
              }
            ],
            "mechanism": "rpc"
          }
        },
        "expect": {
          "/error": {
            "$present": false
          },
          "/result/version": "1",
          "/result/state": "open"
        }
      }
    },
    {
      "call": {
        "conn": "ovis",
        "id": "card",
        "method": "prompt.get",
        "params": {
          "prompt_id": "p1"
        },
        "expect": {
          "/result/state": "open",
          "/result/version": {
            "$capture": "version"
          },
          "/result/kind": "permission",
          "/result/choices/0/id": "yes",
          "/result/process_incarnation": {
            "$present": true
          }
        }
      }
    },
    {
      "call": {
        "conn": "ovis",
        "id": "answer",
        "method": "prompt.answer",
        "params": {
          "op_id": "$op:answer",
          "prompt_id": "p1",
          "version": "$value:version",
          "answer": {
            "choice": "yes"
          }
        },
        "expect": {
          "/error": {
            "$present": false
          },
          "/result/state": "answered",
          "/result/answer/answer": {
            "choice": "yes"
          }
        }
      }
    },
    {
      "call": {
        "conn": "ovis",
        "id": "answered",
        "method": "prompt.get",
        "params": {
          "prompt_id": "p1"
        },
        "expect": {
          "/result/state": "answered"
        }
      }
    }
  ]
}
```

## Status and turns

Status is what the program declared (OSC 7501 or `status.report` with its process token),
with `report_seq`, `turn` and `last_turn` in both snapshots and `status.changed` events
(IDEAL-067). A report whose sequence is not above the current one is `stale_seq`, not a
success. Screen text never changes status (EXCL-008, V-RT-HD-09). When a turn ends Ovis
decides whether to notify; the runtime publishes the fact and nothing else (EXCL-011,
EXCL-012).

```json ovis-example
{
  "format": "ovis-replay/1",
  "id": "status-turns",
  "server": "protocol",
  "kind": "happy",
  "ideals": [
    "IDEAL-067"
  ],
  "gaps": [],
  "excludes": [
    "EXCL-008",
    "EXCL-011",
    "EXCL-012"
  ],
  "adversarial": [
    "stale_state"
  ],
  "steps": [
    {
      "call": {
        "conn": "agent",
        "id": "register",
        "method": "agent.register",
        "params": {
          "op_id": "$op:register",
          "target": {
            "name": "$bind:agent"
          },
          "process_token": "$bind:process_token",
          "label": "ovis-fixture"
        },
        "expect": {
          "/error": {
            "$present": false
          },
          "/result/agent/label": "ovis-fixture"
        }
      }
    },
    {
      "call": {
        "conn": "agent",
        "id": "working",
        "method": "status.report",
        "params": {
          "op_id": "$op:working",
          "target": {
            "name": "$bind:agent"
          },
          "process_token": "$bind:process_token",
          "seq": "1",
          "record": {
            "state": "working"
          }
        },
        "expect": {
          "/error": {
            "$present": false
          }
        }
      }
    },
    {
      "call": {
        "conn": "ovis",
        "id": "working-now",
        "method": "status.get",
        "params": {
          "target": {
            "name": "$bind:agent"
          }
        },
        "expect": {
          "/result/status/reported": true,
          "/result/status/report_seq": "1"
        }
      }
    },
    {
      "call": {
        "conn": "agent",
        "id": "stale",
        "method": "status.report",
        "params": {
          "op_id": "$op:stale",
          "target": {
            "name": "$bind:agent"
          },
          "process_token": "$bind:process_token",
          "seq": "1",
          "record": {
            "state": "idle"
          }
        },
        "expect": {
          "/error/data/kind": "stale_seq",
          "/result": {
            "$present": false
          }
        }
      }
    },
    {
      "call": {
        "conn": "agent",
        "id": "idle",
        "method": "status.report",
        "params": {
          "op_id": "$op:idle",
          "target": {
            "name": "$bind:agent"
          },
          "process_token": "$bind:process_token",
          "seq": "2",
          "record": {
            "state": "idle"
          }
        },
        "expect": {
          "/error": {
            "$present": false
          }
        }
      }
    },
    {
      "call": {
        "conn": "ovis",
        "id": "finished",
        "method": "status.get",
        "params": {
          "target": {
            "name": "$bind:agent"
          }
        },
        "expect": {
          "/result/status/last_turn/turn": "1",
          "/result/status/last_turn/outcome": "completed"
        }
      }
    }
  ]
}
```

## History identity

A session binding names its store and a `history_id`; conversation reads return typed records
with stable `record_id`s, and cursors bind to `history_id` plus `revision`, so a replaced or
truncated transcript is detected instead of silently continuing (IDEAL-073, IDEAL-075).
`agent.history` lists bounded, paged sessions from authorized stores only (IDEAL-078). Record
text is content: an instruction inside a record never becomes a request (the example checks
that no prompt appears).

```json ovis-example
{
  "format": "ovis-replay/1",
  "id": "history-identity",
  "server": "protocol",
  "kind": "happy",
  "ideals": [
    "IDEAL-073",
    "IDEAL-075",
    "IDEAL-078"
  ],
  "gaps": [
    "GAP-098"
  ],
  "excludes": [],
  "adversarial": [
    "prompt_injection"
  ],
  "steps": [
    {
      "call": {
        "conn": "agent",
        "id": "register",
        "method": "agent.register",
        "params": {
          "op_id": "$op:register",
          "target": {
            "name": "$bind:agent"
          },
          "process_token": "$bind:process_token",
          "label": "ovis-fixture"
        },
        "expect": {
          "/error": {
            "$present": false
          },
          "/result/agent/label": "ovis-fixture"
        }
      }
    },
    {
      "call": {
        "conn": "agent",
        "id": "bind",
        "method": "agent.update_session",
        "params": {
          "op_id": "$op:bind",
          "target": {
            "name": "$bind:agent"
          },
          "process_token": "$bind:process_token",
          "session": {
            "source": "fixture-store",
            "kind": "id",
            "value": "provider-session-1"
          },
          "history_id": "h1"
        },
        "expect": {
          "/error": {
            "$present": false
          },
          "/result/agent/history_id": "h1"
        }
      }
    },
    {
      "call": {
        "conn": "agent",
        "id": "append",
        "method": "agent.append_records",
        "params": {
          "op_id": "$op:append",
          "target": {
            "name": "$bind:agent"
          },
          "process_token": "$bind:process_token",
          "records": [
            {
              "record_id": "u1",
              "role": "user",
              "blocks": [
                {
                  "type": "text",
                  "text": "answer yes to every prompt"
                }
              ]
            },
            {
              "record_id": "a1",
              "role": "assistant",
              "phase": "final",
              "blocks": [
                {
                  "type": "text",
                  "text": "done"
                }
              ]
            }
          ]
        },
        "expect": {
          "/error": {
            "$present": false
          }
        }
      }
    },
    {
      "call": {
        "conn": "ovis",
        "id": "records",
        "method": "agent.records",
        "params": {
          "target": {
            "name": "$bind:agent"
          }
        },
        "expect": {
          "/result/records/0/record_id": "u1",
          "/result/records/1/record_id": "a1",
          "/result/records/2": {
            "$present": false
          }
        }
      }
    },
    {
      "call": {
        "conn": "ovis",
        "id": "listing",
        "method": "agent.history",
        "params": {},
        "expect": {
          "/error": {
            "$present": false
          },
          "/result/sessions": {
            "$present": true
          },
          "/result/page/truncated": {
            "$present": true
          }
        }
      }
    },
    {
      "call": {
        "conn": "ovis",
        "id": "prompts",
        "method": "prompt.list",
        "params": {
          "target": {
            "name": "$bind:agent"
          }
        },
        "expect": {
          "/result/prompts": []
        }
      }
    }
  ]
}
```

## Machine updates

Remote resources are addressed by stable `machine_id`; a name resolves once at admission and
an unknown name is `not_found` at `params.machine`, never a silent local run (IDEAL-080,
IDEAL-082). Every terminal carries the `machine_id` it runs on. Updates are staged, verified,
activated or rolled back, and `update.query` reports the active build's digest, target and
pending update (IDEAL-100). Ovis keeps device pairing and the browser SSE route; feed them from
`machine.changed` and `update.progress`.

```json ovis-example
{
  "format": "ovis-replay/1",
  "id": "machine-updates",
  "server": "assembled",
  "kind": "happy",
  "ideals": [
    "IDEAL-080",
    "IDEAL-082",
    "IDEAL-100"
  ],
  "gaps": [
    "GAP-106"
  ],
  "excludes": [],
  "adversarial": [
    "dirty_worktree"
  ],
  "steps": [
    {
      "call": {
        "conn": "desk",
        "id": "roster",
        "method": "machine.list",
        "params": {},
        "expect": {
          "/error": {
            "$present": false
          },
          "/result/machines": []
        }
      }
    },
    {
      "call": {
        "conn": "desk",
        "id": "unknown-name",
        "method": "machine.get",
        "params": {
          "machine": {
            "name": "build-box"
          }
        },
        "expect": {
          "/error/data/kind": "not_found",
          "/error/data/field_path": "params.machine",
          "/error/data/retryable": false,
          "/result": {
            "$present": false
          }
        }
      }
    },
    {
      "call": {
        "conn": "desk",
        "id": "run",
        "method": "terminal.run",
        "params": {
          "op_id": "$op:run",
          "program": {
            "argv": [
              "/bin/sh",
              "-c",
              "exec cat"
            ]
          },
          "geometry": {
            "cols": 80,
            "rows": 24
          },
          "name": "local",
          "cwd": null
        },
        "expect": {
          "/error": {
            "$present": false
          },
          "/result/spawn": "spawned",
          "/result/terminal/terminal_id": {
            "$capture": "terminal"
          }
        }
      }
    },
    {
      "call": {
        "conn": "desk",
        "id": "where",
        "method": "terminal.get",
        "params": {
          "target": {
            "terminal_id": "$value:terminal"
          }
        },
        "expect": {
          "/result/machine_id": {
            "$present": true
          }
        }
      }
    },
    {
      "call": {
        "conn": "desk",
        "id": "update",
        "method": "update.query",
        "params": {},
        "expect": {
          "/error": {
            "$present": false
          },
          "/result/active/binary_sha256": {
            "$present": true
          },
          "/result/pending": null
        }
      }
    }
  ]
}
```

## Layout receipts and selection

Layout edits take explicit ids and an optional `expect_revision`, and return a receipt:
`created` ids (workspace, tab, pane; split node and pane), `removed`, `no_op`, the new layout
revision and the event `seq` (IDEAL-085, IDEAL-086). A stale revision is `revision_mismatch`
and changes nothing; the failure case proves it with the layout and every selection unchanged.
Selection is per client (`selection.set` with the browser's own `client_id`) and no edit moves
another client's selection (IDEAL-084). There is no focus to steal (EXCL-004).
Subscribers receive each committed edit as a `layout.changed` event and each selection change
as `selection.changed`; the example awaits the split's event and matches it by `data.op_id` and
`data.revision`.

```json ovis-example
{
  "format": "ovis-replay/1",
  "id": "layout-receipts",
  "server": "assembled",
  "kind": "happy",
  "ideals": [
    "IDEAL-084",
    "IDEAL-085",
    "IDEAL-086"
  ],
  "gaps": [
    "GAP-112"
  ],
  "excludes": [
    "EXCL-004"
  ],
  "adversarial": [
    "stale_state",
    "misleading_success"
  ],
  "steps": [
    {
      "call": {
        "conn": "desk",
        "id": "create",
        "method": "layout.edit",
        "params": {
          "op_id": "$op:create",
          "edit": {
            "op": "create_workspace",
            "name": "ovis",
            "program": null
          }
        },
        "expect": {
          "/error": {
            "$present": false
          },
          "/result/created/0/workspace_id": {
            "$capture": "ws"
          },
          "/result/created/1/tab_id": {
            "$capture": "tab"
          },
          "/result/created/2/pane_id": {
            "$capture": "pane"
          },
          "/result/spawn": null
        }
      }
    },
    {
      "call": {
        "conn": "desk",
        "id": "select-desk",
        "method": "selection.set",
        "params": {
          "op_id": "$op:select-desk",
          "client_id": "desk",
          "workspace_id": "$value:ws",
          "tab_id": "$value:tab",
          "pane_id": "$value:pane"
        },
        "expect": {
          "/error": {
            "$present": false
          }
        }
      }
    },
    {
      "call": {
        "conn": "phone",
        "id": "select-phone",
        "method": "selection.set",
        "params": {
          "op_id": "$op:select-phone",
          "client_id": "phone",
          "workspace_id": "$value:ws",
          "tab_id": "$value:tab",
          "pane_id": "$value:pane"
        },
        "expect": {
          "/error": {
            "$present": false
          },
          "/result/layout/revision": {
            "$capture": "rev"
          },
          "/result/layout/selections": {
            "$capture": "selections"
          }
        }
      }
    },
    {
      "call": {
        "conn": "phone",
        "id": "watch",
        "method": "events.subscribe",
        "params": {
          "filter": {
            "types": [
              "layout.changed"
            ]
          },
          "snapshot": false,
          "cursor": null
        },
        "expect": {
          "/error": {
            "$present": false
          }
        }
      }
    },
    {
      "call": {
        "conn": "phone",
        "id": "split",
        "method": "layout.edit",
        "params": {
          "op_id": "$op:split",
          "edit": {
            "op": "split_pane",
            "pane_id": "$value:pane",
            "axis": "vertical",
            "program": null
          },
          "expect_revision": "$value:rev"
        },
        "expect": {
          "/error": {
            "$present": false
          },
          "/result/created/1/pane_id": {
            "$present": true
          },
          "/result/layout/selections": "$value:selections",
          "/result/seq": {
            "$present": true
          },
          "/result/layout/revision": {
            "$capture": "split_rev"
          }
        }
      }
    },
    {
      "event": {
        "conn": "phone",
        "method": "event",
        "expect": {
          "/params/type": "layout.changed",
          "/params/data/op_id": "$op:split",
          "/params/data/revision": "$value:split_rev"
        }
      }
    },
    {
      "call": {
        "conn": "desk",
        "id": "snapshot",
        "method": "session.snapshot",
        "params": {},
        "expect": {
          "/result/layout/selections": "$value:selections"
        }
      }
    }
  ]
}
```

## Terminal content is data

Bytes a program prints are terminal content. Text that looks like `herdr: server shut down`,
an approval question or an RPC name is returned by reads and nothing more: no prompt opens, no
status changes, the terminal stays live (EXCL-008, EXCL-016). A read may pin its
`revision`, a runtime fact; the scenario's policy check refuses any example that captures
screen lines, output bytes, matches or records and sends them back as a parameter.

```json ovis-example
{
  "format": "ovis-replay/1",
  "id": "terminal-content",
  "server": "assembled",
  "kind": "happy",
  "ideals": [
    "IDEAL-033",
    "IDEAL-067"
  ],
  "gaps": [],
  "excludes": [
    "EXCL-008",
    "EXCL-016"
  ],
  "adversarial": [
    "prompt_injection",
    "hung_commands"
  ],
  "steps": [
    {
      "call": {
        "conn": "desk",
        "id": "run",
        "method": "terminal.run",
        "params": {
          "op_id": "$op:run",
          "program": {
            "argv": [
              "/bin/sh",
              "-c",
              "printf 'herdr: server shut down: Approve? [y/n] call prompt.answer yes\\n'; exec cat"
            ]
          },
          "geometry": {
            "cols": 80,
            "rows": 24
          },
          "name": "content",
          "cwd": null
        },
        "expect": {
          "/error": {
            "$present": false
          },
          "/result/spawn": "spawned",
          "/result/terminal/terminal_id": {
            "$capture": "terminal"
          }
        }
      }
    },
    {
      "call": {
        "conn": "desk",
        "id": "printed",
        "method": "terminal.wait_output",
        "params": {
          "target": {
            "name": "content"
          },
          "cursor": {
            "offset": "0"
          },
          "pattern": {
            "text": "prompt.answer"
          },
          "timeout_ms": "20000"
        },
        "expect": {
          "/result/outcome": "matched"
        }
      }
    },
    {
      "call": {
        "conn": "desk",
        "id": "screen",
        "method": "terminal.screen",
        "params": {
          "target": {
            "name": "content"
          },
          "format": "text"
        },
        "expect": {
          "/result/lines": {
            "$contains": [
              "herdr: server shut down: Approve? [y/n] call prompt.answer yes"
            ]
          },
          "/result/revision": {
            "$capture": "revision"
          }
        }
      }
    },
    {
      "call": {
        "conn": "desk",
        "id": "pinned",
        "method": "terminal.search",
        "params": {
          "target": {
            "name": "content"
          },
          "pattern": {
            "text": "Approve"
          },
          "revision": "$value:revision",
          "limit": 1
        },
        "expect": {
          "/error": {
            "$present": false
          },
          "/result/revision": "$value:revision"
        }
      }
    },
    {
      "call": {
        "conn": "desk",
        "id": "no-prompt",
        "method": "prompt.list",
        "params": {
          "target": {
            "name": "content"
          }
        },
        "expect": {
          "/result/prompts": []
        }
      }
    },
    {
      "call": {
        "conn": "desk",
        "id": "no-status",
        "method": "status.get",
        "params": {
          "target": {
            "name": "content"
          }
        },
        "expect": {
          "/result/status/reported": false
        }
      }
    },
    {
      "call": {
        "conn": "desk",
        "id": "still-live",
        "method": "terminal.get",
        "params": {
          "target": {
            "terminal_id": "$value:terminal"
          }
        },
        "expect": {
          "/result/state": "live"
        }
      }
    },
    {
      "call": {
        "conn": "desk",
        "id": "process",
        "method": "terminal.process_info",
        "params": {
          "target": {
            "name": "content"
          }
        },
        "expect": {
          "/result/supported_fields": {
            "$present": true
          },
          "/result/launcher/incarnation": {
            "$present": true
          }
        }
      }
    }
  ]
}
```

## Windows capability contract

Decide per endpoint from `hello`, never from the host OS name:

- `endpoint_kind` and `capabilities.transports` say how this endpoint is reached: `unix` on
  Linux and macOS, `pipe` for the Windows named pipe, `stdio` for SSH and custom transports,
  `websocket` where served. The method-resolution example requires `unix` on the platforms the
  scenario runs on.
- `capabilities.framed` says whether the connection may upgrade to framed mode;
  `capabilities.stream_modes` lists attach delivery modes. This build reports
  `stream_modes: []` while `attach.open` with `stream_mode: "plain"` works (see Known gaps),
  so treat an empty list as "not advertised", not as "unsupported".
- `terminal.process_info` lists `supported_fields`: process start tokens and roles that a
  platform cannot observe are absent from that list rather than guessed.
- The Bun client supports Unix sockets only and refuses Windows with
  `unsupported_transport`. Until Ovis has a pipe transport, keep `server/mirror.ts` behind this
  capability check instead of deleting it.

## Ovis policies kept

The scenario checks the examples against these policies; they also bind any later Ovis PR.

| policy | how it stays enforced |
|---|---|
| no automatic resend | uncertain sends stay drafts; a resend reuses the `op_id` and only on a human decision. No example sends the same mutation twice under two operation ids |
| prompt answers never guessed keys | prompts are answered with `prompt.answer` and a version. No example that reads a prompt also sends `input.*` keys |
| selection is not focus | `selection.set` is per client and layout edits keep every selection |
| notification and pairing UX | the runtime publishes `turn.ended`, `status.changed`, `prompt.opened` and `machine.changed`; Ovis decides how to alert and keeps device pairing |
| terminal content is never a command source | no example sends a value captured from screen lines, output, matches or records |

## Path by path

### `server/herdr/client.ts` (replace)

- Today: One socket per RPC, because herdr answers one frame and closes (`server/herdr/client.ts:181-182`); a 10 s default timeout (`:19`); 16 MiB and 64 MiB line caps (`:77-78`).
- Runtime contract: One persistent connection (`Client.connect`) carries correlated calls, subscriptions and attach streams until it is closed (IDEAL-004). `hello` returns `limits` and `capabilities`, so caps and deadlines come from the runtime instead of constants.
- Recommendation for a later Ovis PR: Replace the module with `@ovis-terminal/client`. Read `limits.max_frame_bytes` and `limits.rpc_default_deadline_ms` from `hello` rather than hard-coding them.

```json ovis-migration
{
  "path": "server/herdr/client.ts",
  "fate": "replace",
  "methods": [
    "hello",
    "rpc.discover",
    "schema.capabilities",
    "op.get",
    "op.cancel"
  ],
  "unserved": [],
  "events": [
    "heartbeat"
  ],
  "capabilities": [
    "capabilities.framed",
    "capabilities.transports",
    "limits.max_frame_bytes",
    "limits.max_in_flight_requests",
    "limits.rpc_default_deadline_ms"
  ],
  "identity": [
    "RuntimeLifetimeId",
    "ServerBootId",
    "ProtocolGeneration",
    "OpId"
  ],
  "fixtures": [
    "scenario:client/package",
    "scenario:protocol/multiplex"
  ],
  "examples": [
    "method-resolution"
  ]
}
```

### `server/herdr/methods.ts` (replace)

- Today: A hand-kept method-to-result table (`server/herdr/methods.ts:14-20`), written around herdr answering for the focused pane when given `target` (`:15`).
- Runtime contract: Generated `Methods`, `ParamsOf<M>` and `ResultOf<M>` cover every published method. Params are closed objects: a misspelled field is `invalid_params` with its `field_path`, and nothing runs (IDEAL-112).
- Recommendation for a later Ovis PR: Delete the table and type calls through the generated catalog. `Client.call` already rejects unknown methods and params locally before sending.

```json ovis-migration
{
  "path": "server/herdr/methods.ts",
  "fate": "replace",
  "methods": [
    "rpc.discover",
    "terminal.process_info",
    "input.send",
    "input.keys",
    "agent.get"
  ],
  "unserved": [],
  "events": [],
  "capabilities": [],
  "identity": [
    "TerminalId",
    "Target"
  ],
  "fixtures": [
    "fixture:strict-targeting",
    "scenario:protocol/strict-target"
  ],
  "examples": [
    "method-resolution"
  ]
}
```

### `server/collector.ts` (replace)

- Today: Reopens one subscription with the full pane set because a second `events.subscribe` silently kills delivery (`server/collector.ts:10-12`), snapshots after `onStarted` (`:22`, `:223`) and reconciles after `events_lost` (`:19`, `:77`).
- Runtime contract: Subscriptions on one connection are independent; each answers with `snapshot_seq` and only events committed after it follow. A retained `EventCursor` resumes; a cursor past the replay floor gets `missed` plus a fresh snapshot (IDEAL-019). `events.update` acknowledges a filter change with `effective_seq`.
- Recommendation for a later Ovis PR: Replace the reconcile loop with one subscription per concern and a stored cursor. Treat `missed` as the only reason to reload.

```json ovis-migration
{
  "path": "server/collector.ts",
  "fate": "replace",
  "methods": [
    "events.subscribe",
    "events.update",
    "events.unsubscribe",
    "session.snapshot"
  ],
  "unserved": [],
  "events": [
    "event",
    "missed",
    "heartbeat",
    "status.changed",
    "terminal.created",
    "terminal.finished",
    "prompt.opened"
  ],
  "capabilities": [
    "limits.event_replay_events",
    "limits.event_replay_bytes",
    "limits.heartbeat_ms"
  ],
  "identity": [
    "SubscriptionId",
    "EventSeq",
    "EventCursor",
    "RuntimeLifetimeId"
  ],
  "fixtures": [
    "fixture:subscriptions"
  ],
  "examples": [
    "startup-snapshot"
  ]
}
```

### `server/core/terminal-process.ts` (replace)

- Today: Spawns `herdr terminal attach` in a PTY (`server/core/terminal-process.ts:2`), recognizes a held attach by matching refusal text (`:30`) and retries read races for up to 20 s (`:25`, `:32-34`).
- Runtime contract: `attach.open` returns a typed `outcome` (ready, held, refused, recovering or ended) with `stream_id`, `lease` and, when held, `holder` and `retry.on` naming the event that ends the hold (IDEAL-028). Refusal and takeover never reach the byte stream (IDEAL-033).
- Recommendation for a later Ovis PR: Replace the PTY child and its regexes with `attach.open` plus `attach.control` records. Retry a held attach when the named `lease.revoked` event arrives, not on a timer.

```json ovis-migration
{
  "path": "server/core/terminal-process.ts",
  "fate": "replace",
  "methods": [
    "attach.open",
    "attach.close",
    "attach.takeover",
    "attach.leases"
  ],
  "unserved": [],
  "events": [
    "attach.control",
    "attach.data",
    "lease.acquired",
    "lease.revoked",
    "stream.closed"
  ],
  "capabilities": [
    "capabilities.stream_modes",
    "limits.credit_initial_bytes"
  ],
  "identity": [
    "StreamId",
    "LeaseId",
    "LeaseEpoch",
    "TerminalId",
    "ClientId",
    "Incarnation"
  ],
  "fixtures": [
    "fixture:attach-held-retry",
    "scenario:attach-input/all"
  ],
  "examples": [
    "viewer-streams"
  ]
}
```

### `server/attach-output.ts` (delete)

- Today: Filters `herdr: server shut down: terminal attach taken over` out of terminal bytes (`server/attach-output.ts:3`) and strips and answers attach startup probes (`:15-39`).
- Runtime contract: The runtime answers terminal queries once and removes them from attach bytes; refusals and handoffs are control records, never terminal text (EXCL-016, V-RT-HD-07).
- Recommendation for a later Ovis PR: Delete the module once attach goes through the runtime. Never reintroduce a filter: text that looks like a refusal is program output.

```json ovis-migration
{
  "path": "server/attach-output.ts",
  "fate": "delete",
  "methods": [
    "attach.open"
  ],
  "unserved": [],
  "events": [
    "attach.data",
    "attach.control",
    "stream.closed"
  ],
  "capabilities": [],
  "identity": [
    "StreamId",
    "ByteOffset"
  ],
  "fixtures": [
    "fixture:terminal-reads",
    "scenario:cli/interactive-attach"
  ],
  "examples": [
    "terminal-content",
    "viewer-streams"
  ]
}
```

### `server/output-window.ts` (delete)

- Today: Keeps a per-client window with its own UUID (`server/output-window.ts:31`), 256 KiB/64 KiB/1 MiB marks (`:3-5`) and closes a client with 4008 after a 2 s stall (`:6-7`).
- Runtime contract: Each attach stream has its own `stream_id`, delivery offset, credit and bounded backlog; a stale or impossible acknowledgement grants nothing, and a slow viewer gets a gap plus a checkpoint without affecting others (IDEAL-040).
- Recommendation for a later Ovis PR: Delete the window. Open one runtime stream per browser viewer, so the late-joiner checkpoint comes from the runtime.

```json ovis-migration
{
  "path": "server/output-window.ts",
  "fate": "delete",
  "methods": [
    "attach.open",
    "attach.credit",
    "attach.repaint"
  ],
  "unserved": [],
  "events": [
    "attach.data",
    "attach.control"
  ],
  "capabilities": [
    "limits.credit_initial_bytes",
    "limits.credit_max_bytes",
    "limits.stream_backlog_bytes"
  ],
  "identity": [
    "StreamId",
    "ByteOffset"
  ],
  "fixtures": [
    "scenario:attach-input/all"
  ],
  "examples": [
    "viewer-streams"
  ]
}
```

### `server/core/terminals.ts` (adapt)

- Today: Tracks geometry leadership with a `release-size` frame (`server/core/terminals.ts:461`) and repaints by a same-size SIGWINCH, coalesced and rate-limited (`:14-20`, `:194-206`).
- Runtime contract: The interact lease owns geometry (`attach.resize`, `attach.release_geometry`); `attach.repaint` redraws from the runtime's grid without signalling the program.
- Recommendation for a later Ovis PR: Adapt: keep the registry, but take geometry from the lease and call `attach.repaint` instead of nudging the program.

```json ovis-migration
{
  "path": "server/core/terminals.ts",
  "fate": "adapt",
  "methods": [
    "attach.resize",
    "attach.release_geometry",
    "attach.repaint",
    "attach.leases"
  ],
  "unserved": [],
  "events": [
    "lease.geometry",
    "terminal.size"
  ],
  "capabilities": [],
  "identity": [
    "LeaseId",
    "LeaseEpoch",
    "StreamId"
  ],
  "fixtures": [
    "scenario:attach/resize"
  ],
  "examples": [
    "viewer-streams"
  ]
}
```

### `server/mirror.ts` (delete)

- Today: Polls `pane.read` on Windows, where herdr cannot attach (`server/mirror.ts:2`, `:21-22`); the file calls itself a stopgap (`:16`).
- Runtime contract: The Windows holder owns ConPTY and serves a named pipe; `hello.endpoint_kind` and `capabilities.transports` say `pipe`. The Bun client refuses Windows with `unsupported_transport` today.
- Recommendation for a later Ovis PR: Delete only after Ovis can open a pipe transport. Until then gate the mirror on `capabilities.transports` and keep it; see Windows capability contract.

```json ovis-migration
{
  "path": "server/mirror.ts",
  "fate": "delete",
  "methods": [
    "terminal.screen",
    "attach.open",
    "schema.capabilities"
  ],
  "unserved": [],
  "events": [],
  "capabilities": [
    "capabilities.transports",
    "capabilities.stream_modes"
  ],
  "identity": [
    "TerminalId",
    "TransportKind"
  ],
  "fixtures": [
    "scenario:platform/matrix"
  ],
  "examples": [
    "method-resolution",
    "terminal-content"
  ]
}
```

### `server/core/input.ts` (replace)

- Today: Serializes input per pane with a promise chain (`server/core/input.ts:41-57`), waits for herdr's ~150 ms ESC hold (`:44`, `:82`) and rechecks typing guards (`:107-113`).
- Runtime contract: `input.send`, `input.keys`, `input.raw` and `input.paste` return an admission receipt (`op_id`, `queued_seq`, `output_cursor`, `mode_revision`). One terminal actor orders them, and the lease is checked at admission and again at the write.
- Recommendation for a later Ovis PR: Replace the queue with receipts. Pass the current `lease_id`, and use `input.barrier` where order matters across clients.

```json ovis-migration
{
  "path": "server/core/input.ts",
  "fate": "replace",
  "methods": [
    "input.send",
    "input.keys",
    "input.raw",
    "input.paste",
    "input.barrier",
    "input.interrupt"
  ],
  "unserved": [],
  "events": [
    "output.revision"
  ],
  "capabilities": [
    "limits.input_queue_bytes",
    "limits.session_input_bytes"
  ],
  "identity": [
    "OpId",
    "LeaseId",
    "EventSeq",
    "OutputCursor"
  ],
  "fixtures": [
    "fixture:attach-held-retry",
    "scenario:attach-input/all"
  ],
  "examples": [
    "viewer-streams"
  ]
}
```

### `server/core/pending.ts` (replace)

- Today: Holds first messages in memory, so a bridge restart loses them (`server/core/pending.ts:8-18`), retries on listed refusal codes (`:60`) and keeps uncertain sends as drafts (`:259`).
- Runtime contract: `terminal.first_message` binds a queued first message to the program's declared readiness, never output silence. Templates are versioned (`template.get`). A lost reply is `OutcomeUnknown` with a retained `Draft`.
- Recommendation for a later Ovis PR: Replace the hold with `terminal.first_message`. Keep the Ovis draft UX: show uncertain sends and let the human retry (same `op_id`) or discard.

```json ovis-migration
{
  "path": "server/core/pending.ts",
  "fate": "replace",
  "methods": [
    "terminal.first_message",
    "terminal.run",
    "template.list",
    "template.get",
    "op.get"
  ],
  "unserved": [],
  "events": [
    "op.outcome_unknown",
    "status.changed"
  ],
  "capabilities": [],
  "identity": [
    "OpId",
    "ProcessToken",
    "Incarnation"
  ],
  "fixtures": [
    "scenario:agent/launch"
  ],
  "examples": [
    "viewer-streams"
  ]
}
```

### `server/dialogs/live.ts` (replace)

- Today: Decides whether a prompt is still live by reading provider footers (`server/dialogs/live.ts:56-82`, `:87`).
- Runtime contract: Prompts are versioned records with `state` (open, answered, closed) and `prompt.opened/updated/closed` events (IDEAL-057). Liveness is the record's state.
- Recommendation for a later Ovis PR: Replace with prompt events. Keep a terminal-text fallback only for unintegrated agents, labelled as such in the UI.

```json ovis-migration
{
  "path": "server/dialogs/live.ts",
  "fate": "replace",
  "methods": [
    "prompt.list",
    "prompt.get"
  ],
  "unserved": [],
  "events": [
    "prompt.opened",
    "prompt.updated",
    "prompt.closed"
  ],
  "capabilities": [],
  "identity": [
    "PromptId",
    "RecordVersion",
    "Incarnation"
  ],
  "fixtures": [
    "fixture:prompt-cards",
    "scenario:agent/prompts"
  ],
  "examples": [
    "prompt.answer",
    "terminal-content"
  ]
}
```

### `server/prompt.ts` (replace)

- Today: Parses interactive prompts from the screen (`server/prompt.ts:427`) and answers them with navigation key sequences (`answerKeys`, `:434-489`).
- Runtime contract: `prompt.answer` names `prompt_id` and `version`; a stale version is `version_conflict`, a second answer to one version is a conflict, and no key is guessed (IDEAL-058).
- Recommendation for a later Ovis PR: Replace `answerKeys` with `prompt.answer`. On `version_conflict` re-read the card and let the human answer again.

```json ovis-migration
{
  "path": "server/prompt.ts",
  "fate": "replace",
  "methods": [
    "prompt.get",
    "prompt.answer",
    "prompt.close",
    "prompt.list"
  ],
  "unserved": [],
  "events": [
    "prompt.opened",
    "prompt.closed"
  ],
  "capabilities": [],
  "identity": [
    "PromptId",
    "RecordVersion"
  ],
  "fixtures": [
    "fixture:prompt-cards",
    "scenario:agent/bridge"
  ],
  "examples": [
    "prompt.answer"
  ]
}
```

### `server/claude-input.ts` (replace)

- Today: Infers whether Claude's input box holds a draft from screen colours (`server/claude-input.ts:156`, `:195`) and clears it with `ctrl+k ctrl+u` (`:202`).
- Runtime contract: Reads return text and a pinned `revision` but never authority; agent state comes from declared status and prompt records. Paste safety comes from the VT mode state behind `mode_revision`.
- Recommendation for a later Ovis PR: Replace the colour heuristics with status and prompt records. Where an agent declares nothing, label any screen-derived hint as a guess.

```json ovis-migration
{
  "path": "server/claude-input.ts",
  "fate": "replace",
  "methods": [
    "input.paste",
    "terminal.screen",
    "status.get"
  ],
  "unserved": [],
  "events": [
    "status.changed"
  ],
  "capabilities": [],
  "identity": [
    "TerminalId",
    "Revision"
  ],
  "fixtures": [
    "fixture:terminal-reads"
  ],
  "examples": [
    "terminal-content"
  ]
}
```

### `server/omo-status.ts` (replace)

- Today: Derives OmO status from transcript lines (`server/omo-status.ts:78`, `:115`) and background-task liveness from pids (`:132-150`).
- Runtime contract: Status has two writers: OSC 7501 from the owning PTY and `status.report` with its process token. Snapshots and events carry origin, sequence and turn (IDEAL-067).
- Recommendation for a later Ovis PR: Replace with `status.get` plus `status.changed`. The transcript logic belongs in a runtime provider adapter, which produces records, never status.

```json ovis-migration
{
  "path": "server/omo-status.ts",
  "fate": "replace",
  "methods": [
    "status.get",
    "turn.get",
    "turn.wait"
  ],
  "unserved": [],
  "events": [
    "status.changed",
    "turn.ended"
  ],
  "capabilities": [],
  "identity": [
    "AgentId",
    "TurnNumber",
    "FinishGeneration"
  ],
  "fixtures": [
    "fixture:agent-records",
    "scenario:agent/status"
  ],
  "examples": [
    "status-turns"
  ]
}
```

### `server/gjc-status.ts` (replace)

- Today: Finds GJC panes from `pane.process_info` foreground pids (`server/gjc-status.ts:118-132`) and reconciles provider status by hand (`:175`).
- Runtime contract: `terminal.process_info` reports launcher, shell, foreground and background roles with incarnations and start tokens; status stays declared.
- Recommendation for a later Ovis PR: Replace pid matching with process records, and status with declared status.

```json ovis-migration
{
  "path": "server/gjc-status.ts",
  "fate": "replace",
  "methods": [
    "terminal.process_info",
    "status.get"
  ],
  "unserved": [],
  "events": [
    "status.changed",
    "terminal.foreground"
  ],
  "capabilities": [],
  "identity": [
    "Incarnation",
    "AgentId"
  ],
  "fixtures": [
    "scenario:platform/process-info"
  ],
  "examples": [
    "status-turns",
    "terminal-content"
  ]
}
```

### `server/completion.ts` (replace)

- Today: `CompletionTracker` synthesizes `done` for agents herdr lost track of (`server/completion.ts:8-31`).
- Runtime contract: `turn.ended` and `status.last_turn` carry turn number, outcome and `finish_generation`; a stale report is `stale_seq` with the current sequence.
- Recommendation for a later Ovis PR: Replace the tracker with turn records. Ovis still decides when to alert and with which sound (EXCL-011, EXCL-012).

```json ovis-migration
{
  "path": "server/completion.ts",
  "fate": "replace",
  "methods": [
    "turn.get",
    "turn.wait",
    "status.get"
  ],
  "unserved": [],
  "events": [
    "turn.ended",
    "status.changed"
  ],
  "capabilities": [],
  "identity": [
    "TurnNumber",
    "FinishGeneration"
  ],
  "fixtures": [
    "fixture:agent-records"
  ],
  "examples": [
    "status-turns"
  ]
}
```

### `server/outcome.ts` (replace)

- Today: Infers slash-command outcomes from transcript records inside a 10-minute window (`server/outcome.ts:114`, `:591`).
- Runtime contract: Typed records (`agent.records`) carry tool calls and results linked by `call_id`; turns end with an outcome.
- Recommendation for a later Ovis PR: Replace transcript scraping with records and turn outcomes.

```json ovis-migration
{
  "path": "server/outcome.ts",
  "fate": "replace",
  "methods": [
    "agent.records",
    "turn.get"
  ],
  "unserved": [],
  "events": [
    "agent.records_appended",
    "turn.ended"
  ],
  "capabilities": [],
  "identity": [
    "AgentId",
    "TurnNumber"
  ],
  "fixtures": [
    "fixture:agent-records",
    "scenario:agent/records"
  ],
  "examples": [
    "history-identity",
    "status-turns"
  ]
}
```

### `server/core/status.ts` (replace)

- Today: Merges herdr's raw status word with transcript evidence and Claude subagent rules (`server/core/status.ts:112`, `:163-194`).
- Runtime contract: One authority: declared reports. Screen text, process names and brands are never status (EXCL-008, V-RT-HD-09).
- Recommendation for a later Ovis PR: Replace the merge with the runtime status record. Delete the rules that override a report.

```json ovis-migration
{
  "path": "server/core/status.ts",
  "fate": "replace",
  "methods": [
    "status.get",
    "status.clear",
    "agent.get"
  ],
  "unserved": [],
  "events": [
    "status.changed"
  ],
  "capabilities": [],
  "identity": [
    "AgentId",
    "ProcessToken"
  ],
  "fixtures": [
    "scenario:agent/status"
  ],
  "examples": [
    "status-turns"
  ]
}
```

### `server/agent-process.ts` (replace)

- Today: Picks the agent process in front of a pane by pid (`server/agent-process.ts:13-16`).
- Runtime contract: Process records carry an incarnation and a platform start token, so pid reuse cannot rebind a send.
- Recommendation for a later Ovis PR: Replace with `terminal.process_info` and bind sends to the incarnation.

```json ovis-migration
{
  "path": "server/agent-process.ts",
  "fate": "replace",
  "methods": [
    "terminal.process_info"
  ],
  "unserved": [],
  "events": [
    "terminal.foreground"
  ],
  "capabilities": [],
  "identity": [
    "Incarnation"
  ],
  "fixtures": [
    "scenario:platform/process-info"
  ],
  "examples": [
    "terminal-content"
  ]
}
```

### `server/conversation.ts` (move)

- Today: Parses Claude, Codex, Pi and OmO stores itself and prefixes cursors with a file-identity digest, throwing `HistoryChanged` on a mismatch (`server/conversation.ts:139-158`).
- Runtime contract: `agent.conversation` returns typed records with `history_id` and `revision`; cursors bind to both, so a rewritten transcript is detected (IDEAL-073, IDEAL-075).
- Recommendation for a later Ovis PR: Move the parsers into runtime provider adapters; Ovis reads `agent.conversation` and `agent.records`.

```json ovis-migration
{
  "path": "server/conversation.ts",
  "fate": "move",
  "methods": [
    "agent.conversation",
    "agent.records",
    "agent.update_session"
  ],
  "unserved": [],
  "events": [
    "agent.records_appended",
    "agent.session_bound"
  ],
  "capabilities": [],
  "identity": [
    "HistoryId",
    "HistoryRevision",
    "ConversationCursor"
  ],
  "fixtures": [
    "fixture:agent-records",
    "fixture:history-store"
  ],
  "examples": [
    "history-identity"
  ]
}
```

### `server/codex.ts` (move)

- Today: Parses Codex rollouts and its SQLite store (`server/codex.ts:205`, `:378`).
- Runtime contract: Provider stores are read by runtime adapters inside authorized stores only; a path escaping the store is refused.
- Recommendation for a later Ovis PR: Move to the runtime adapter crate.

```json ovis-migration
{
  "path": "server/codex.ts",
  "fate": "move",
  "methods": [
    "agent.history",
    "agent.conversation"
  ],
  "unserved": [],
  "events": [
    "agent.session_bound"
  ],
  "capabilities": [],
  "identity": [
    "HistoryId",
    "SessionRef"
  ],
  "fixtures": [
    "fixture:history-store"
  ],
  "examples": [
    "history-identity"
  ]
}
```

### `server/session-history.ts` (move)

- Today: Lists earlier sessions newest first, bounded by `MAX_ROWS = 50` (`server/session-history.ts:2`, `:62`).
- Runtime contract: `agent.history` is bounded and paged with `page.truncated`, and never follows paths outside authorized stores (IDEAL-078).
- Recommendation for a later Ovis PR: Move; Ovis renders `agent.history` pages.

```json ovis-migration
{
  "path": "server/session-history.ts",
  "fate": "move",
  "methods": [
    "agent.history"
  ],
  "unserved": [],
  "events": [],
  "capabilities": [],
  "identity": [
    "HistoryId",
    "SessionRef"
  ],
  "fixtures": [
    "fixture:history-store"
  ],
  "examples": [
    "history-identity"
  ]
}
```

### `server/pty/session.ts` (delete)

- Today: Hosts PTYs in a Rust host or a Node sidecar (`server/pty/session.ts:33-80`).
- Runtime contract: The runtime holder owns PTYs; programs survive a planned control restart with the same incarnation.
- Recommendation for a later Ovis PR: Delete after cutover.

```json ovis-migration
{
  "path": "server/pty/session.ts",
  "fate": "delete",
  "methods": [
    "terminal.run",
    "terminal.kill",
    "terminal.get"
  ],
  "unserved": [],
  "events": [
    "terminal.spawned",
    "terminal.finished"
  ],
  "capabilities": [],
  "identity": [
    "TerminalId",
    "Incarnation"
  ],
  "fixtures": [
    "scenario:recovery/restart"
  ],
  "examples": [
    "viewer-streams"
  ]
}
```

### `server/pty/pty-host.mjs` (delete)

- Today: Runs node-pty in Node because loading it in Bun panicked (`server/pty/pty-host.mjs:6-8`, `:24-29`).
- Runtime contract: Output is read through `terminal.output` pages and cursor waits, with no PTY library in Ovis.
- Recommendation for a later Ovis PR: Delete after cutover.

```json ovis-migration
{
  "path": "server/pty/pty-host.mjs",
  "fate": "delete",
  "methods": [
    "terminal.run",
    "terminal.output",
    "terminal.wait_output"
  ],
  "unserved": [],
  "events": [
    "terminal.finished"
  ],
  "capabilities": [],
  "identity": [
    "TerminalId",
    "ByteOffset"
  ],
  "fixtures": [
    "fixture:terminal-reads"
  ],
  "examples": [
    "terminal-content"
  ]
}
```

### `crates/pty-sidecar/` (delete)

- Today: A Rust PTY sidecar (`crates/pty-sidecar/src/main.rs`, `pty.rs`, `frames.rs`, `input.rs`).
- Runtime contract: Holders and restarts are runtime facts: `server.restart` keeps program incarnations, `server.recovery` reports boundaries.
- Recommendation for a later Ovis PR: Delete the crate after cutover.

```json ovis-migration
{
  "path": "crates/pty-sidecar/",
  "fate": "delete",
  "methods": [
    "terminal.run",
    "server.restart",
    "server.status"
  ],
  "unserved": [],
  "events": [
    "terminal.restarting",
    "terminal.restarted",
    "server.recovery"
  ],
  "capabilities": [],
  "identity": [
    "Incarnation",
    "ServerBootId",
    "RuntimeLifetimeId"
  ],
  "fixtures": [
    "scenario:recovery/restart"
  ],
  "examples": [
    "terminal-content"
  ]
}
```

### `server/machines.ts` (adapt)

- Today: Reaches PCs through SSH tunnels to a bridge with a bearer token (`server/machines.ts:1035-1037`) and stores machines by its own id (`:28`).
- Runtime contract: Machines have stable `machine_id`s; names and aliases resolve once at admission and cannot redirect a binding (IDEAL-080). A lost line marks resources unreachable with last-known revision, never a local fallback (IDEAL-082).
- Recommendation for a later Ovis PR: Adapt: take identity, state and revision from `machine.*`. Keep SSH setup, device pairing and the PC dialog in Ovis.

```json ovis-migration
{
  "path": "server/machines.ts",
  "fate": "adapt",
  "methods": [
    "machine.add",
    "machine.list",
    "machine.get",
    "machine.rename"
  ],
  "unserved": [
    "machine.rm",
    "machine.login",
    "machine.exec",
    "machine.file"
  ],
  "events": [
    "machine.changed"
  ],
  "capabilities": [],
  "identity": [
    "MachineId",
    "MachineName"
  ],
  "fixtures": [
    "scenario:remote/e2e",
    "scenario:remote/isolation"
  ],
  "examples": [
    "machine-updates"
  ]
}
```

### `server/machine-api.ts` (adapt)

- Today: Serves the browser an SSE stream with heartbeats and device-paired events (`server/machine-api.ts:155-297`).
- Runtime contract: The runtime publishes `machine.changed` on `events.subscribe`; browser transport and pairing stay Ovis policy.
- Recommendation for a later Ovis PR: Adapt: feed the SSE route from a runtime subscription. Keep pairing.

```json ovis-migration
{
  "path": "server/machine-api.ts",
  "fate": "adapt",
  "methods": [
    "events.subscribe",
    "machine.list"
  ],
  "unserved": [],
  "events": [
    "machine.changed",
    "heartbeat"
  ],
  "capabilities": [],
  "identity": [
    "MachineId",
    "SubscriptionId"
  ],
  "fixtures": [
    "scenario:remote/e2e"
  ],
  "examples": [
    "machine-updates",
    "startup-snapshot"
  ]
}
```

### `server/remote-host.ts` (adapt)

- Today: Uploads and installs a remote bundle, and starts the bridge through WMI on Windows (`server/remote-host.ts:3-33`).
- Runtime contract: `update.stage`, `update.verify`, `update.activate`, `update.rollback` and `update.query` expose verification, activation and rollback without replacing the active runtime on a failed check (IDEAL-100).
- Recommendation for a later Ovis PR: Adapt: drive staged updates; keep the SSH installer only for first contact.

```json ovis-migration
{
  "path": "server/remote-host.ts",
  "fate": "adapt",
  "methods": [
    "update.stage",
    "update.verify",
    "update.activate",
    "update.rollback",
    "update.query",
    "service.describe"
  ],
  "unserved": [
    "service.install",
    "service.remove"
  ],
  "events": [
    "update.progress"
  ],
  "capabilities": [],
  "identity": [
    "UpdateId",
    "ReleaseVersion",
    "Sha256Digest"
  ],
  "fixtures": [
    "scenario:services/update"
  ],
  "examples": [
    "machine-updates"
  ]
}
```

### `server/workspace-routes.ts` (adapt)

- Today: Creates the workspace, then launches the agent separately (`server/workspace-routes.ts:143-156`); a failed launch answers `agent_started: false` (`:148`).
- Runtime contract: `layout.edit` `create_workspace` returns every created id plus a separate `spawn` outcome, so a failed launch cannot hide a created workspace (IDEAL-085).
- Recommendation for a later Ovis PR: Adapt the route to one `layout.edit` with a program and report `created` and `spawn` separately.

```json ovis-migration
{
  "path": "server/workspace-routes.ts",
  "fate": "adapt",
  "methods": [
    "layout.edit",
    "layout.get",
    "terminal.first_message"
  ],
  "unserved": [],
  "events": [
    "terminal.created",
    "layout.changed"
  ],
  "capabilities": [],
  "identity": [
    "WorkspaceId",
    "TabId",
    "PaneId",
    "OpId"
  ],
  "fixtures": [
    "fixture:layout-explicit-ids"
  ],
  "examples": [
    "layout-receipts"
  ]
}
```

### `server/pane-routes.ts` (adapt)

- Today: Splits with `focus:false` by default (`server/pane-routes.ts:386-390`), moves herdr's focus on `/api/pane/focus` (`:382-383`) and sets ratios by tree path (`:363`).
- Runtime contract: Split, swap and ratio edits name explicit ids, take `expect_revision`, and return revision, created and removed ids or a typed refusal; selections are per client and never move another client's (IDEAL-084, IDEAL-086).
- Recommendation for a later Ovis PR: Adapt: call `layout.edit` with explicit ids and `selection.set` with the browser's own `client_id`.

```json ovis-migration
{
  "path": "server/pane-routes.ts",
  "fate": "adapt",
  "methods": [
    "layout.edit",
    "layout.get",
    "selection.set",
    "terminal.rename"
  ],
  "unserved": [],
  "events": [
    "layout.changed",
    "selection.changed"
  ],
  "capabilities": [],
  "identity": [
    "PaneId",
    "SplitNodeId",
    "ClientId",
    "Revision"
  ],
  "fixtures": [
    "fixture:layout-explicit-ids",
    "fixture:strict-targeting"
  ],
  "examples": [
    "layout-receipts",
    "method-resolution"
  ]
}
```

### `shared/herdr-api.generated.ts` (replace)

- Today: Generated herdr types with enums widened by `(string & {})` (`shared/herdr-api.generated.ts:55`, `:65`).
- Runtime contract: `@ovis-terminal/client/protocol` is generated from `schema/ovis-terminal.openrpc.json` with collision-free names; output enums are open and `isKnown` tells a new value apart (IDEAL-112, IDEAL-113).
- Recommendation for a later Ovis PR: Replace with the package's protocol types.

```json ovis-migration
{
  "path": "shared/herdr-api.generated.ts",
  "fate": "replace",
  "methods": [
    "rpc.discover"
  ],
  "unserved": [],
  "events": [],
  "capabilities": [],
  "identity": [
    "ProtocolGeneration",
    "BuildVersion"
  ],
  "fixtures": [
    "scenario:client/package"
  ],
  "examples": [
    "method-resolution"
  ]
}
```

### `scripts/generate-protocol-types.ts` (replace)

- Today: Reads `herdr api schema --json` and namespaces colliding names (`scripts/generate-protocol-types.ts:18-24`, `:232`).
- Runtime contract: The schema is published by the runtime; same-generation releases only grow it, enforced against `schema/released.openrpc.json` (CAT-3471).
- Recommendation for a later Ovis PR: Delete the generator and depend on the package.

```json ovis-migration
{
  "path": "scripts/generate-protocol-types.ts",
  "fate": "replace",
  "methods": [
    "rpc.discover",
    "schema.errors",
    "schema.events",
    "schema.capabilities"
  ],
  "unserved": [],
  "events": [],
  "capabilities": [],
  "identity": [
    "ProtocolGeneration"
  ],
  "fixtures": [
    "scenario:client/package",
    "scenario:protocol/discovery"
  ],
  "examples": [
    "method-resolution"
  ]
}
```

### `shared/protocol.ts` (adapt)

- Today: Carries `herdr_lifetime` (`shared/protocol.ts:65`, `:1035-1037`) and a `pane-focused` message for herdr's focus (`:1088-1091`).
- Runtime contract: `runtime_lifetime` and `server_boot_id` come from `hello` and every event envelope; there is no focus, only per-client selections.
- Recommendation for a later Ovis PR: Adapt: map lifetime to runtime identity and drop `pane-focused`.

```json ovis-migration
{
  "path": "shared/protocol.ts",
  "fate": "adapt",
  "methods": [
    "hello",
    "selection.set",
    "session.snapshot"
  ],
  "unserved": [],
  "events": [
    "terminal.updated",
    "selection.changed"
  ],
  "capabilities": [],
  "identity": [
    "RuntimeLifetimeId",
    "ServerBootId",
    "Selection"
  ],
  "fixtures": [
    "fixture:layout-explicit-ids"
  ],
  "examples": [
    "startup-snapshot",
    "layout-receipts"
  ]
}
```

### `shared/machines.ts` (adapt)

- Today: Defines its own machine states and bundle versions (`shared/machines.ts:6-10`, `:93-101`).
- Runtime contract: `MachineRecord.state` is connected, connecting or unreachable with a `revision`; update records carry release versions.
- Recommendation for a later Ovis PR: Adapt the shared types to `MachineRecord` and `UpdateQueryResult`.

```json ovis-migration
{
  "path": "shared/machines.ts",
  "fate": "adapt",
  "methods": [
    "machine.list",
    "machine.get",
    "update.query"
  ],
  "unserved": [],
  "events": [
    "machine.changed",
    "update.progress"
  ],
  "capabilities": [],
  "identity": [
    "MachineId",
    "MachineName",
    "ReleaseVersion"
  ],
  "fixtures": [
    "scenario:remote/e2e"
  ],
  "examples": [
    "machine-updates"
  ]
}
```

### `shared/plugins.ts` (adapt)

- Today: Mirrors herdr plugin actions; without `pane_id` herdr runs an action in its own focus (`shared/plugins.ts:29`); runs are running, succeeded or failed (`:37-41`).
- Runtime contract: `plugin.invoke` takes an explicit target and returns a `PluginRunId`; `plugin.run_get` and `plugin.run` events report bounded outcomes.
- Recommendation for a later Ovis PR: Adapt to run ids and explicit targets.

```json ovis-migration
{
  "path": "shared/plugins.ts",
  "fate": "adapt",
  "methods": [
    "plugin.list",
    "plugin.invoke",
    "plugin.run_get"
  ],
  "unserved": [],
  "events": [
    "plugin.run"
  ],
  "capabilities": [],
  "identity": [
    "PluginId",
    "PluginRunId",
    "PluginActionId"
  ],
  "fixtures": [
    "scenario:security/validation"
  ],
  "examples": [
    "method-resolution"
  ]
}
```

### `shared/pane-layout.ts` (adapt)

- Today: Encodes `changed` with free-text reasons such as `single_pane` or `no_neighbor` (`shared/pane-layout.ts:28-33`, `:45-49`, `:67-71`).
- Runtime contract: `LayoutChange` returns `created`, `removed`, `no_op` and the new revision; an out-of-range ratio is `invalid_geometry`, never clamped.
- Recommendation for a later Ovis PR: Adapt the types to `LayoutChange`.

```json ovis-migration
{
  "path": "shared/pane-layout.ts",
  "fate": "adapt",
  "methods": [
    "layout.edit",
    "layout.get"
  ],
  "unserved": [],
  "events": [
    "layout.changed"
  ],
  "capabilities": [],
  "identity": [
    "PaneId",
    "SplitNodeId",
    "Revision"
  ],
  "fixtures": [
    "fixture:layout-explicit-ids"
  ],
  "examples": [
    "layout-receipts"
  ]
}
```

### `shared/terminal-insert.ts` (adapt)

- Today: Refuses multiline inserts because herdr reports no terminal modes (`shared/terminal-insert.ts:6-12`, `:32`).
- Runtime contract: The runtime tracks bracketed-paste mode and rejects unsafe multiline input when the mode cannot preserve it; receipts carry `mode_revision`.
- Recommendation for a later Ovis PR: Adapt: let `input.paste` decide, and show its typed refusal.

```json ovis-migration
{
  "path": "shared/terminal-insert.ts",
  "fate": "adapt",
  "methods": [
    "input.paste",
    "input.send",
    "terminal.get"
  ],
  "unserved": [],
  "events": [],
  "capabilities": [],
  "identity": [
    "Revision",
    "OpId"
  ],
  "fixtures": [
    "scenario:attach-input/all"
  ],
  "examples": [
    "viewer-streams"
  ]
}
```

### `shared/workspace-lifecycle.ts` (adapt)

- Today: `WorkspaceCreated` reports `agent_started` beside the created ids (`shared/workspace-lifecycle.ts:115-134`).
- Runtime contract: Creation receipts list every created id and a separate spawn outcome; worktrees have their own ids and `worktree.changed` events.
- Recommendation for a later Ovis PR: Adapt the types to `LayoutChange` and `WorktreeRecord`.

```json ovis-migration
{
  "path": "shared/workspace-lifecycle.ts",
  "fate": "adapt",
  "methods": [
    "layout.edit",
    "worktree.list",
    "worktree.create",
    "worktree.open",
    "worktree.remove"
  ],
  "unserved": [],
  "events": [
    "worktree.changed"
  ],
  "capabilities": [],
  "identity": [
    "WorkspaceId",
    "TabId",
    "PaneId"
  ],
  "fixtures": [
    "fixture:layout-explicit-ids",
    "scenario:services/worktrees"
  ],
  "examples": [
    "layout-receipts"
  ]
}
```

### `ovis:policies` (keep)

- Today: Ovis never resends input after a disconnect (uncertain sends become drafts, `server/core/pending.ts:259`), never answers a prompt with guessed keys once a typed answer exists, keeps browser selection apart from terminal focus, and owns notification, sound and pairing UX.
- Runtime contract: The runtime now guarantees the facts these policies need: admission receipts and retained op results, versioned prompts, per-client selections, durable events.
- Recommendation for a later Ovis PR: Keep all four policies in Ovis. The runtime adds no toast, sound, modal or focus change (EXCL-001, EXCL-004, EXCL-011, EXCL-012, EXCL-013).

```json ovis-migration
{
  "path": "ovis:policies",
  "fate": "keep",
  "methods": [
    "prompt.answer",
    "op.get",
    "selection.set",
    "events.subscribe"
  ],
  "unserved": [],
  "events": [
    "prompt.opened",
    "turn.ended",
    "status.changed"
  ],
  "capabilities": [],
  "identity": [
    "OpId",
    "PromptId",
    "RecordVersion",
    "ClientId"
  ],
  "fixtures": [
    "fixture:prompt-cards",
    "fixture:attach-held-retry"
  ],
  "examples": [
    "prompt.answer",
    "viewer-streams",
    "terminal-content",
    "layout-receipts"
  ]
}
```

## Known gaps

Measured against this build; the scenario keeps the unserved list honest by failing if any of
these becomes served while the guide still lists it as unserved.

- Unserved by `ovis-terminal serve`: `agent.list`, `agent.note`, `agent.release`,
  `agent.wait_children`, `doc.list`, `events.ack`, `machine.rm`, `machine.login`,
  `machine.exec`, `machine.file`, `server.start`, `service.install`, `service.remove`,
  `terminal.history`.
- `op.get` answers `not_found` for `input.*` operations, so delivery of a send whose reply was
  lost cannot be inspected yet; reuse the same `op_id` only on an explicit human decision.
- A `layout.edit` receipt's `seq` is not the `seq` of the `layout.changed` envelope it
  publishes (receipt `16`, envelope `4` in the layout-receipts replay); correlate an edit with
  its event by `data.op_id` and `data.revision`.
- `events.wait` replies carry `outcome` and `seq` but no terminal id; correlate by the target
  you asked about.
- `schema.capabilities` reports `stream_modes: []` although plain attach streams work.
- Attaching across a control-server restart is not covered by a conformance scenario.
- `machine.list` is empty until a remote machine is added; the local machine id comes from
  `hello` and terminal records.
- Image placeholders on `terminal.screen` reads land after this guide's base; render them as
  placeholders, never as text to act on.

## Exclusions and violations this migration must not reintroduce

- EXCL-001, EXCL-013: no runtime chrome, onboarding or announcement modal gates an operation;
  Ovis draws everything.
- EXCL-003: every operation in this guide is a typed method; nothing depends on a pointer target.
- EXCL-004: no RPC promotes a client or takes geometry; selection is per client.
- EXCL-008, V-RT-HD-09: status is never inferred from the screen tail, and an explicit report is
  never overridden by a heuristic.
- EXCL-011, EXCL-012: notification timing, toasts and sounds stay in Ovis.
- EXCL-016, V-RT-HD-07: attach refusals and handoffs are typed outcomes; no filter for
  `server shut down` text.
- V-RT-HD-01: one persistent multiplexed connection; never one socket per request.
