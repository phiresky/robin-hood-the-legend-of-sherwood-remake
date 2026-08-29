## Rust port notes

- Never defensively return fake data - e.g. if an object is missing that is reqired and you need to return a bool depending on that object, don't just return false, return an error or panic - or at least log a warning.

- Add todos notes when stuff is unclear or incomplete, or suboptimal to be cleaned up later.

- feel free to add crates as dependencies - better use battle-tested code rather than build our own implementations

- Always reference `./original-code` if available when implementing, debugging, or checking behavior against the original game.

- **NEVER use `git stash`.** Multiple worktree agents are often running concurrently, each with their own working tree changes. `git stash` is scoped to the repository, not the worktree, so stashing in one worktree and popping in another (or simply having multiple stashes in flight) will restore files on top of each other and destroy work. If you need to temporarily set aside uncommitted changes, commit them to a scratch branch instead, or just leave them in the working tree and work around them.

## Worktree agent instructions

When working in a worktree (`.worktrees/<name>/`), you are in a **full copy of the repo**. All files are here. Do NOT try to access files via `../../` or the parent repo path. Work entirely within the current working directory.

- The Cargo workspace is at the repo root. Run `cargo test` to build and test.
- **NEVER change Cargo's target directory.** Do not set `CARGO_TARGET_DIR`, pass `--target-dir`, or add Cargo configuration that redirects it. Cargo's target directory must remain at `target/` inside each worktree so build artifacts are isolated per worktree.
- When done, **commit your changes** with `git add` and `git commit`. The branch will be merged into `rust` later.
- The Rust implementation is functionally runnable. Net-new features go in `docs/NEW_FEATURES.md`.
- Only modify the files specified in your task. Don't touch unrelated files.
- Use `serde::{Serialize, Deserialize}` for all new structs. No legacy binary serialization.
- Register new modules in `crates/robin_rs/src/lib.rs` with `pub mod <name>;`
- **Add crate dependencies** if it makes sense (e.g. `bitflags`, `anyhow`, `thiserror`). Edit `crates/robin_rs/Cargo.toml` to add them. Prefer battle-tested crates over hand-rolled implementations.

## Building

- Run `cargo test` for tests. Run `cargo fmt` before committing.
- **Do NOT run `cargo clippy` as part of normal worktree work.** Clippy is expensive and clippy fixes are batched into dedicated cleanup sessions, not interleaved with feature/parity work. If you have an instinct to "make sure clippy passes" before committing — don't. Just `cargo build --bin robin` + `cargo fmt` and commit.
- **NEVER pipe cargo output through `head`, `tail`, `grep`, or any other filter, and don't redirect to a shared path like `/tmp/cargo.log` (multiple worktree agents run in parallel and would clobber each other).** Cargo runs are expensive — re-running them to recover hidden errors costs more than the few extra lines of context. Just run cargo plain; the Bash tool harness automatically spills long output to a per-call temp file you can grep, so nothing is lost by not filtering up front.
- **Always build and run in separate steps.** `cargo build` can take arbitrarily long (especially on clean builds or after dep changes) and must run without a timeout, while `cargo run` needs a timeout so it doesn't hang forever on a running game. Do `cargo build --bin robin` first (no timeout), then `cargo run --bin robin ...` with a timeout. Never combine them into a single `cargo run` call with a tight timeout — the build will get killed.
- Run with `RUST_LOG=debug ROBINHOOD_DATA_DIR=../../../datadirs/demo_leicester_ecoste cargo run --bin robin` — this is the default datadir to use for development/testing. The datadir is in the repo root, this is the only place you should use `../..` if necessary. Other options: `datadirs/fullgame_gog`, `datadirs/fullgame_linux`, `datadirs/fullgame_shipping`, `datadirs/demo_leicester_linux`, `datadirs/demo_lincoln`, `datadirs/demo_shipping`.
- Set `RUST_LOG=debug` (or `RUST_LOG=robin_rs=debug`, `RUST_LOG=trace`, etc.) to control logging verbosity via the `env_logger` / `tracing` setup.
- The main game binary is `robin` (defined in `crates/robin_rs/src/bin/robin.rs`).
- Developer tools are *examples*, not bins — run them with `cargo run --example <name> -- <args>`. Available: `batch_run`, `count_quads`, `cpf_to_json`, `datadir_breakdown`, `disasm_scb`, `dump_res`, `dump_save`, `jxl_map_roundtrip`, `pak_res_roundtrip`, `run_script`, `sprite_size_bench`, `verify_rollback`. They are only built on demand, which keeps `cargo build` fast.
- For wasm/emscripten builds (served from `wasm-www/`), see `README.md` — uses the `wasm-dev` / `wasm-release` profiles with flags configured in `.cargo/config.toml`.

## Searching

- `ast-grep` is installed. Prefer it over grep, sed, awk whenever the query is structural (e.g. `ast-grep run --pattern 'panic!($$$)' --lang rust crates/robin_rs/src/`).

## Replays

- The game records every session to `~/.local/share/robin_hood/replays/*.rhrec.jsonl`. Newest file is the most recent run.
- Replay a specific session with `--replay <path>` (see `feedback_use_replay` memory for why this is the default debug workflow).

## Repo structure

- `crates/robin_rs/` — main Rust crate (game engine, renderer, all game logic)
- `original-code/` — original game code/reference material; consult this if available for parity and behavior questions
- `assets/` — icons, font, other non-code assets
- `datadirs/` — symlinks to game data directories (demo, fullgame)
