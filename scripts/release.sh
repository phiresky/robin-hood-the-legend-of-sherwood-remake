#!/usr/bin/env bash
# One-command release: leaderboard service to the VPS, then the web game to
# Cloudflare. Usage:
#   scripts/release.sh [--server-only|--web-only] [--rebuild-datadir] [--dry-run] [--ssh-config PATH]
#
# --dry-run prints every side-effecting command as "[dry-run] ..." without
# running it. Read-only checks (git state, VPS `current`, Worker versions, live
# latest.json and datadir header) still run and are printed as "check: ...".
# Web releases build on the staging directory `$staging/live` points at (the
# last deployed upload set) and move that pointer after a verified release.
set -euo pipefail

do_server=1 do_web=1 rebuild_datadir=0 dry_run=0 ssh_config=""
while (($#)); do
    case "$1" in
        --server-only) do_web=0 ;;
        --web-only) do_server=0 ;;
        --rebuild-datadir) rebuild_datadir=1 ;;
        --dry-run) dry_run=1 ;;
        --ssh-config) ssh_config=${2:?--ssh-config needs a path}; shift ;;
        *) sed -n '4p' "$0" >&2; exit 2 ;;
    esac
    shift
done
((do_server || do_web)) || { sed -n '4p' "$0" >&2; exit 2; }

repo=$(cd "$(dirname "$0")/.." && pwd -P)
cd "$repo"
main_repo=$(dirname "$(git rev-parse --path-format=absolute --git-common-dir)")
ssh_config=${ssh_config:-$main_repo/tmp/ssh_deploy_config}
host=robinhood-vps
site=https://robinhood.phiresky.xyz
image=robin-release-bookworm:nightly-2026-08-25
toolchain=${ROBIN_RELEASE_TOOLCHAIN:-$HOME/.local/share/robin_hood/deployment-toolchain}
staging=${ROBIN_RELEASE_STAGING:-$HOME/.local/share/robin_hood/deployment-staging}
datadir_source=${ROBIN_RELEASE_DATADIR_SOURCE:-$main_repo/datadirs/demo_leicester_ecoste}
codec=crates/robin_assets/src/shipping_datadir/codec.rs
wrangler=$repo/wasm-www/node_modules/.bin/wrangler

log_dir=$HOME/.local/share/robin_hood/release-logs
mkdir -p "$log_dir"
log=$log_dir/$(date -u +%Y%m%dT%H%M%SZ).log
exec > >(tee -a "$log") 2>&1

step() { printf '\n==> %s\n' "$*"; }
note() { printf '    %s\n' "$*"; }
die() { printf 'release: %s\n' "$*" >&2; exit 1; }
# Side-effecting command: always printed, executed unless --dry-run.
run() {
    if ((dry_run)); then printf '[dry-run] '; else printf '+ '; fi
    printf '%q ' "$@"
    printf '\n'
    if ((dry_run)); then return 0; fi
    "$@"
}
# Read-only probe that may fail in a dry run without network or credentials.
probe_failed() { ((dry_run)) || die "$1"; note "check failed ($1); dry run continues"; }

rollback_hint=""
on_exit() {
    local status=$?
    if ((status != 0)); then
        [[ -z $rollback_hint ]] || printf '\nrelease FAILED (exit %d). Rollback:\n%s\n' "$status" "$rollback_hint" >&2
        printf 'log: %s\n' "$log" >&2
    fi
}
trap on_exit EXIT

step "checks"
[[ -z "$(git status --porcelain --untracked-files=no)" ]] || die "working tree has uncommitted changes; commit first"
commit=$(git rev-parse HEAD)
[[ $commit == "$(git rev-parse refs/heads/main)" ]] || die "HEAD $commit is not the tip of main"
short=${commit:0:12}
note "commit $commit, log $log"
if ((dry_run)); then note "DRY RUN: [dry-run] commands are not executed"; fi

