#!/usr/bin/env bash
# Stub test for scripts/release.sh. Every external tool (git, docker, ssh, scp,
# curl, zstd, node, wrangler, avifenc, avifdec) is a fake; nothing touches the network, the
# VPS or Cloudflare.
set -euo pipefail

source_root=$(cd "$(dirname "$0")/.." && pwd -P)
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT
fixture=$work/repo
bin=$work/bin
export HOME=$work/home FAKE_CALLS=$work/calls FAKE_WORK=$work
export FAKE_MAIN=1111111111111111111111111111111111111111
mkdir -p "$fixture/scripts" "$fixture/crates/robin_assets/src/shipping_datadir" \
    "$fixture/wasm-www/node_modules/.bin" "$bin" "$work/main/tmp"
cp "$source_root/scripts/release.sh" "$fixture/scripts/"
cat >"$fixture/crates/robin_assets/src/shipping_datadir/codec.rs" <<'EOF'
pub(super) const SHIPPING_DATADIR_MAGIC: [u8; 8] = *b"RHDDNA16";
pub const SHIPPING_DATADIR_VERSION: u32 = 16;
EOF
touch "$fixture/Cargo.lock" "$work/main/tmp/ssh_deploy_config"
printf 'CLOUDFLARE_ACCOUNT_ID=fake\nCLOUDFLARE_ZONE_ID=fake\nCLOUDFLARE_API_TOKEN=fake\n' >"$work/main/.env"
toolchain=$HOME/.local/share/robin_hood/deployment-toolchain
staging=$HOME/.local/share/robin_hood/deployment-staging
mkdir -p "$toolchain/wasm-bindgen-0.2.128/bin" "$staging/prior/runtime-dist" \
    "$staging/prior/datadir-dist" "$staging/prior/public-dist"
echo '{}' >"$staging/prior/datadir-authority.json"
printf '{"demo":{"datadir_sha256":"%064d","datadir_byte_length":12,"native_content_sha256":"%064d"},"worker_version_id":"datadir-old"}\n' \
    0 0 >"$staging/prior/datadir-deployment.json"
ln -s prior "$staging/live"

fake() {
    local path=$1
    {
        printf '#!/usr/bin/env bash\nprintf "%%s %%s\\n" "${0##*/}" "$*" >>"$FAKE_CALLS"\n'
        cat
    } >"$path"
    chmod +x "$path"
}
fake "$toolchain/wasm-bindgen-0.2.128/bin/wasm-bindgen" </dev/null
# Datadir rebuilds require opusenc on libopus 1.6.1 and the lossless music WAV
# directory; the real loader and mapping checks live in convert_datadir.
mkdir -p "$toolchain/opus-tools-0.2/bin" "$work/main/datadirs/music-rhmods-lossless"
fake "$toolchain/opus-tools-0.2/bin/opusenc" <<'EOF'
echo "opusenc opus-tools 0.2 (using ${FAKE_OPUSENC_LIBOPUS:-libopus 1.6.1})"
EOF
fake "$bin/git" <<'EOF'
case "$*" in
    "status --porcelain --untracked-files=no") [[ -z ${FAKE_DIRTY:-} ]] || echo ' M crates/robin_rs/src/lib.rs' ;;
    "rev-parse --path-format=absolute --git-common-dir") echo "$FAKE_WORK/main/.git" ;;
    "rev-parse HEAD") echo "${FAKE_HEAD:-$FAKE_MAIN}" ;;
    "rev-parse refs/heads/main") echo "$FAKE_MAIN" ;;
esac
EOF
fake "$bin/docker" <<'EOF'
if [[ $1 == run ]]; then
    out=${!#}
    mkdir -p "$out"
    echo tarball >"$out/robin-highscores-$FAKE_MAIN.tar.zst"
fi
EOF
fake "$bin/ssh" <<'EOF'
case "${!#}" in
    *readlink*) echo 0000previous ;;
    *deploy-*.sh*) if [[ -n ${FAKE_DEPLOY_FAIL:-} ]]; then echo 'deploy: readyz failed' >&2; exit 1; fi ;;
esac
EOF
fake "$bin/scp" </dev/null
fake "$bin/node" </dev/null
for avif_tool in avifenc avifdec; do
    fake "$bin/$avif_tool" <<'EOF'
