# Debian 12 (glibc 2.36) build image for leaderboard service releases; the VPS
# runs Debian 12, so binaries linked on a newer host glibc do not start there.
# scripts/release.sh builds it on demand as robin-release-bookworm:nightly-2026-08-25
# (keep the tag and toolchain in sync with rust-toolchain.toml).
FROM debian:bookworm
RUN apt-get update && apt-get install -y --no-install-recommends \
      build-essential pkg-config git curl ca-certificates zstd && rm -rf /var/lib/apt/lists/*
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:$PATH
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path --profile minimal --default-toolchain none \
 && rustup toolchain install nightly-2026-08-25 --profile minimal -c rustc-codegen-cranelift-preview -c rust-src -c rustfmt -t wasm32-unknown-unknown \
 && chmod -R a+rwX /opt/rustup /opt/cargo
