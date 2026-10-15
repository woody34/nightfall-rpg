These files are real `nightfall.v1.ServerMessage` frames that a server made, for the client
projection tests. Decode each `frames` entry from base64. Then use the usual protobuf
decoder and dispatch the frames in array order. The producer tests use test characters and real
zone commands. The socket producers use the durable actor and the checkpoint gate.

- `npc-effects.json`: a Shield cast on an NPC, then the late-observer identity and the hydration of the
  effect that exists. Producer: `tests/zone_phase3_runtime.rs:2761`.
- `pending-swings.json`: a late observer of two players that swing at each other. The original swing
  starts before the identity baseline. The deadlines and the endpoint lives stay the same.
  Producer: `tests/phase3_hydration_sockets.rs:292`.
- `replacement-offer.json`: a pending offer on socket replacement, then the acceptance,
  the revival owner baselines, and the terminal offer for the previous corpse life.
  Producer: `tests/phase3_hydration_sockets.rs:581`.

Contract: architecture-decisions C12 (additive offer baseline) and C2 (identity-first
hydration). The offer mapper marks only an Offered frame that is anchored to a matching V5
identity id/incarnation/tick in that observer batch (`src/interface/zone_mapping.rs:1494`).

To make these files again, write them into a temporary directory. The usual tests never write these
committed files again:

```sh
mkdir -p /tmp/nightfall-hydration-frames
NIGHTFALL_HYDRATION_FIXTURE_DIR=/tmp/nightfall-hydration-frames \
  cargo test --test phase3_hydration_sockets --test zone_phase3_runtime -- --test-threads=1
```

When you make the socket captures again, compare the decoded semantics. The test character UUIDs and
the socket admission ticks can be different. Before the tests export the frames, they assert the necessary
temporal and privacy relations against the real producer. This procedure has no effect on the historical `.nfr`
recordings. These recordings stay subject to `fixtures/sessions/replay-check.sh`.
