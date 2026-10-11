# V5 cast/effect regression recording

`phase3-casts-v5.nfr` is an offline, deterministic domain recording using the complete
shipped Phase3 rules, Snapshot9 and Record7. It pins the original experimental runtime
revision 1. Its historical generator is `record-phase3-casts.sh` at commit `7d28b9e`;
regenerating with current revision-2 defaults would intentionally produce different bytes.
Normal tests never rewrite it. Current gameplay uses the separate `phase3-gameplay-v5.nfr`.
The generator verifies encoded and digest-only replay, a midcast snapshot suffix, and
rejection of tampered facts before writing the file.

The initial snapshot contains one admitted human mystic and one admitted keltir. The
fixture seeds learned Shield 1 and Wind Strike 1 before player admission. Commands run
two real Shield casts (including replacement), admit a second observing player, start
and cancel Wind Strike, cancel a staged cast in the same tick, and reject an unknown
skill. AOI entry hydrates the existing effect without creating another lifecycle fact.
No live state is modified directly, and no production fixture endpoint is involved.

This recording covers the supported cast/effect slice. It does not cover auto/CP combat,
player death/PvP/resurrection, the complete RNG matrix, paid-learning durability, the
packaged client soak, or full-pool gameplay. It is not an integration-readiness claim or
the complete scenario recording required by design section 8.2.
