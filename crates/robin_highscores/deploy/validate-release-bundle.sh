#!/bin/sh
set -eu

# Read-only validation shared by upload review, deployment, and rollback.
# The expected commit and SHA256SUMS digest must arrive out of band; trusting
# values found only inside the candidate would not authenticate the bundle.

usage() {
    echo "usage: $0 ABSOLUTE_BUNDLE_DIR EXPECTED_40HEX_COMMIT EXPECTED_SHA256SUMS_SHA256 /proc/self/fd/MANIFESTCTL_FD" >&2
    exit 64
}

fail() {
    echo "release validation failed: $*" >&2
    exit 1
}

[ "$#" -eq 4 ] || usage
bundle_arg=$1
expected_commit=$2
expected_sums_sha256=$3
manifest_tool_fd=$4

case "$manifest_tool_fd" in
    /proc/self/fd/*)
        manifest_tool_number=${manifest_tool_fd#/proc/self/fd/}
        case "$manifest_tool_number" in ''|*[!0-9]*) fail "manifest-tool descriptor is not canonical" ;; esac
        ;;
    *) fail "manifest tool must be an explicit inherited descriptor" ;;
esac
[ -f "$manifest_tool_fd" ] && [ "$(stat -Lc %u -- "$manifest_tool_fd")" -eq "$(id -u)" ] &&
    [ "$(stat -Lc %h -- "$manifest_tool_fd")" -eq 1 ] &&
    [ "$(stat -Lc %a -- "$manifest_tool_fd")" = 550 ] ||
    fail "manifest-tool descriptor has unsafe owner, type, links, or mode"

case "$bundle_arg" in
    /*) ;;
    *) fail "bundle path must be absolute" ;;
esac
case "$expected_commit" in
    *[!0-9a-f]*|'') fail "expected commit must be lowercase hexadecimal" ;;
esac
[ "${#expected_commit}" -eq 40 ] || fail "expected commit must contain exactly 40 hexadecimal digits"
case "$expected_sums_sha256" in
    *[!0-9a-f]*|'') fail "expected SHA256SUMS digest must be lowercase hexadecimal" ;;
esac
[ "${#expected_sums_sha256}" -eq 64 ] || fail "expected SHA256SUMS digest must contain exactly 64 hexadecimal digits"

[ -d "$bundle_arg" ] || fail "bundle is not a directory"
[ ! -L "$bundle_arg" ] || fail "bundle directory must not be a symlink"
bundle=$(realpath -e -- "$bundle_arg")
[ -d "$bundle" ] && [ ! -L "$bundle" ] || fail "bundle did not resolve to a real directory"

bad_path=0
while IFS= read -r path; do
    relative=${path#"$bundle"/}
    case "$relative" in
        ''|*[!A-Za-z0-9_./-]*|/*|*/../*|../*|*/..|..|*//*|./*|*/./*)
            echo "release validation failed: unsafe bundle path: $relative" >&2
            bad_path=1
            ;;
    esac
done <<EOF_PATHS
$(find "$bundle" -mindepth 1 -print)
EOF_PATHS
[ "$bad_path" -eq 0 ] || exit 1

[ -z "$(find "$bundle" -mindepth 1 ! -type d ! -type f -print -quit)" ] ||
    fail "bundle contains a symlink or special file"
[ -z "$(find "$bundle" -type f -links +1 -print -quit)" ] ||
    fail "bundle contains a hard-linked regular file"
[ -z "$(find "$bundle" -mindepth 1 -perm /022 -print -quit)" ] ||
    fail "bundle contains a group- or world-writable path"

