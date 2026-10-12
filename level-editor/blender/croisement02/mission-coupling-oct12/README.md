# Remaining mission patch coupling

Private CPU recipes for 18 selected Crossings02 patch occurrences. They do not
modify shared application code, the canonical library, geometry or approvals.

`bindings.py` binds 15 arrow callbacks directly from current SCB JSON quads.
Its restricted interpreter supports only the observed one-shot callback subset:
natives emit typed commands, never gameplay effects. Unknown operations fail.
The commands retain sequence boundaries and actor/location handles, including
Emb07's differing moved and unlocked soldier sets. Two Tac06 patches have no
literal script reference or automatic apply sector. Tac19's loose-net patch has
an automatic PC-crossing triangle; it has no proven capture callback.

`preview.py` samples initial, apply and reset epochs using caller-supplied eligible
update counts. Initial/reset means this patch's state, not a reset of the callback
latch, actor positions or recorded sequences. The caller owns the authoritative
clock and full mission snapshot. This helper does not schedule delayed AI unlocks.

`verify.py` checks current input hashes, source quads, actual timing boundaries,
placement against installed native contracts, and exact donor-source/model pins.
`native-binding-candidates.json` selects each native patch explicitly from its
pinned existing context contract. Integrated terminal artwork follows that
patch's `integrate_in_background` flag; it does not need an invented background
ID. Candidate selection is not canonical publication or gameplay verification.

Outputs are immutable under
`work/croisement02-refinement/mission-coupling-oct12-v1/`. Use `bindings-v2.json`;
`bindings.json` is an earlier diagnostic superseded after checking automatic
crossing and sprite-origin semantics. Neither version is a geometry approval.

Run the eight bounded tests from the repository root:

```sh
python -m unittest discover -s level-editor/blender/croisement02/mission-coupling-oct12 -p 'test_*.py'
```

The builder and verifier deliberately refuse to overwrite existing packets.
For a fresh run, change the private output directory explicitly. Root owns
production integration, browser review and any physical-state reuse decision.
