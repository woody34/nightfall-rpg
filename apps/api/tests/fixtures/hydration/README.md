These are actual server-produced `nightfall.v1.ServerMessage` frames for the client
projection tests. Decode each `frames` entry from base64, then use the normal protobuf
decoder and dispatch in array order. The producer tests use test characters and real
zone commands; the socket producers use the durable actor and checkpoint gate.

- `npc-effects.json`: Shield cast on an NPC, then late-observer identity and existing
  effect hydration. Producer: `tests/zone_phase3_runtime.rs:2761`.
- `pending-swings.json`: late observer of mutually swinging players. Original swing
  start precedes the identity baseline; deadlines and endpoint lives are preserved.
  Producer: `tests/phase3_hydration_sockets.rs:292`.
- `replacement-offer.json`: pending offer on socket replacement, followed by acceptance,
  revival owner baselines, and the terminal offer for the previous corpse life.
  Producer: `tests/phase3_hydration_sockets.rs:581`.

Contract: architecture-decisions C12 (additive offer baseline) and C2 (identity-first
hydration). The offer mapper marks only an Offered frame anchored to a matching V5
identity id/incarnation/tick in that observer batch (`src/interface/zone_mapping.rs:1494`).

Regenerate explicitly into a temporary directory; ordinary tests never rewrite these
committed files:

```sh
mkdir -p /tmp/nightfall-hydration-frames
NIGHTFALL_HYDRATION_FIXTURE_DIR=/tmp/nightfall-hydration-frames \
  cargo test --test phase3_hydration_sockets --test zone_phase3_runtime -- --test-threads=1
```

Compare decoded semantics when regenerating socket captures: test character UUIDs and
socket admission ticks can differ. The tests assert the required temporal and privacy
relationships against the actual producer before exporting frames. Historical `.nfr`
recordings are unaffected and remain subject to `fixtures/sessions/replay-check.sh`.