for required_file in \
    SOURCE_COMMIT \
    SHA256SUMS \
    MODE_INVENTORY \
    vps-release-manifest-v2.json \
    bin/robin-highscores-admin \
    bin/robin-highscores-manifestctl \
    bin/robin-highscores-server \
    bin/robin-highscores-worker \
    bin/robin-replay-verifier \
    config/highscores-server.toml \
    config/highscores-worker.toml \
    config/api.env \
    config/worker.env \
    deploy/README.md \
    deploy/VPS_RELEASE_INSTALL.md \
    deploy/BACKUP_RESTORE.md \
    deploy/DEPLOY_BOOTSTRAP_SHA256SUMS \
    deploy/deploy-release.sh \
    deploy/rollback-release.sh \
    deploy/ROOT_ONCE_SHA256SUMS \
    deploy/root-once.sh \
    deploy/validate-release-bundle.sh \
    deploy/tests/real-runtime-fence-release-gate.sh \
    deploy/tests/real-runtime-fence-e2e.py \
    deploy/tests/real-runtime-fence-e2e-selftest.py \
    deploy/nginx-robinhood-api.challenge.conf \
    deploy/nginx-robinhood-cloudflare-only.conf \
    deploy/nginx-robinhood-api.locations.conf \
    deploy/nginx-robinhood-api.vhost.conf \
    systemd/user/robin-highscores.target \
    systemd/user/robin-highscores-api.service \
    systemd/user/robin-highscores-worker.service \
    systemd/user/robin-highscores-backup.service \
    systemd/user/robin-highscores-backup.timer
do
    [ -f "$bundle/$required_file" ] && [ ! -L "$bundle/$required_file" ] ||
        fail "required regular file is missing: $required_file"
done

for executable in \
    bin/robin-highscores-admin \
    bin/robin-highscores-manifestctl \
    bin/robin-highscores-server \
    bin/robin-highscores-worker \
    bin/robin-replay-verifier \
    deploy/deploy-release.sh \
    deploy/rollback-release.sh \
    deploy/root-once.sh \
    deploy/validate-release-bundle.sh \
    deploy/tests/real-runtime-fence-release-gate.sh \
    deploy/tests/real-runtime-fence-e2e.py \
    deploy/tests/real-runtime-fence-e2e-selftest.py
do
    [ -x "$bundle/$executable" ] || fail "required executable is not executable: $executable"
done

[ "$(wc -l < "$bundle/SOURCE_COMMIT" | tr -d ' ')" -eq 1 ] ||
    fail "SOURCE_COMMIT must contain exactly one line"
actual_commit=$(sed -n '1p' "$bundle/SOURCE_COMMIT")
[ "$actual_commit" = "$expected_commit" ] || fail "SOURCE_COMMIT differs from the reviewed commit"

actual_sums_sha256=$(sha256sum "$bundle/SHA256SUMS" | cut -d' ' -f1)
[ "$actual_sums_sha256" = "$expected_sums_sha256" ] ||
    fail "SHA256SUMS differs from the reviewed digest"

temporary_directory=$(mktemp -d)
cleanup() {
    rm -f -- "$temporary_directory/actual" "$temporary_directory/listed" "$temporary_directory/env"
    rmdir -- "$temporary_directory"
}
trap cleanup EXIT HUP INT TERM

line_number=0
: > "$temporary_directory/listed"
while IFS= read -r line || [ -n "$line" ]; do
    line_number=$((line_number + 1))
    digest=${line%% *}
    remainder=${line#"$digest"}
    [ "${#digest}" -eq 64 ] || fail "invalid digest length in SHA256SUMS line $line_number"
    case "$digest" in
        *[!0-9a-f]*|'') fail "invalid digest in SHA256SUMS line $line_number" ;;
    esac
    case "$remainder" in
        '  '*) relative=${remainder#'  '} ;;
        *) fail "SHA256SUMS line $line_number is not in canonical two-space format" ;;
    esac
    case "$relative" in
        ''|*[!A-Za-z0-9_./-]*|/*|*/../*|../*|*/..|..|*//*|./*|*/./*|SHA256SUMS)
            fail "unsafe path in SHA256SUMS line $line_number"
            ;;
    esac
    printf '%s\n' "$relative" >> "$temporary_directory/listed"
done < "$bundle/SHA256SUMS"
[ "$line_number" -gt 0 ] || fail "SHA256SUMS is empty"

(cd "$bundle" && find . -type f ! -path ./SHA256SUMS -printf '%P\n' | LC_ALL=C sort) \
    > "$temporary_directory/actual"
cmp -s "$temporary_directory/actual" "$temporary_directory/listed" ||
    fail "SHA256SUMS does not list every regular file exactly once"
(cd "$bundle" && sha256sum --strict --check SHA256SUMS >/dev/null) ||
    fail "one or more bundle file hashes do not match"

