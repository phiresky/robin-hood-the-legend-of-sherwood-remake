#!/usr/bin/env bash
# Exercise ops/deploy.sh and ops/rollback.sh against a temporary HOME with stub
# systemctl/curl on PATH and stub release binaries. Run directly or through
# `cargo test -p robin_highscores --test ops_scripts`.
set -euo pipefail

ops=$(realpath "$(dirname "$0")/..")
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

export HOME="$work/home"
root="$HOME/.local/opt/robin-highscores"
state="$HOME/.local/share/robin-highscores"
mkdir -p "$root/releases" "$state/backups" "$HOME/.config/robin-highscores" "$work/stubs" "$work/tarballs"
touch "$HOME/.config/robin-highscores/server.toml"

export STUB_LOG="$work/calls.log"
export STUB_DB="$work/db-schema"
echo 2 >"$STUB_DB"
: >"$STUB_LOG"

cat >"$work/stubs/systemctl" <<'EOF'
#!/usr/bin/env bash
echo "systemctl $*" >>"$STUB_LOG"
EOF
cat >"$work/stubs/curl" <<'EOF'
#!/usr/bin/env bash
echo "curl $*" >>"$STUB_LOG"
exit "${STUB_CURL_EXIT:-0}"
EOF
chmod +x "$work/stubs/systemctl" "$work/stubs/curl"
export PATH="$work/stubs:$PATH"

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

# make_release COMMIT SUPPORTED_SCHEMA -> tarball path
make_release() {
    local commit=$1 supported=$2
    local dir="$work/build/robin-highscores-$commit"
    rm -rf "$dir"
    mkdir -p "$dir/bin"
    printf '#!/bin/sh\n' >"$dir/bin/robin-highscores-server"
    printf '#!/bin/sh\n' >"$dir/bin/robin-highscores-worker"
    cat >"$dir/bin/robin-highscores-admin" <<EOF
#!/usr/bin/env bash
set -euo pipefail
[[ "\$1" == --config ]] && shift 2
echo "admin $commit \$*" >>"\$STUB_LOG"
case "\$1" in
    supported-schema-version) echo $supported ;;
    database-schema-version) cat "\$STUB_DB" ;;
    snapshot-db) [[ ! -e "\$2" ]] || exit 1; cp "\$STUB_DB" "\$2" ;;
    migrate)
        (( \$(<"\$STUB_DB") <= $supported )) || { echo "schema newer than binary" >&2; exit 1; }
        echo $supported >"\$STUB_DB" ;;
    *) exit 2 ;;
esac
EOF
    chmod +x "$dir"/bin/*
    echo "$commit" >"$dir/SOURCE_COMMIT"
    (cd "$dir" && sha256sum bin/* SOURCE_COMMIT >SHA256SUMS)
    tar -cf "$work/tarballs/$commit.tar" -C "$work/build" "robin-highscores-$commit"
    echo "$work/tarballs/$commit.tar"
}

current() { basename "$(readlink "$root/current")"; }

# Verifier release: oldest directory in releases/, just the static verifier.
authority_commit=0000000
mkdir -p "$root/releases/$authority_commit/bin"
printf '#!/bin/sh\n' >"$root/releases/$authority_commit/bin/robin-replay-verifier"
chmod +x "$root/releases/$authority_commit/bin/robin-replay-verifier"
echo "$authority_commit" >"$root/releases/$authority_commit/SOURCE_COMMIT"
touch -d '2020-01-01' "$root/releases/$authority_commit"
ln -s "$root/releases/$authority_commit" "$root/authority"

# --- missing authority symlink is refused -------------------------------------------
mv "$root/authority" "$root/authority.off"
if "$ops/deploy.sh" "$(make_release 1111111 2)" >"$work/out" 2>&1; then
    fail "deploy without authority symlink succeeded"
fi
grep -q "authority must be a symlink" "$work/out" || fail "missing authority message"
mv "$root/authority.off" "$root/authority"

# --- a verifier release without its verifier is refused -------------------------------
chmod -x "$root/releases/$authority_commit/bin/robin-replay-verifier"
: >"$STUB_LOG"
if "$ops/deploy.sh" "$work/tarballs/1111111.tar" >"$work/out" 2>&1; then
    fail "deploy with a broken verifier release succeeded"
fi
grep -q "has no executable bin/robin-replay-verifier" "$work/out" || { cat "$work/out"; fail "no verifier message"; }
! grep -q systemctl "$STUB_LOG" || fail "systemctl called for a broken verifier release"
chmod +x "$root/releases/$authority_commit/bin/robin-replay-verifier"

# --- success (first deploy) -----------------------------------------------------------
: >"$STUB_LOG"
"$ops/deploy.sh" "$work/tarballs/1111111.tar" >"$work/out" 2>&1 || { cat "$work/out"; fail "first deploy"; }
[[ "$(current)" == 1111111 ]] || fail "current is not 1111111"
grep -q "systemctl --user start robin-highscores.target" "$STUB_LOG" || fail "services not started"
grep -q "admin 1111111 snapshot-db $state/backups/pre-deploy-1111111-" "$STUB_LOG" || fail "no pre-deploy snapshot"
ls "$state"/backups/pre-deploy-1111111-*/highscores.sqlite3 >/dev/null || fail "snapshot file missing"

