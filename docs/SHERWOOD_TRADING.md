# Sherwood item trading

Sherwood trading converts stored production items into ransom currency. It is
an optional gameplay rule (`Sherwood Item Trading` in Gameplay settings), is
enabled for newly created profiles, and remains disabled when an older profile
has no saved value for the setting.

The panel is opened with **T** while in Sherwood. Mouse and touch users can open
it directly from the live Sherwood HUD's money icon, or from the **Sherwood
trading** row in the pause menu, whenever the local player is the host and the
rule is enabled. All routes pass through the same typed input request and
revalidate host ownership, setting, and location immediately before the modal
opens. If the player assigns **T** to a custom shortcut, that binding takes
precedence and the two pointer routes remain available. Select an inventory row and use
the explicit **Sell 1** or **Sell 5** action. `Sell 5` is disabled below five
units. A second activation confirms the exact item, quantity, and proceeds; the
UI does not change stock or currency until the authoritative simulation returns
a receipt.

## Price table

| Production item | Capacity in shipped Sherwood | Unit price |
| --- | ---: | ---: |
| Arrow | 50 | £1 |
| Purse | 25 | £2 |
| Stone | 25 | £2 |
| Apple | 25 | £3 |
| Lamb leg | 25 | £3 |
| Ale | 25 | £4 |
| Plant/herb | 35 | £5 |
| Net | 15 | £7 |
| Wasp nest | 15 | £9 |

The original Sherwood script registers every item sector at speed 5. The
original production formula is `speed / 1000 × workers × mission seconds`, with
a 1.5× specialist multiplier, so production labor/time gives every item the
same base value. Prices then apply conservative whole-pound modifiers for:

- storage scarcity (50, 35, 25, or 15 authored slots),
- tactical utility and the opportunity cost of consuming the item rather than
  carrying it into a mission,
- the stronger crowd-control value of nets and wasp nests.

The £2 purse price values the produced, unfilled bag at the same conservative
tier as stones: both have 25 shipped storage slots, while using a purse in a
mission separately withdraws five £10 coins from campaign ransom. Selling purse
stock therefore pays only for the produced item; it never credits the coins
that would be placed in a thrown purse.

The values are intentionally well below the equivalent value of mission ransom
pickups, keeping production a supplementary income source rather than a way to
skip campaign progression. There is no buy-back path and no dynamic price that
could be manipulated between save/reload or network peers.

## Integrity and anti-exploit rules

- Only the host seat can issue a sale, and the simulation accepts it only while
  the current campaign mission is Sherwood.
- A sale removes exact units from active production-point bonus stacks in
  deterministic entity-table order. Equipped hero ammunition and in-flight
  projectiles are not sold.
- Every true `MAKE_*` inventory item is sellable, including purses. Training,
  healing, relic, unknown, and all non-item sector types are rejected.
- Insufficient stock and ransom integer overflow reject the entire command;
  there is no clamping or partial success.
- Proceeds update campaign ransom directly. They intentionally do not increment
  mission-collected-money, score, or achievement statistics.
- The command, configuration, resulting world/campaign state, and receipt are
  serialized/state-hashed for replay, rollback, saves, and multiplayer.
- Multiplayer trading is a non-pausing overlay. Every host request carries a
  session-local correlation id, so delayed or stale receipts cannot mutate a
  newly opened panel; clients neither issue trades nor retain host receipts.

Original behavior references: `original-code/RHSectorProduction.cpp` for the
formula and item-to-action mapping, `original-code/RHScript.cpp` for production
sector registration, and the shipped Sherwood script for speed and production
point counts. The original game contains no item-sale mechanic.
