# Batch-20 sweep manifest (prepared, NOT yet executed)

`manifest.snapshot` = 985 traces:
- **204 failing** — every non-EOF trace from the batch-19 sweep at frozen runner 07872e94
  (`../batch19-head07872e94-nestedd7792d55-preflight/remaining-failures-204.snapshot`); all 204
  confirmed still present in the current 4,712-trace universe.
- **781 regression guard** (`regression.sample`) — currently-PASSING traces, deliberately included.
  The user's standing directive is failures-only sweeps until 100%, but three batch-20 merges rewrote
  shared core paths and a failures-only manifest is structurally blind to regressions they could cause:
  `198f7a6da` (465 lines of sight_obstacle.rs, IsReachableImpact semantics, used well beyond
  projectiles), `f0c371b84` (Think recursion depth for ALL AI — revives two previously-dead
  depth-gated behaviours), `906cd1cf3` (path-request scheduling for every actor).
  Composition: all 441 passing `15-no-input` traces (short, ~375 frames, broadest save coverage per
  CPU-minute) plus every 12th passing trace of each longer corpus (177 schema12, 106 schema14, 57 30s).
  Any status != 0 in this subset is a REGRESSION and must block the runner freeze.

Universe built by `scripts/build_parity_full_snapshot.sh` → `universe.snapshot` (4,712);
`passing.pool` = universe minus the 204 failures (4,508).

## To execute
1. Build the runner from the batch-20 HEAD: `cargo build --release --example original_parity_replay`
   (no timeout; never pipe cargo output through a filter).
2. Freeze it: copy to `<audit>/runner-rust/original_parity_replay-<shortsha>`, chmod 0555, record sha256.
3. Shard: `scripts/run_parity_release_sweep.sh /home/phire/robinhood <audit> <runner> SHARD SHARDS`
   with `PARITY_SWEEP_CONCURRENCY` set; status 0 = pass, 1 = state divergence, 101 = RNG divergence,
   124 = timeout (900s cap). Key = trace path with every `/` replaced by `__`.
4. Do NOT launch while many fix agents are running — replays are CPU-bound and will skew nothing but
   will take far longer; also check disk (each agent worktree is 6-17 GB).