# --- corrupted tarball is refused before touching services ---------------------------
make_release 2222222 2 >/dev/null
echo tampered >>"$work/build/robin-highscores-2222222/bin/robin-highscores-server"
tar -cf "$work/tarballs/bad.tar" -C "$work/build" robin-highscores-2222222
: >"$STUB_LOG"
if "$ops/deploy.sh" "$work/tarballs/bad.tar" >"$work/out" 2>&1; then
    fail "tampered tarball deployed"
fi
grep -q "SHA256SUMS verification failed" "$work/out" || { cat "$work/out"; fail "no checksum message"; }
! grep -q systemctl "$STUB_LOG" || fail "systemctl called for a tampered tarball"
[[ "$(current)" == 1111111 ]] || fail "current changed after tampered tarball"

# --- health-check failure without migration: symlink restored, services restarted --
: >"$STUB_LOG"
if STUB_CURL_EXIT=22 "$ops/deploy.sh" "$(make_release 2222222 2)" >"$work/out" 2>&1; then
    fail "deploy with failing health check succeeded"
fi
[[ "$(current)" == 1111111 ]] || fail "current not restored after health failure"
grep -q "previous release restarted" "$work/out" || { cat "$work/out"; fail "no restart message"; }
[[ "$(tail -n 1 "$STUB_LOG")" == "systemctl --user start robin-highscores.target" ]] ||
    fail "previous release was not restarted last"

# --- failure after a migration ran: services left stopped with restore message ------
: >"$STUB_LOG"
if STUB_CURL_EXIT=22 "$ops/deploy.sh" "$(make_release 3333333 3)" >"$work/out" 2>&1; then
    fail "deploy with migration and failing health check succeeded"
fi
[[ "$(current)" == 1111111 ]] || fail "current not restored after migrated failure"
grep -q "Services are left STOPPED" "$work/out" || { cat "$work/out"; fail "no stopped message"; }
grep -q "copy $state/backups/pre-deploy-3333333-.*/highscores.sqlite3 over database_path" "$work/out" ||
    fail "no restore instruction"
[[ "$(tail -n 1 "$STUB_LOG")" == "systemctl --user stop robin-highscores-worker.service robin-highscores-api.service" ]] ||
    fail "services were not left stopped"
[[ "$(<"$STUB_DB")" == 3 ]] || fail "stub database was not migrated"

# --- rollback refuses a schema the target does not support ---------------------------
"$ops/deploy.sh" "$work/tarballs/3333333.tar" >"$work/out" 2>&1 || { cat "$work/out"; fail "redeploy 3333333"; }
[[ "$(current)" == 3333333 ]] || fail "3333333 not live"
: >"$STUB_LOG"
if "$ops/rollback.sh" 1111111 >"$work/out" 2>&1; then
    fail "rollback past a migration succeeded"
fi
grep -q "database schema is 3 but 1111111 requires 2" "$work/out" || { cat "$work/out"; fail "no schema refusal"; }
! grep -q systemctl "$STUB_LOG" || fail "rollback refusal touched services"
if "$ops/rollback.sh" "$authority_commit" >"$work/out" 2>&1; then
    fail "rollback to the verifier release succeeded"
fi
grep -q "verifier release" "$work/out" || { cat "$work/out"; fail "no verifier release refusal"; }

# --- refusing to replace the verifier release ------------------------------------------
if "$ops/deploy.sh" "$(make_release $authority_commit 3)" >"$work/out" 2>&1; then
    fail "deploy over the verifier release succeeded"
fi
grep -q "is the verifier release named by authority" "$work/out" || { cat "$work/out"; fail "no authority refusal"; }
[[ -x "$root/releases/$authority_commit/bin/robin-replay-verifier" ]] || fail "verifier release modified"

# --- prune keeps 3 service releases and never the verifier release ---------------------
for commit in 4444444 5555555 6666666; do
    "$ops/deploy.sh" "$(make_release $commit 3)" >"$work/out" 2>&1 || { cat "$work/out"; fail "deploy $commit"; }
done
[[ "$(current)" == 6666666 ]] || fail "current is not 6666666"
[[ -x "$root/releases/$authority_commit/bin/robin-replay-verifier" ]] ||
    fail "prune removed the verifier release"
remaining=$(cd "$root/releases" && ls -1d */ | tr -d / | sort | tr '\n' ' ')
[[ "$remaining" == "$authority_commit 4444444 5555555 6666666 " ]] || fail "unexpected releases after prune: $remaining"

# --- default rollback picks the previous service release ------------------------------
: >"$STUB_LOG"
"$ops/rollback.sh" >"$work/out" 2>&1 || { cat "$work/out"; fail "default rollback"; }
[[ "$(current)" == 5555555 ]] || fail "default rollback did not select 5555555"
grep -q "curl .*readyz" "$STUB_LOG" || fail "rollback skipped the health check"

echo "ops deploy/rollback tests passed"