validate_literal_opt_root() {
    script=$1
    awk '
        BEGIN {
            expected = "opt_root=/home/robinhood/.local/opt/robin-highscores"
        }
        {
            assignment = $0
            sub(/^[[:space:]]*/, "", assignment)
            if (assignment ~ /^opt_root=/) {
                assignment_count++
                if ($0 != expected) {
                    invalid = 1
                }
            }
        }
        END {
            exit !(assignment_count == 1 && invalid == 0)
        }
    ' "$bundle/$script" ||
        fail "$script must contain exactly one literal canonical opt_root assignment"
}

validate_literal_opt_root deploy/deploy-release.sh
validate_literal_opt_root deploy/rollback-release.sh

manifest_tool_authority_is_accepted() {
    _authority_candidate_v2=$1
    _authority_sums=$2
    _authority_commit=$3
    _authority_embedded=$4
    _authority_inherited=$5

    [ "$_authority_inherited" = "$_authority_embedded" ] || {
        [ "$_authority_candidate_v2" = 6b5417d94ff17349188bc189ab996948ff58fee26dc16c3cdae432e347688ad9 ] &&
            [ "$_authority_sums" = c7a158a6d534509728051e5740a4f71f18e0dc8f7f285cf8038643c8f0162157 ] &&
            [ "$_authority_commit" = d3012fbb64954274aa4778df0882ba354e90df9c ] &&
            [ "$_authority_embedded" = 673037c81a26f4254c66c0a286a0a879239fd7d01033fb05f97b32c2341f354c ] &&
            [ "$_authority_inherited" = 1dbeead7aba2f62eb95acb2f5b2ea19031a5b62c1d81252079399bd33c331d72 ]
    }
}

# The complete candidate, including its embedded manifest tool, was already
# authenticated above against the out-of-band-pinned SHA256SUMS. Normally the
# inherited executable must be byte-identical. The sole mixed-provenance
# exception is pinned to the exact installed candidate and reviewed e075 tool.
manifest_tool_sha=$(awk '$2 == "bin/robin-highscores-manifestctl" { print $1 }' "$bundle/SHA256SUMS")
case "$manifest_tool_sha" in *[!0-9a-f]*|'') fail "SHA256SUMS omits the canonical manifest-tool digest" ;; esac
[ "${#manifest_tool_sha}" -eq 64 ] || fail "SHA256SUMS has a malformed manifest-tool digest"
candidate_v2_sha=$(sha256sum "$bundle/vps-release-manifest-v2.json" | cut -d' ' -f1)
inherited_manifest_tool_sha=$(sha256sum "$manifest_tool_fd" | cut -d' ' -f1)
manifest_tool_authority_is_accepted \
    "$candidate_v2_sha" "$actual_sums_sha256" "$actual_commit" \
    "$manifest_tool_sha" "$inherited_manifest_tool_sha" ||
    fail "manifest-tool descriptor is outside the authenticated candidate or reviewed override authority"
"$manifest_tool_fd" validate-vps-release-v2 "$bundle" ||
    fail "typed VPS release manifest validation failed"