release_server() {
    local root='~/.local/opt/robin-highscores' build_tree=$main_repo/.worktrees/release-build
    local name=robin-highscores-$commit previous out tarball sha
    [[ -f $ssh_config ]] || die "missing ssh config $ssh_config (pass --ssh-config)"
    step "server: build release tarball in Debian 12"
    note "check: live service release on $host"
    previous=$(ssh -F "$ssh_config" "$host" "basename \"\$(readlink $root/current)\"") ||
        probe_failed "cannot read $root/current on $host"
    note "live service release: ${previous:-<unknown>}"
    if [[ -e $build_tree ]]; then
        run git -C "$build_tree" checkout --detach "$commit"
    else
        run git worktree add --detach "$build_tree" "$commit"
    fi
    docker image inspect "$image" >/dev/null 2>&1 ||
        run docker build -t "$image" crates/robin_highscores/ops/release-image
    out=$build_tree/target/release-tarballs
    # The target dir stays inside the dedicated build worktree, so Debian 12
    # artifacts never mix with host builds. CARGO_HOME is per container.
    run docker run --rm -u "$(id -u):$(id -g)" -v "$main_repo:$main_repo" -w "$build_tree" \
        -e HOME=/tmp/buildhome -e CARGO_HOME=/tmp/buildhome/cargo \
        -e CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=cc "$image" bash -c \
        'set -e; mkdir -p "$CARGO_HOME" "$1"; cp -r /opt/cargo/bin "$CARGO_HOME/"; git config --global --add safe.directory "*"; crates/robin_highscores/ops/build-release.sh "$1"' \
        _ "$out"
    tarball=$out/$name.tar.zst
    if ((dry_run)); then sha="<sha256>"; else sha=$(sha256sum "$tarball" | cut -d' ' -f1); fi

    step "server: upload and deploy on $host"
    rollback_hint="  deploy.sh already restores \`current\` when one of its steps fails; read its output
  (after a migration it leaves services stopped and names the DB snapshot to restore).
  Otherwise: ssh -F $ssh_config $host '$root/current/ops/rollback.sh ${previous:-<previous-commit>}'"
    run ssh -F "$ssh_config" "$host" 'mkdir -p ~/releases-incoming'
    run scp -F "$ssh_config" "$tarball" "$host:releases-incoming/"
    run ssh -F "$ssh_config" "$host" "set -e; cd ~/releases-incoming; echo '$sha  $name.tar.zst' | sha256sum -c -; tar --zstd -xOf $name.tar.zst $name/ops/deploy.sh >deploy-$commit.sh; bash deploy-$commit.sh ~/releases-incoming/$name.tar.zst"
    step "server: verify"
    run ssh -F "$ssh_config" "$host" 'curl -fsS http://127.0.0.1:8787/readyz'
    run curl -fsS -o /dev/null "$site/api/v1/leaderboard-metadata"
    rollback_hint=""
    note "server release $name is live"
}

# Live 100% version of the Worker in wasm-www/deploy/wrangler-$1.json.
worker_version() {
    (cd wasm-www && "$wrangler" deployments list --config "deploy/wrangler-$1.json" --json) |
        jq -er 'last.versions | max_by(.percentage).version_id'
}
# Hex of the first 12 decoded bytes (8-byte magic + u32 LE version) at URL $1.
# Download to a file first: piping curl into `head -c 12` makes curl report a
# spurious write failure when the reader closes early. A real fetch error still
# returns non-zero and yields an empty header, which the caller rejects.
datadir_header() {
    local tmp
    tmp=$(mktemp) || return 1
    if ! curl -fsS -o "$tmp" "$1"; then
        rm -f "$tmp"
        return 1
    fi
    (set +o pipefail; zstd -dc <"$tmp" 2>/dev/null | head -c 12 | od -An -tx1 | tr -d ' \n')
    rm -f "$tmp"
}
# Replace a checkout upload directory with a staged one.
stage_checkout() { run rm -rf "$2"; run cp -RH "$1" "$2"; run chmod -R u+w "$2"; }

