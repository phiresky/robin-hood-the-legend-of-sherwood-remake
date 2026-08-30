# Timed missions and runtime ambience

Hackable `Data/Levels/<mission>.level.json` descriptors may contain a time
limit and an ambience schedule. Both clocks count completed interactive game
ticks at 25 Hz, not wall time. They therefore pause with the game's
single-player pause, modal and engine-lock semantics. Multiplayer remains
server/lockstep authoritative and does not acquire a client-local pause.

```json
{
  "timed_mission": {
    "limit_seconds": 180,
    "warning_seconds": 45,
    "countdown": "final_only"
  },
  "ambience_schedule": [
    { "at_seconds": 20, "ambiance": "night", "transition_seconds": 10 },
    { "at_seconds": 70, "ambiance": "fog", "transition_seconds": 8 },
    { "at_seconds": 120, "ambiance": "day", "transition_seconds": 10 }
  ]
}
```

`limit_seconds` must be positive. `warning_seconds` defaults to 60 and may not
exceed the limit. `countdown` is `always`, `final_only`, or `hidden`; the
player's Graphics → Mission Countdown setting is an additional local gate.
The timer stops as soon as the mission is won. Expiry follows the normal loss
path, so campaign and debriefing behavior stays consistent with scripted loss.

Schedule entries must be strictly increasing by `at_seconds`; zero is valid
and changes the effective load ambience. `ambiance` accepts `day`, `night`,
`fog`, `attack`, and `custom1` through `custom4`. Perception radius, authored
light sectors, ambience-filtered sound and background/minimap selection switch
on the cue tick. `transition_seconds` crossfades the deterministic RGB565
lighting from the old value to the new one. A missing ambience-specific map or
minimap logs a warning and uses the existing Day, then bare-map fallback.

Options → Gameplay can disable authored timers or dynamic-ambience gameplay.
Those settings are deterministic simulation configuration and are negotiated,
recorded and hashed. Graphics → Dynamic Ambience Visuals independently keeps
the local renderer on the mission's initial presentation without changing
gameplay. All runtime clock, cue cursor, ambience and crossfade values are part
of mission save/snapshot state.

Run the included example with the normal mod mission picker or directly:

```text
robin --mission TimedAmbienceDemo
```