echo "Version: ${FAKE_AVIF_VERSION:-1.4.2 (aom [enc/dec]:3.15.0)}"
EOF
done
fake "$bin/zstd" <<'EOF'
cat
EOF
fake "$bin/curl" <<'EOF'
out=""
args=("$@")
for ((i = 0; i < ${#args[@]}; i++)); do
    [[ ${args[i]} == -o ]] && out=${args[i + 1]}
done
[[ -z $out || $out == /dev/null ]] || exec >"$out"
case "$*" in
    *latest.json*) echo '{"short":"000000000000","multiplayerContent":{"demo":{"url":"https://robinhood.phiresky.xyz/datadirs/demo.rhdata.zst","sha256":"x"},"full":null}}' ;;
    *demo.rhdata.zst*) printf '%s\x10\x00\x00\x00payload' "${FAKE_LIVE_MAGIC:-RHDDNA16}" ;;
esac
EOF
fake "$fixture/wasm-www/node_modules/.bin/wrangler" <<'EOF'
arguments="$*"
config=${arguments##*wrangler-}
printf '[{"versions":[{"version_id":"%s-old","percentage":100}]}]\n' "${config%%.json*}"
EOF

output=""
status=0
release() {
    : >"$FAKE_CALLS"
    status=0
    output=$(cd "$work" && PATH="$bin:$PATH" bash "$fixture/scripts/release.sh" "$@" 2>&1) || status=$?
}
fail() { printf 'FAIL: %s\n--- output ---\n%s\n--- calls ---\n%s\n' "$1" "$output" "$(cat "$FAKE_CALLS")" >&2; exit 1; }
expect_status() { if [[ $1 == 0 ]]; then ((status == 0)) || fail "exit $status"; else ((status != 0)) || fail "unexpected success"; fi; }
expect() { [[ $output == *"$1"* ]] || fail "missing: $1"; }
reject() { [[ $output != *"$1"* ]] || fail "unexpected: $1"; }
called() { grep -q "^$1" "$FAKE_CALLS" || fail "not called: $1"; }
not_called() { ! grep -q "^$1" "$FAKE_CALLS" || fail "called: $1"; }
in_order() {
    local rest=$output marker
    for marker; do
        [[ $rest == *"$marker"* ]] || fail "missing or out of order: $marker"
        rest=${rest#*"$marker"}
    done
}

# 1. Dry run of a full release: exact command sequence, no side effects.
release --dry-run
expect_status 0
in_order '==> checks' '[dry-run] git worktree add --detach' '[dry-run] docker run --rm' \
    'build-release.sh' "[dry-run] ssh -F $work/main/tmp/ssh_deploy_config robinhood-vps mkdir" \
    '[dry-run] scp -F' "robin-highscores-$FAKE_MAIN.tar.zst robinhood-vps:releases-incoming/" \
    'sha256sum\ -c' "deploy-$FAKE_MAIN.sh" '127.0.0.1:8787/readyz' \
    '[dry-run] curl -fsS -o /dev/null https://robinhood.phiresky.xyz/api/v1/leaderboard-metadata' \
    'runtime: runtime-old' 'web: datadir unchanged' '[dry-run] env DEMO_ASSET_SHA256=' \
    'stage-runtime-addition.mjs --root' 'assemble-runtime-corpus.mjs --update' \
    '[dry-run] env ROBINHOOD_DATADIR_VERSION_ID=datadir-old ROBINHOOD_PUBLIC_RETAIN=' \
    'deploy-cloudflare.sh --runtime' '[dry-run] ln -sfn' '==> release 111111111111 done'
reject build_web_shipping_datadir.sh
reject 'release FAILED'
not_called scp
not_called node
not_called 'docker run'
not_called 'ssh .*deploy-'
called 'wrangler deployments list'
if grep '^wrangler' "$FAKE_CALLS" | grep -qv 'deployments list'; then fail 'wrangler used beyond deployments list'; fi
compgen -G "$HOME/.local/share/robin_hood/release-logs/*.log" >/dev/null || fail 'no release log written'

# 2. Live datadir header differs from the code: a new generation is built first.
FAKE_LIVE_MAGIC=RHDDNA15 release --web-only --dry-run
expect_status 0
in_order 'building a new datadir generation' 'build_web_shipping_datadir.sh' \
    'assemble-datadir-corpus.mjs --update' 'datadir-release-authority.mjs author' \
    'deploy-cloudflare.sh --datadir-only' 'stage-runtime-addition.mjs' \
    'ROBINHOOD_DATADIR_VERSION_ID=\<new\ datadir\ version\>' 'deploy-cloudflare.sh --runtime'
reject 'docker run'

# 3. --rebuild-datadir forces the rebuild even when the header matches.
release --web-only --rebuild-datadir --dry-run
expect_status 0
expect 'build_web_shipping_datadir.sh'
reject 'building a new datadir generation'

# 3b. A rebuild without opusenc, or with one on another libopus, is refused
# before converting.
mv "$toolchain/opus-tools-0.2" "$toolchain/opus-tools-moved"
release --web-only --rebuild-datadir --dry-run
mv "$toolchain/opus-tools-moved" "$toolchain/opus-tools-0.2"
expect_status 1
expect 'opus-tools-0.2/bin/opusenc'
reject 'build_web_shipping_datadir.sh'
FAKE_OPUSENC_LIBOPUS='libopus 1.5.2' release --web-only --rebuild-datadir --dry-run
expect_status 1
expect 'does not report libopus 1.6.1'
reject 'build_web_shipping_datadir.sh'

# 3c. A rebuild with an avifenc/avifdec other than the pinned libavif 1.4.2 on
# libaom 3.15.0 is refused before converting (AVIF output depends on both).
FAKE_AVIF_VERSION='1.4.1 (aom [enc/dec]:3.14.0)' release --web-only --rebuild-datadir --dry-run
expect_status 1
expect 'is not the pinned libavif 1.4.2 / libaom 3.15.0 build'
reject 'build_web_shipping_datadir.sh'

# 4. Dirty tree and a HEAD that is not main are refused before any work.
FAKE_DIRTY=1 release --dry-run
expect_status 1
expect 'uncommitted changes'
not_called docker
FAKE_HEAD=2222222222222222222222222222222222222222 release --dry-run
expect_status 1
expect 'is not the tip of main'
not_called docker

# 5. A failed server deploy stops the release and prints the rollback.
FAKE_DEPLOY_FAIL=1 release
expect_status 1
called scp
expect 'release FAILED'
expect "rollback.sh 0000previous"
not_called 'curl -fsS -o /dev/null'
reject '==> web:'

# 6. A failed web stage prints the recorded Worker versions to roll back to.
release --web-only
expect_status 1
expect 'release FAILED'
expect 'wrangler rollback --config deploy/wrangler-runtime.json runtime-old'
expect 'wrangler rollback --config deploy/wrangler-public.json public-old'

# 7. Usage errors.
release --server-only --web-only
expect_status 1

echo 'test_release.sh: all checks passed'