release_web() {
    local prior=$staging/live stage=$staging/release-$short latest demo_url live_header code_magic code_version code_header
    local rebuild=$rebuild_datadir worker full binding datadir_version
    local -A before=()
    step "web: checks"
    [[ -x $wrangler ]] || die "missing $wrangler; run pnpm --dir wasm-www install --frozen-lockfile"
    [[ -f $main_repo/.env ]] || die "missing $main_repo/.env (Cloudflare credentials)"
    set -a; . "$main_repo/.env"; set +a
    : "${CLOUDFLARE_ACCOUNT_ID:?}" "${CLOUDFLARE_ZONE_ID:?}" "${CLOUDFLARE_API_TOKEN:?}"
    export PATH=$toolchain/bin:$toolchain/node-v24.19.0-linux-x64/bin:$toolchain/wasm-tools-132-1.0.41/binaryen/bin:$toolchain/wasm-tools-132-1.0.41/wabt/bin:$PATH
    export ROBINHOOD_WASM_BINDGEN=$toolchain/wasm-bindgen-0.2.128/bin/wasm-bindgen
    [[ -x $ROBINHOOD_WASM_BINDGEN ]] || die "missing $ROBINHOOD_WASM_BINDGEN"
    for binding in runtime-dist datadir-dist public-dist datadir-authority.json datadir-deployment.json; do
        [[ -e $prior/$binding ]] || die "$prior/$binding missing; point $prior at the last deployed staging directory"
    done
    prior=$(realpath "$prior")
    [[ ! -e $stage ]] || die "$stage already exists (remove it to restage $short)"

    code_magic=$(sed -n 's/.*SHIPPING_DATADIR_MAGIC: \[u8; 8\] = \*b"\([A-Z0-9]*\)";.*/\1/p' "$codec")
    code_version=$(sed -n 's/.*pub const SHIPPING_DATADIR_VERSION: u32 = \([0-9]*\);.*/\1/p' "$codec")
    [[ ${#code_magic} == 8 && -n $code_version ]] || die "cannot parse datadir magic/version from $codec"
    code_header=$(printf '%s' "$code_magic" | od -An -tx1 | tr -d ' \n')$(printf '%02x%02x%02x%02x' \
        $((code_version & 255)) $((code_version >> 8 & 255)) $((code_version >> 16 & 255)) $((code_version >> 24)))
    note "check: $site/wasm/latest.json and its Demo datadir header"
    latest=$(curl -fsS "$site/wasm/latest.json") || die "cannot fetch $site/wasm/latest.json"
    demo_url=$(jq -er .multiplayerContent.demo.url <<<"$latest")
    live_header=$(datadir_header "$demo_url")
    [[ -n $live_header ]] || die "cannot download/decode $demo_url"
    note "live build $(jq -r .short <<<"$latest"), datadir $demo_url header $live_header; code $code_magic v$code_version $code_header"
    if [[ $live_header != "$code_header" ]]; then
        note "live datadir format differs from this commit: building a new datadir generation"
        rebuild=1
    fi
    note "check: current Worker versions (rollback targets)"
    for worker in datadir runtime signer public; do
        before[$worker]=$(worker_version "$worker") || probe_failed "cannot list $worker Worker deployments"
        note "$worker: ${before[$worker]:=<unknown>}"
    done
    rollback_hint="  Roll back each Worker this run already deployed (order: public, signer, runtime, datadir):"
    for worker in public signer runtime datadir; do
        rollback_hint+=$'\n'"  (cd wasm-www && node_modules/.bin/wrangler rollback --config deploy/wrangler-$worker.json ${before[$worker]})"
    done

    run mkdir -p "$stage"
    datadir_version=${before[datadir]}
    binding=$prior/datadir-deployment.json
    if ((rebuild)); then
        step "web: build and deploy a new datadir generation"
        command -v cjxl >/dev/null || die "cjxl not on PATH (put the static cjxl binary in $toolchain/bin)"
        # The converter itself verifies the loaded version string and that
        # ffmpeg resolved exactly this file (docs/COMPRESSION.md, 2026-09-14).
        export ROBIN_LIBOPUS_DIR=$toolchain/libopus-1.6.1/lib
        [[ -e $ROBIN_LIBOPUS_DIR/libopus.so.0 ]] || die "missing $ROBIN_LIBOPUS_DIR/libopus.so.0 (build libopus 1.6.1 into $toolchain/libopus-1.6.1)"
        export ROBIN_LOSSLESS_MUSIC_DIR=$main_repo/datadirs/music-rhmods-lossless
        [[ -d $ROBIN_LOSSLESS_MUSIC_DIR ]] || die "missing lossless music WAV directory $ROBIN_LOSSLESS_MUSIC_DIR (mapping: convert_datadir/lossless_music_mapping.json)"
        run scripts/build_web_shipping_datadir.sh "$datadir_source" "$stage/demo-converter-output"
        run node wasm-www/scripts/assemble-datadir-corpus.mjs --update "$prior/datadir-dist" "$stage/demo-converter-output" "$stage/datadir-dist"
        run node wasm-www/scripts/datadir-release-authority.mjs author "$stage/datadir-dist" "$commit" \
            "$(sha256sum Cargo.lock | cut -d' ' -f1)" "$stage/datadir-inventory.json" "$stage/datadir-authority.json"
        stage_checkout "$stage/datadir-dist" wasm-www/datadir-dist
        run cp "$stage/datadir-authority.json" target/datadir-authority.json
        run rm -f target/datadir-deployment.json
        run wasm-www/scripts/deploy-cloudflare.sh --datadir-only
        run cp target/datadir-deployment.json "$stage/datadir-deployment.json"
        binding=$stage/datadir-deployment.json
        if ((dry_run)); then datadir_version="<new datadir version>"; else datadir_version=$(jq -er .worker_version_id "$binding"); fi
    else
        step "web: datadir unchanged, reusing $(realpath "$prior/datadir-dist")"
        run ln -s "$(realpath "$prior/datadir-dist")" "$stage/datadir-dist"
        run cp "$prior/datadir-authority.json" "$prior/datadir-deployment.json" "$stage/"
    fi

    step "web: stage runtime $short and assemble the complete runtime corpus"
    demo_field() { if [[ -f $binding ]]; then jq -er ".demo.$1" "$binding"; else printf '<%s>' "$1"; fi; }
    full=$(jq -r '.multiplayerContent.full.manifestSha256 // empty' <<<"$latest")
    run env DEMO_ASSET_SHA256="$(demo_field datadir_sha256)" DEMO_ASSET_BYTE_LENGTH="$(demo_field datadir_byte_length)" \
        DEMO_CONTENT_IDENTITY_SHA256="$(demo_field native_content_sha256)" FULL_CONTENT_MANIFEST_SHA256="$full" \
        node wasm-www/scripts/stage-runtime-addition.mjs --root "$stage/runtime-addition" --bindgen "$ROBINHOOD_WASM_BINDGEN"
    run node wasm-www/scripts/assemble-runtime-corpus.mjs --update "$prior/runtime-dist" "$stage/runtime-addition" \
        "$stage/datadir-authority.json" "$stage/datadir-deployment.json" "$stage/runtime-dist" "$prior/datadir-authority.json"
    stage_checkout "$stage/runtime-dist" wasm-www/runtime-dist
    run cp "$stage/datadir-authority.json" target/datadir-authority.json

    step "web: deploy runtime, signer and public site"
    run env ROBINHOOD_DATADIR_VERSION_ID="$datadir_version" ROBINHOOD_PUBLIC_RETAIN="$prior/public-dist" \
        wasm-www/scripts/deploy-cloudflare.sh --runtime
    run cp -R wasm-www/dist "$stage/public-dist"

    step "web: verify live"
    if ((dry_run)); then
        note "[dry-run] would check latest.json names $short, its datadir sha256 and header $code_header"
    else
        latest=$(curl -fsS "$site/wasm/latest.json")
        [[ $(jq -er .short <<<"$latest") == "$short" ]] || die "live latest.json does not name $short"
        demo_url=$(jq -er .multiplayerContent.demo.url <<<"$latest")
        [[ $(curl -fsS "$demo_url" | sha256sum | cut -d' ' -f1) == "$(jq -er .multiplayerContent.demo.sha256 <<<"$latest")" ]] ||
            die "live $demo_url sha256 does not match latest.json"
        [[ $(datadir_header "$demo_url") == "$code_header" ]] || die "live $demo_url does not decode to $code_magic v$code_version"
        note "latest.json -> $short; $demo_url ok"
    fi
    run ln -sfn "$stage" "$staging/live"
    rollback_hint=""
}

if ((do_server)); then release_server; fi
if ((do_web)); then release_web; fi
step "release $short done"