# Secrets and mutable production state are never release inputs.
while IFS= read -r relative; do
    case "$relative" in
        */api-secrets/*|api-secrets/*|*.key|*.token|*.sqlite|*.sqlite3|*.db|*.pem)
            fail "bundle contains a forbidden secret or mutable-state filename: $relative"
            ;;
        *verifier-broker*|*polkit*)
            fail "bundle contains an obsolete privileged broker artifact: $relative"
            ;;
    esac
done < "$temporary_directory/actual"

printf 'RUST_LOG=info\n' > "$temporary_directory/env"
cmp -s "$temporary_directory/env" "$bundle/config/api.env" ||
    fail "config/api.env must contain exactly RUST_LOG=info"
cmp -s "$temporary_directory/env" "$bundle/config/worker.env" ||
    fail "config/worker.env must contain exactly RUST_LOG=info"

release_root="/home/robinhood/.local/opt/robin-highscores/releases/$expected_commit"
state_root=/home/robinhood/.local/share/robin-highscores
server_config=$bundle/config/highscores-server.toml
worker_config=$bundle/config/highscores-worker.toml

for required_text in \
    "$state_root/database" \
    "$state_root/replays" \
    "$state_root/campaign-states" \
    "$state_root/api-secrets/cursor-hmac.key" \
    "$state_root/api-secrets/competition-run-grant.key" \
    "$state_root/api-secrets/run-preflight-grant.key" \
    "$state_root/api-secrets/backup-authority-hmac.key" \
    "$state_root/api-secrets/moderation-bearer.token" \
    "$state_root/runtime-fence" \
    "$state_root/status/backup-status.json" \
    "$release_root/vps-release-manifest-v2.json"
do
    grep -Fq "$required_text" "$server_config" ||
        fail "server config does not pin required path: $required_text"
done
grep -Fq 'bind = "127.0.0.1:8787"' "$server_config" ||
    fail "server config must bind exactly to loopback port 8787"
grep -Fq 'allowed_origins = []' "$server_config" ||
    fail "production server config must keep CORS disabled"
grep -Fq 'trusted_proxy_cidrs = ["127.0.0.1/32", "::1/128"]' "$server_config" ||
    fail "server config must trust only the loopback nginx proxy"

for required_text in \
    "server_config = \"$release_root/config/highscores-server.toml\"" \
    "campaign_state_directory = \"$state_root/campaign-states\"" \
    'bwrap_program = "/usr/bin/bwrap"' \
    'prlimit_program = "/usr/bin/prlimit"' \
    "verifier_program = \"$release_root/bin/robin-replay-verifier\""
do
    grep -Fq "$required_text" "$worker_config" ||
        fail "worker config does not pin required value: $required_text"
done
grep -Fq '[verifier_launcher]' "$worker_config" ||
    fail "worker config is missing the direct verifier launcher policy"
if grep -Eq 'broker_socket|broker_response_timeout|= "/(opt|var/lib|etc|srv)/robin-highscores' "$server_config" "$worker_config"; then
    fail "runtime config contains an obsolete privileged or system-wide path"
fi

for unit in \
    robin-highscores-api.service \
    robin-highscores-worker.service \
    robin-highscores-backup.service
do
    unit_path=$bundle/systemd/user/$unit
    grep -Fq "$release_root" "$unit_path" || fail "$unit is not pinned to the exact release"
    if grep -Eq '^(User|Group|SupplementaryGroups)=|WantedBy=multi-user.target|/etc/systemd/system|verifier-broker|sudo|polkit' "$unit_path"; then
        fail "$unit contains a root/system-service assumption"
    fi
done

status_root=$state_root/status
status_path=$status_root/backup-status.json
backup_root=$state_root/backups
secret_root=$state_root/api-secrets
backup_authority_key=$secret_root/backup-authority-hmac.key
runtime_fence=$state_root/runtime-fence
api_unit=$bundle/systemd/user/robin-highscores-api.service
worker_unit=$bundle/systemd/user/robin-highscores-worker.service
backup_unit=$bundle/systemd/user/robin-highscores-backup.service
backup_timer=$bundle/systemd/user/robin-highscores-backup.timer

grep -Fxq "backup_manifest_path = \"$status_path\"" "$server_config" ||
    fail "server backup_manifest_path does not name the isolated status authority"
grep -Fxq "release_manifest_path = \"$release_root/vps-release-manifest-v2.json\"" "$server_config" ||
    fail "server release_manifest_path does not name the exact installed release"
grep -Fxq 'maximum_backup_age_hours = 32' "$server_config" ||
    fail "server maximum backup age does not cover schedule jitter, timeout, and margin"
grep -Fxq "backup_authority_hmac_secret_path = \"$backup_authority_key\"" "$server_config" ||
    fail "server config does not bind the exact fifth backup authority"
grep -Fxq "runtime_fence_directory = \"$runtime_fence\"" "$server_config" ||
    fail "server config does not bind the exact protected runtime database fence"
grep -Fxq "ReadOnlyPaths=$status_root" "$api_unit" ||
    fail "API unit cannot read the backup status authority"
for unit in "$api_unit" "$worker_unit" "$backup_unit"; do
    grep -Fxq "ReadOnlyPaths=$runtime_fence" "$unit" ||
        fail "service unit cannot read the protected runtime database fence: $unit"
    if grep -Fxq "ReadWritePaths=$runtime_fence" "$unit"; then
        fail "service unit may mutate the protected runtime database fence: $unit"
    fi
done
for unit in "$api_unit" "$backup_unit"; do
    grep -Fxq "ReadOnlyPaths=$secret_root" "$unit" ||
        fail "service unit cannot read the exact owner-only secret root: $unit"
    if grep -Fxq "ReadWritePaths=$secret_root" "$unit"; then
        fail "service unit may mutate the owner-only secret root: $unit"
    fi
done
grep -Fxq "InaccessiblePaths=$secret_root" "$worker_unit" ||
    fail "worker unit can access API and backup authority secrets"
grep -Fxq "InaccessiblePaths=$backup_root" "$api_unit" ||
    fail "API unit does not hide backup payloads"
if grep -Fxq "ReadWritePaths=$status_root" "$api_unit" ||
    grep -Eq "^Read(Only|Write)Paths=([^[:space:]]+[[:space:]])*$backup_root([[:space:]]|$)" "$api_unit"; then
    fail "API unit grants forbidden backup/status access"
fi
grep -Fxq "ReadWritePaths=$backup_root" "$backup_unit" ||
    fail "backup unit cannot write backup payloads"
grep -Fxq "ReadWritePaths=$status_root" "$backup_unit" ||
    fail "backup unit cannot publish backup status"
grep -Fq " --release-manifest-path $release_root/vps-release-manifest-v2.json --backup-root $backup_root --status-path $status_path " "$backup_unit" ||
    fail "backup command does not publish the server's exact status authority"
if grep -Fq -- "--configuration-root" "$backup_unit" ||
    grep -Fq -- "--restore-source-map $release_root=" "$backup_unit" ||
    grep -Fq -- "--restore-source-map /home/robinhood/.config/systemd/user=" "$backup_unit" ||
    grep -Fq -- "--restore-source-map $backup_authority_key=" "$backup_unit" ||
    grep -Fq -- "--restore-source-map $runtime_fence=" "$backup_unit"; then
    fail "backup command copies immutable/config, backup authority, runtime fence, or the symlink-bearing user-unit tree"
fi
expected_restore_maps=0
for restore_path in \
    "$state_root/api-secrets/cursor-hmac.key" \
    "$state_root/api-secrets/competition-run-grant.key" \
    "$state_root/api-secrets/run-preflight-grant.key" \
    "$state_root/api-secrets/moderation-bearer.token" \
    /home/robinhood/.config/systemd/user/robin-highscores.target \
    /home/robinhood/.config/systemd/user/robin-highscores-api.service \
    /home/robinhood/.config/systemd/user/robin-highscores-worker.service \
    /home/robinhood/.config/systemd/user/robin-highscores-backup.service \
    /home/robinhood/.config/systemd/user/robin-highscores-backup.timer
do
    grep -Fq -- "--restore-source-map $restore_path=$restore_path" "$backup_unit" ||
        fail "backup command omits exact restore source $restore_path"
    expected_restore_maps=$((expected_restore_maps + 1))
done
[ "$(grep -Fo -- '--restore-source-map ' "$backup_unit" | wc -l | tr -d ' ')" -eq "$expected_restore_maps" ] ||
    fail "backup command has an unexpected restore source map"
if grep -Fxq 'ReadOnlyPaths=/home/robinhood/.config/systemd/user' "$backup_unit"; then
    fail "backup sandbox exposes the whole symlink-bearing user-unit tree"
fi
for unit in \
    robin-highscores.target \
    robin-highscores-api.service \
    robin-highscores-worker.service \
    robin-highscores-backup.service \
    robin-highscores-backup.timer
do
    grep -Fxq "ReadOnlyPaths=/home/robinhood/.config/systemd/user/$unit" "$backup_unit" ||
        fail "backup sandbox omits exact user-unit source $unit"
done
grep -Fxq 'TimeoutStartSec=6h' "$backup_unit" ||
    fail "backup unit timeout differs from the readiness timing contract"
grep -Fxq 'OnCalendar=*-*-* 02:15:00' "$backup_timer" ||
    fail "backup timer is not on the exact daily schedule"
grep -Fxq 'RandomizedDelaySec=45m' "$backup_timer" ||
    fail "backup timer jitter differs from the readiness timing contract"
for inaccessible in "$backup_root" "$status_root"; do
    grep -Fxq "InaccessiblePaths=$inaccessible" "$worker_unit" ||
        fail "worker unit can access protected backup state: $inaccessible"
    if grep -Eq "^Read(Only|Write)Paths=([^[:space:]]+[[:space:]])*$inaccessible([[:space:]]|$)" "$worker_unit"; then
        fail "worker unit grants forbidden backup/status access: $inaccessible"
    fi
done

deploy_script=$bundle/deploy/deploy-release.sh
rollback_script=$bundle/deploy/rollback-release.sh

require_script_text() {
    inspected_script=$1
    required_text=$2
    description=$3
    grep -Fq -- "$required_text" "$inspected_script" ||
        fail "$description: ${inspected_script#"$bundle"/}"
}

reject_script_pattern() {
    inspected_script=$1
    rejected_pattern=$2
    description=$3
    if grep -Eq -- "$rejected_pattern" "$inspected_script"; then
        fail "$description: ${inspected_script#"$bundle"/}"
    fi
}

# Deploy retains one exact plan and candidate inode from the outer Rust wrapper
# through source consumption and no-replace promotion. The plan, candidate V2
# manifest, and activation lock are all independent descriptor authorities.
for required_text in \
    'plan_fd=' \
    'candidate_root_fd=' \
    'expected_plan_sha256=' \
    'expected_vps_manifest_sha256=' \
    'consume-vps-sources-v2' \
    'promote-inherited-vps-release-v2' \
    '--candidate-root-fd' \
    '--activation-lock-fd' \
    'probe-runtime-authority-v2' \
    'initialize-backup-authority-key-v2' \
    'complete-backup-authority-key-v2' \
    'initialize-vps-runtime-fence-v1' \
    'real-runtime-fence-release-gate.sh' \
    'ROBIN_REAL_FENCE_PINNED_CANDIDATE_FD' \
    'mandatory authentic runtime-fence release gate failed before activation mutation' \
    'verify-live-database-schema-v2'
do
    require_script_text "$deploy_script" "$required_text" \
        "deploy script omits the descriptor-pinned V2 transaction contract ($required_text)"
done

# Both source and target backup boundaries must be admitted by the typed
# BackupV4 verifier. A raw status-file digest is not a backup receipt.
for inspected_script in "$deploy_script" "$rollback_script"; do
    require_script_text "$inspected_script" 'verify-transaction-backup' \
        "activation script omits typed BackupV4 transaction verification"
    require_script_text "$inspected_script" 'estimate-backup-space' \
        "activation script omits typed backup capacity estimation"
    require_script_text "$inspected_script" '--require-available' \
        "activation script does not make typed capacity admission mandatory"
    require_script_text "$inspected_script" 'verify-live-database-schema-v2' \
        "activation script omits descriptor-pinned live-schema verification"
    reject_script_pattern "$inspected_script" \
        'robin-highscores-prebackup-receipt-v1|backup_status_sha256=|backup_du=|du[[:space:]]+--bytes' \
        "activation script retains a legacy status-hash prebackup or shell capacity authority"
done

reject_script_pattern "$deploy_script" \
    '(^|[[:space:]"])promote-vps-release-v2([[:space:]"\\]|$)|bundle_staging=|release_staging=' \
    "deploy script retains a copy-based or path-only release promotion lane"
reject_script_pattern "$rollback_script" \
    'consume-vps-sources-v2|expected_plan_sha256|PLAN_FD' \
    "rollback must remain disjoint from uploader plan and source consumption"

# PublicationV3 and VpsReleaseManifestV2 are the only admitted release wire
# formats. The typed validator enforces their full closure; reject obvious stale
# documents here as a readable operator diagnostic as well.
if grep -Eq '(^|/)[^/]*(publication-v2|vps-release-manifest-v1)[^/]*(/|$)' "$temporary_directory/actual"; then
    fail "bundle contains a stale PublicationV2 or VPS V1 release document"
fi

echo "release bundle validated: $expected_commit"
