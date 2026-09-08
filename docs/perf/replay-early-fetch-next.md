# Replay mission fetching before runtime startup: investigation

2026-09-08, source audit at `2c504019a`.

Do not start a bulk mission prefetch as soon as isolated replay admission finishes. It competes with the runtime download on the same 16 Mbit/s connection and can substantially delay compilation, decoding, and renderer preparation. The remaining useful scheduling opportunity is after runtime transfer, before authoritative mission loading starts. This report changes no production loading behavior.

## Existing overlap and available information

`wasm-www/src/boot-lifecycle.ts` already downloads the boot datadir in parallel with the runtime. `src/main.ts` and `src/replay.ts` also overlap isolated replay admission with that runtime load. Merely adding another `Promise.all` does not uncover a serial network phase.

The admission worker's `validate_compact_replay` checks the exact selected build and bounded replay structure, then discards the decoded result and returns acceptance. It does not return a mission download plan. The replay header could provide a validated mission identity, but that alone cannot determine content-addressed part URLs or every required character dependency.

`crates/robin_rs/src/shipping_mission.rs::required_dependencies` uses the installed shipping index, selected mission, campaign mission team, eligible gang reinforcements, normalized town/forest Robin profile, audio dependency indexes, and saved-world status. `ShippingMissionRef::files` is inside the compressed bitcode boot datadir. The shell has compressed boot bytes, not a standalone mission index. Custom replay packages and authenticated local Full-content loading also make a hard-coded default-demo plan inappropriate.

The existing `wasm_preload_shipping_file` hook is for verified local Full content. It requires an installed shipping manifest, validates references, decodes the file for validation, and rejects duplicates. Feeding a speculative download into it would add decode/copy work; it is not a generic in-flight network handoff.

## Stable historical network evidence

Reanalyzed the five optimized 16 Mbit/s runs retained under `/tmp/robin-startup-more/replay-matched/16-pair-*-candidate.json`. Those are the existing matched, idle-machine measurements, not new timings of main.

| Run | Admission accepted | Main WASM transfer ended | First mission request | Final preceding payload → mission request |
| --- | ---: | ---: | ---: | ---: |
| 1 | 1,015 ms | 5,673 ms | 6,159 ms | 334 ms |
| 2 | 920 ms | 5,581 ms | 6,066 ms | 336 ms |
| 3 | 955 ms | 5,610 ms | 6,089 ms | 329 ms |
| 4 | 948 ms | 5,607 ms | 6,095 ms | 337 ms |
| 5 | 934 ms | 5,590 ms | 6,077 ms | 337 ms |

The interval between admission and mission requests contains roughly **338–352 ms of unused payload capacity**, calculated as elapsed time minus recorded chunk bytes divided by 2,000 bytes/ms. A chunk straddling admission and scheduler timing add small boundary error. This is a scheduling estimate, not a measured startup gain or a universal upper bound: a new implementation could also alter CPU overlap.

Most of the apparent 4–5 second overlap window is already transferring required boot/runtime/interface bytes. Existing interface assets occupy about 149–152 ms after WASM transfer. Pure scheduling without reducing bytes therefore targets approximately **0.35 seconds**, not the full runtime startup interval.

## Browser oracle experiment

To test the hypothesis before introducing a new content index or runtime API, copied the retained production site and injected a research-only external script. Its 54 exact required mission/terrain URLs came from the already validated Leicester fixture trace. It used eight concurrent transfers and an in-memory promise handoff to `window.fetch`, publishing every promise before transfers began. All runtime requests consumed these same responses: **zero duplicate URLs** and no extra mission payload. This oracle deliberately omits discovery costs and is not safe as a production dependency planner.

The second variant waited for the `robin_bg.wasm` resource-completion entry instead of the admission-accepted mark. The original worker admission and exact-byte replay installation stayed in place in both variants.

| Sequential diagnostic run | WASM transfer end | First present returned | Pre-bootstrap payload |
| --- | ---: | ---: | ---: |
| Admission-triggered oracle | 15.656 s | 22.527 s | 31,097,399 B |
| Unchanged site | 5.588 s | 18.362 s | 31,095,658 B |
| Post-WASM-transfer oracle | 5.630 s | 17.364 s | 31,097,401 B |
| Unchanged site | 5.599 s | 17.601 s | 31,095,658 B |

These were bounded diagnostic samples while other background work was active. The two unchanged samples differ by 761 ms, so **do not report a reliable 1-second improvement** from the post-WASM oracle or replace the established 17.137-second headline. Its direction supports investigating the smaller initialization overlap. Admission-triggered bulk fetching clearly delayed the required WASM transfer by about 10 seconds; it fetched all mission pixels before the runtime could decode them.

Every run reached active, unpaused replay frame 1/306 on Leicester with no browser error; none was an EOF correctness test. Screenshots and full request/log records are retained. The additional ~1.7 KB is the research script and changed shell; game assets, WASM, admission, replay, and corpus remained the matching existing artifacts.

## Decision and next safe implementation boundary

Reject bulk mission fetching during the WASM transfer. No production change is justified by that experiment.

TODO: investigate moving authoritative replay launch/dependency planning before GPU/window initialization, then hand the exact in-flight downloads into normal mission loading. `bin/robin.rs::wasm_main` currently initializes the shipping datadir/profiles before `window::run_with_game`, while `main_entry/run.rs` takes and prepares the pending replay after the window exists. Moving that boundary would target the remaining ~0.35-second gap without parsing bitcode in the shell or introducing a second dependency index. It needs ownership/lifecycle work: await isolated admission, bind the exact replay/build and installed corpus, handle custom and saved-world launch preparation, cancel stale work, and reuse each response once. Keep decoding and the existing worker pool overlap intact.

A separate generated mission index is another option, but it must be bound to the exact corpus and reproduce the relevant dependency rules. Measure its bytes, discovery latency, and maintenance cost before adding another shipping format solely for this scheduling improvement.

## Reproduction and retained evidence

Artifacts: `/tmp/robin-early-fetch-next/` contains `oracle-site/`, `late-oracle-site/`, their readable `assets/research-prefetch.js`, `oracle.json` with exact URLs, four run JSON/PNG pairs, `summary.json`, and `idle-analysis.json`.

Each browser run used:

```sh
node scripts/wasm_production_startup_chrome.mjs \
  --pkg /tmp/robin-startup-more/final-replay-pkg \
  --datadir /tmp/robin-startup-more/corpus-trimmed \
  --core /tmp/robin-startup-more/baseline-core \
  --site /tmp/robin-early-fetch-next/oracle-site \
  --replay /tmp/robin-startup-more/replay-fixture/leicester-300-c7244b5ddca8.rhrec \
  --http-wasm-br /tmp/robin-startup-more/wasm/final-game-http.br \
  --http-admission-br /tmp/robin-startup-more/wasm/final-admission-http.br \
  --query wasm-threads=4 --query wasm-log=info --mbit 16 \
  --output /tmp/robin-early-fetch-next/oracle-1
```

Substitute the unchanged `final-site` or `late-oracle-site` and a unique output path for the other modes. The harness validates both compressed WASM fixtures against the supplied package. Its 16 Mbit/s limiter is shared across every response, including worker fetches; no RTT, packet loss, or TCP overhead is simulated. First present is submission-side, not physical display presentation. This is a documentation-only change; native compilation/tests do not validate these browser scheduling experiments and were not rerun.
