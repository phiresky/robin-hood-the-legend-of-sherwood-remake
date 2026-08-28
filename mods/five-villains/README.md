# The Five Villains

This custom mission reuses the original **Save Stuteley** mission—the early
Nottingham mission where Robin begins alone and frees Stutely and three Merry
Men—but replaces its complete playable cast with the five villains.

Guy of Guisbourne replaces Robin at the original entry point. Longchamp,
Prince John, Scathlock, and the Sheriff occupy the four original prisoner
slots and join the playable party as they are freed. The original Nottingham
map, guards, civilians, objectives, triggers, and compiled mission script are
retained. Guy inherits Robin's mission role, so Robin-only rescue, VIP, and
script checks continue to work with the replacement cast.

Mission-specific popup, objective, and dialogue overrides live in
`Data/Levels/FiveVillains.text.patch.json`. Unspecified IDs retain the base
mission text and pictures, so the villain rewrite can be expanded without
editing `Level.res` or the compiled mission script.

Launch from the custom-mission menu or directly:

```sh
cargo run --bin robin -- --mission FiveVillains --proto nottingham
```
