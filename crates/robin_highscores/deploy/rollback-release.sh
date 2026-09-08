#!/bin/sh
set -eu
PATH=/usr/bin:/bin
export PATH

usage() {
    echo "usage: $0 TARGET_40HEX_COMMIT EXPECTED_SHA256SUMS_SHA256 EXPECTED_DEPLOY_BOOTSTRAP_SHA256SUMS_SHA256 /proc/self/fd/MANIFEST_FD /proc/self/fd/VALIDATOR_FD /proc/self/fd/MANIFESTCTL_FD /proc/self/fd/LOCK_FD" >&2
    echo "   or: $0 --resume-target TARGET_40HEX_COMMIT EXPECTED_SHA256SUMS_SHA256 EXPECTED_DEPLOY_BOOTSTRAP_SHA256SUMS_SHA256 /proc/self/fd/MANIFEST_FD /proc/self/fd/VALIDATOR_FD /proc/self/fd/MANIFESTCTL_FD /proc/self/fd/LOCK_FD" >&2
    exit 64
}

fail() {
    echo "rollback failed: $*" >&2
    exit 1
}

valid_proc_fd_path() {
    case "$1" in
        /proc/self/fd/*)
            descriptor_number=${1#/proc/self/fd/}
            case "$descriptor_number" in ''|*[!0-9]*) return 1 ;; esac
            ;;
        *) return 1 ;;
    esac
}

case "$#:$1" in
    7:*)
        rollback_mode=normal
        target_commit=$1
        expected_sums_sha256=$2
        expected_bootstrap_sums_sha256=$3
        bootstrap_manifest_fd=$4
        validator_fd=$5
        manifest_tool_fd=$6
        activation_lock_fd=$7
        ;;
    8:--resume-target)
        rollback_mode=resume
        target_commit=$2
        expected_sums_sha256=$3
        expected_bootstrap_sums_sha256=$4
        bootstrap_manifest_fd=$5
        validator_fd=$6
        manifest_tool_fd=$7
        activation_lock_fd=$8
        ;;
    *) usage ;;
esac
case "$target_commit" in *[!0-9a-f]*|'') fail "target commit must be lowercase hexadecimal" ;; esac
[ "${#target_commit}" -eq 40 ] || fail "target commit must contain exactly 40 hexadecimal digits"
for digest in "$expected_sums_sha256" "$expected_bootstrap_sums_sha256"; do
    case "$digest" in *[!0-9a-f]*|'') fail "expected digest must be lowercase hexadecimal" ;; esac
    [ "${#digest}" -eq 64 ] || fail "expected digest must contain exactly 64 hexadecimal digits"
done

expected_user=robinhood
expected_home=/home/robinhood
opt_root=/home/robinhood/.local/opt/robin-highscores
release_root=$opt_root/releases
current_link=$opt_root/current
user_unit_root=$expected_home/.config/systemd/user
state_root=$expected_home/.local/share/robin-highscores
secret_root=$state_root/api-secrets
raw_root=$state_root/raw-content
backup_status_path=$state_root/status/backup-status.json
backup_authority_key=$secret_root/backup-authority-hmac.key
target_release=$release_root/$target_commit
activation_lock=$opt_root/activation.lock
bootstrap_manifest_name=DEPLOY_BOOTSTRAP_SHA256SUMS
release_manifest_name=vps-release-manifest-v2.json

[ "$(id -un)" = "$expected_user" ] && [ "$(id -u)" -ne 0 ] || fail "must run as non-root robinhood"
passwd_home=$(getent passwd "$expected_user" | cut -d: -f6)
[ "$passwd_home" = "$expected_home" ] && [ "${HOME:-}" = "$expected_home" ] ||
    fail "robinhood home must be exactly /home/robinhood"

valid_proc_fd_path "$0" && valid_proc_fd_path "$bootstrap_manifest_fd" &&
    valid_proc_fd_path "$validator_fd" && valid_proc_fd_path "$manifest_tool_fd" &&
    valid_proc_fd_path "$activation_lock_fd" ||
    fail "rollback, bootstrap manifest, validator, manifest tool, and activation lock must be explicit inherited descriptors"
[ -f "$0" ] && [ "$(stat -Lc %u -- "$0")" -eq "$(id -u)" ] &&
    [ "$(stat -Lc %h -- "$0")" -eq 1 ] && [ "$(stat -Lc %a -- "$0")" = 500 ] || fail "unsafe rollback descriptor"
[ -f "$bootstrap_manifest_fd" ] && [ "$(stat -Lc %u -- "$bootstrap_manifest_fd")" -eq "$(id -u)" ] &&
    [ "$(stat -Lc %h -- "$bootstrap_manifest_fd")" -eq 1 ] &&
    [ "$(stat -Lc %a -- "$bootstrap_manifest_fd")" = 400 ] || fail "unsafe bootstrap-manifest descriptor"
[ -f "$validator_fd" ] && [ "$(stat -Lc %u -- "$validator_fd")" -eq "$(id -u)" ] &&
    [ "$(stat -Lc %h -- "$validator_fd")" -eq 1 ] && [ "$(stat -Lc %a -- "$validator_fd")" = 500 ] ||
    fail "unsafe validator descriptor"
[ -f "$manifest_tool_fd" ] && [ "$(stat -Lc %u -- "$manifest_tool_fd")" -eq "$(id -u)" ] &&
    [ "$(stat -Lc %h -- "$manifest_tool_fd")" -eq 1 ] && [ "$(stat -Lc %a -- "$manifest_tool_fd")" = 550 ] ||
    fail "unsafe manifest-tool descriptor"
[ -f "$activation_lock_fd" ] && [ "$(stat -Lc %u -- "$activation_lock_fd")" -eq "$(id -u)" ] &&
    [ "$(stat -Lc %h -- "$activation_lock_fd")" -eq 1 ] && [ "$(stat -Lc %a -- "$activation_lock_fd")" = 600 ] ||
    fail "unsafe activation-lock descriptor"
activation_lock_number=${activation_lock_fd#/proc/self/fd/}
activation_lock_identity=$(stat -Lc %d:%i -- "$activation_lock_fd") || fail "cannot inspect activation-lock descriptor"
[ -f "$activation_lock" ] && [ ! -L "$activation_lock" ] &&
    [ "$(stat -c %d:%i -- "$activation_lock")" = "$activation_lock_identity" ] ||
    fail "activation-lock descriptor is not the canonical activation.lock inode"
[ "$(sha256sum "$bootstrap_manifest_fd" | cut -d' ' -f1)" = "$expected_bootstrap_sums_sha256" ] ||
    fail "bootstrap manifest differs from its out-of-band digest"
bootstrap_digest_for() {
    bootstrap_name=$1
    bootstrap_digest=$(awk -v wanted="$bootstrap_name" '$2 == wanted { print $1 }' "$bootstrap_manifest_fd")
    case "$bootstrap_digest" in *[!0-9a-f]*|'') return 1 ;; esac
    [ "${#bootstrap_digest}" -eq 64 ] || return 1
    [ "$(awk -v wanted="$bootstrap_name" '$2 == wanted { count += 1 } END { print count + 0 }' "$bootstrap_manifest_fd")" -eq 1 ] || return 1
    printf '%s\n' "$bootstrap_digest"
}
[ "$(wc -l < "$bootstrap_manifest_fd" | tr -d ' ')" -eq 3 ] || fail "bootstrap manifest must contain exactly three entries"
bootstrap_digest_for deploy-release.sh >/dev/null || fail "bootstrap manifest omits deploy-release.sh"
[ "$(sha256sum "$0" | cut -d' ' -f1)" = "$(bootstrap_digest_for rollback-release.sh)" ] ||
    fail "rollback descriptor differs from the pinned bootstrap manifest"
[ "$(sha256sum "$validator_fd" | cut -d' ' -f1)" = "$(bootstrap_digest_for validate-release-bundle.sh)" ] ||
    fail "validator descriptor differs from the pinned bootstrap manifest"

for command_path in /usr/bin/bash /usr/bin/curl /usr/bin/findmnt /usr/bin/flock /usr/bin/mv; do
    [ -f "$command_path" ] && [ ! -L "$command_path" ] && [ -x "$command_path" ] || fail "required host executable is missing: $command_path"
done
[ "$(sha256sum /usr/bin/mv | cut -d' ' -f1)" = a781be46e6f27ca5d7e119225429a4a12ef941eea15ed2c44eea0c8bca7e4ebe ] ||
    fail "host /usr/bin/mv differs from the reviewed no-replace implementation"

# Only the five exact outer authorities may occupy the POSIX single-digit FD
# pool. Reject ambient inheritance before opening a release or mutating state,
# and require two free slots for distinct retained target/source roots.
rollback_free_descriptor_count=0
for rollback_pool_fd in 9 8 7 6 5 4 3; do
    rollback_pool_path=/proc/self/fd/$rollback_pool_fd
    if [ -e "$rollback_pool_path" ] || [ -L "$rollback_pool_path" ]; then
        rollback_pool_recognized=0
        for rollback_authority in "$0" "$bootstrap_manifest_fd" "$validator_fd" \
            "$manifest_tool_fd" "$activation_lock_fd"; do
            [ "$rollback_pool_path" != "$rollback_authority" ] ||
                rollback_pool_recognized=1
        done
        [ "$rollback_pool_recognized" -eq 1 ] ||
            fail "unrecognized ambient rollback descriptor occupies $rollback_pool_fd"
    else
        rollback_free_descriptor_count=$((rollback_free_descriptor_count + 1))
    fi
done
[ "$rollback_free_descriptor_count" -ge 2 ] ||
    fail "rollback descriptor pool cannot retain distinct target and source roots"

# POSIX sh has no dynamic-descriptor allocation. Reserve one descriptor from
# the bounded high-FD pool not inherited by the outer transaction wrapper and
# keep the directory open for the entire transaction. Refuse ambient
# descriptor crowding rather than overwrite any inherited authority.
open_pinned_release_root() {
    pinned_path=$1
    pinned_result_name=$2
    pinned_fd=
    for candidate_fd in 9 8 7 6 5 4 3; do
        [ ! -e "/proc/self/fd/$candidate_fd" ] && [ ! -L "/proc/self/fd/$candidate_fd" ] || continue
        case "$candidate_fd" in
            9) exec 9< "$pinned_path" ;;
            8) exec 8< "$pinned_path" ;;
            7) exec 7< "$pinned_path" ;;
            6) exec 6< "$pinned_path" ;;
            5) exec 5< "$pinned_path" ;;
            4) exec 4< "$pinned_path" ;;
            3) exec 3< "$pinned_path" ;;
        esac
        pinned_fd=$candidate_fd
        break
    done
    [ -n "$pinned_fd" ] || return 1
    eval "$pinned_result_name=\$pinned_fd"
}

validate_pinned_release_root() {
    pinned_fd=$1
    pinned_path=$2
    pinned_commit=$3
    pinned_expected_manifest=$4
    pinned_proc=/proc/self/fd/$pinned_fd
    [ -d "$pinned_proc" ] &&
        [ "$(stat -Lc %u -- "$pinned_proc")" -eq "$(id -u)" ] &&
        [ "$(stat -Lc %d:%i -- "$pinned_proc")" = "$(stat -c %d:%i -- "$pinned_path")" ] &&
        [ "$(realpath -e -- "$pinned_path")" = "$pinned_path" ] || return 1
    [ "$(cat -- "$pinned_proc/SOURCE_COMMIT")" = "$pinned_commit" ] || return 1
    pinned_actual_manifest=$("$manifest_tool_fd" validate-vps-release-v2 "$pinned_proc/.") || return 1
    [ "$pinned_actual_manifest" = "$pinned_expected_manifest" ]
}

release_publication_lock_sha256() {
    projection_release_root=$1
    projection_vps_sha256=$2
    projected_publication_sha256=$(/usr/bin/bash -c '
        set -eu
        exec {release_manifest_fd}<"$1/vps-release-manifest-v2.json"
        exec "$2" project-vps-publication-lock-v2 \
            --release-manifest-fd "$release_manifest_fd" \
            --expected-vps-release-manifest-sha256 "$3"
    ' robin-rollback-vps-publication-projection \
        "$projection_release_root" "$manifest_tool_fd" "$projection_vps_sha256") || return 1
    case "$projected_publication_sha256" in *[!0-9a-f]*|'') return 1 ;; esac
    [ "${#projected_publication_sha256}" -eq 64 ] || return 1
    printf '%s\n' "$projected_publication_sha256"
}

retain_source_release_authority() {
    source_release=$release_root/$previous_commit
    if [ -n "${source_release_root_fd:-}" ]; then
        validate_pinned_release_root "$source_release_root_fd" "$source_release" "$previous_commit" \
            "$source_vps_release_manifest_sha256"
        return
    fi
    [ -d "$source_release" ] && [ ! -L "$source_release" ] &&
        [ "$(realpath -e -- "$source_release")" = "$source_release" ] &&
        [ "$(stat -c %u -- "$source_release")" -eq "$(id -u)" ] || return 1
    open_pinned_release_root "$source_release" source_release_root_fd || return 1
    source_release_root=/proc/self/fd/$source_release_root_fd
    [ "$(cat -- "$source_release_root/SOURCE_COMMIT")" = "$previous_commit" ] || return 1
    source_vps_release_manifest_sha256=$("$manifest_tool_fd" \
        validate-vps-release-v2 "$source_release_root/.") || return 1
    case "$source_vps_release_manifest_sha256" in *[!0-9a-f]*|'') return 1 ;; esac
    [ "${#source_vps_release_manifest_sha256}" -eq 64 ] || return 1
    source_publication_lock_sha256=$(release_publication_lock_sha256 \
        "$source_release_root" "$source_vps_release_manifest_sha256") || return 1
    validate_pinned_release_root "$source_release_root_fd" "$source_release" "$previous_commit" \
        "$source_vps_release_manifest_sha256"
}

[ -d "$target_release" ] && [ ! -L "$target_release" ] &&
    [ "$(realpath -e -- "$target_release")" = "$target_release" ] &&
    [ "$(stat -c %u -- "$target_release")" -eq "$(id -u)" ] || fail "target release is not canonical or owner-controlled"
open_pinned_release_root "$target_release" target_release_root_fd ||
    fail "could not retain a descriptor for the exact rollback target"
target_release_root=/proc/self/fd/$target_release_root_fd
target_release_identity=$(stat -Lc %d:%i -- "$target_release_root") ||
    fail "could not inspect the retained rollback target"
target_device=$(stat -c %d -- "$target_release")
mount_inventory=$(/usr/bin/findmnt -Rrn -o TARGET --target "$target_release") || fail "could not inspect target mounts"
old_ifs=$IFS
IFS='
'
for mount_target in $mount_inventory; do
    case "$mount_target" in "$target_release"|"$target_release"/*) fail "target release contains a nested mount" ;; esac
done
IFS=$old_ifs
target_devices=$(find "$target_release" -xdev -printf '%D\n') || fail "could not scan target devices"
[ "$(printf '%s\n' "$target_devices" | LC_ALL=C sort -u)" = "$target_device" ] || fail "target release crosses devices"
target_violation=$(find "$target_release" -xdev -mindepth 0 ! -uid "$(id -u)" -print -quit) || fail "could not scan target ownership"
[ -z "$target_violation" ] || fail "target release contains mixed ownership"
target_violation=$(find "$target_release" -xdev -mindepth 1 ! -type d ! -type f -print -quit) || fail "could not scan target topology"
[ -z "$target_violation" ] || fail "target release contains links or special nodes"
if ! /bin/sh "$validator_fd" "$target_release_root/." "$target_commit" "$expected_sums_sha256" "$manifest_tool_fd"; then
    fail "trusted descriptor validation rejected the rollback target"
fi
target_vps_release_manifest_sha256=$("$manifest_tool_fd" validate-vps-release-v2 "$target_release_root/.") ||
    fail "typed validation rejected the retained rollback target"
case "$target_vps_release_manifest_sha256" in *[!0-9a-f]*|'') fail "target V2 manifest digest is invalid" ;; esac
[ "${#target_vps_release_manifest_sha256}" -eq 64 ] || fail "target V2 manifest digest is invalid"
target_publication_lock_sha256=$(release_publication_lock_sha256 \
    "$target_release_root" "$target_vps_release_manifest_sha256") ||
    fail "could not project target publication identity"
systemctl --user show-environment >/dev/null 2>&1 || fail "systemd user manager is unavailable"

tree_mount_target() {
    inspected_root=$1
    mount_inventory=$(/usr/bin/findmnt -Rrn -o TARGET --target "$inspected_root") || return 1
    old_ifs=$IFS
    IFS='
'
    for target in $mount_inventory; do
        case "$target" in "$inspected_root"|"$inspected_root"/*) printf '%s\n' "$target" ;; esac
    done
    IFS=$old_ifs
}

validate_owned_tree() {
    inspected_root=$1
    [ -d "$inspected_root" ] && [ ! -L "$inspected_root" ] || return 1
    inspected_uid=$(stat -c %u -- "$inspected_root") || return 1
    inspected_device=$(stat -c %d -- "$inspected_root") || return 1
    [ "$inspected_uid" -eq "$(id -u)" ] || return 1
    mount_target=$(tree_mount_target "$inspected_root") || return 1
    [ -z "$mount_target" ] || return 1
    device_inventory=$(find "$inspected_root" -xdev -printf '%D\n') || return 1
    [ "$(printf '%s\n' "$device_inventory" | LC_ALL=C sort -u)" = "$inspected_device" ] || return 1
    violation=$(find "$inspected_root" -xdev -mindepth 0 ! -uid "$inspected_uid" -print -quit) || return 1
    [ -z "$violation" ] || return 1
    violation=$(find "$inspected_root" -xdev -type f -links +1 -print -quit) || return 1
    [ -z "$violation" ] || return 1
    violation=$(find "$inspected_root" -xdev -mindepth 1 ! -type d ! -type f -print -quit) || return 1
    [ -z "$violation" ]
}

remove_owned_tree() {
    cleanup_root=$1
    cleanup_parent=$2
    [ "$(dirname -- "$cleanup_root")" = "$cleanup_parent" ] || return 1
    if [ ! -e "$cleanup_root" ]; then [ ! -L "$cleanup_root" ]; return; fi
    validate_owned_tree "$cleanup_root" || return 1
    find "$cleanup_root" -xdev -type d -exec chmod u+rwx -- {} + || return 1
    rm -rf --one-file-system -- "$cleanup_root" || return 1
    [ ! -e "$cleanup_root" ] && [ ! -L "$cleanup_root" ]
}

remove_tracked_link() {
    cleanup_link=$1
    expected_target=$2
    if [ -L "$cleanup_link" ]; then
        [ "$(stat -c %u -- "$cleanup_link")" -eq "$(id -u)" ] &&
            [ "$(readlink -- "$cleanup_link")" = "$expected_target" ] || return 1
        rm -f -- "$cleanup_link"
    elif [ -e "$cleanup_link" ]; then return 1; fi
}

probe() {
    endpoint=$1; attempts=$2; count=0
    while [ "$count" -lt "$attempts" ]; do
        /usr/bin/curl --fail --silent --show-error --max-time 2 "$endpoint" >/dev/null 2>&1 && return 0
        count=$((count + 1)); sleep 1
    done
    return 1
}

umask 077
unit_stage_root=$user_unit_root/.robin-highscores-rollback-$target_commit.stage
unit_backup_root=$user_unit_root/.robin-highscores-rollback-$target_commit.recovery
temporary_link=$opt_root/.current-rollback-$target_commit
restore_link=$opt_root/.current-rollback-$target_commit.restore
prebackup_receipt=$opt_root/.rollback-prebackup-$target_commit
prebackup_receipt_temporary=$opt_root/.rollback-prebackup-$target_commit.new
target_backup_receipt=$opt_root/.rollback-target-backup-$target_commit
target_backup_receipt_temporary=$opt_root/.rollback-target-backup-$target_commit.new
prepared_journal=$opt_root/.rollback-prepared-$target_commit
prepared_journal_temporary=$opt_root/.rollback-prepared-$target_commit.new
previous_commit=
previous_target=
source_release_root_fd=
source_release_root=
unit_stage_owned=0
unit_backup_owned=0
temporary_link_owned=0
restore_link_owned=0
transaction_prepared=0
activation_started=0
rollback_complete=0
preserve_recovery=0
prebackup_timer_disabled=0
backup_quiesced=0
target_selected=0
selection_attempted=0
prebackup_receipt_temporary_owned=0
target_backup_receipt_temporary_owned=0
prepared_journal_owned=0
prepared_journal_temporary_owned=0
reuse_recovery_snapshot=0
journal_source_status_before=none
journal_target_status_before=none
journal_invalidated_source_receipt_sha=none

current_backup_status_sha256() {
    if [ ! -e "$backup_status_path" ] && [ ! -L "$backup_status_path" ]; then
        printf 'absent\n'
        return
    fi
    [ -f "$backup_status_path" ] && [ ! -L "$backup_status_path" ] &&
        [ "$(stat -c %u -- "$backup_status_path")" -eq "$(id -u)" ] &&
        [ "$(stat -c %h -- "$backup_status_path")" -eq 1 ] &&
        [ "$(stat -c %a -- "$backup_status_path")" = 400 ] || return 1
    sha256sum "$backup_status_path" | cut -d' ' -f1
}

valid_status_boundary() {
    case "$1" in
        absent) return 0 ;;
        *[!0-9a-f]*|'') return 1 ;;
    esac
    [ "${#1}" -eq 64 ]
}

recovery_snapshot_sha256() {
    validate_owned_tree "$unit_backup_root" || return 1
    [ "$(stat -c %a -- "$unit_backup_root")" = 700 ] || return 1
    recovery_paths=$(find "$unit_backup_root" -xdev -type f -printf '%P\n') || return 1
    recovery_paths=$(printf '%s\n' "$recovery_paths" | LC_ALL=C sort) || return 1
    recovery_inventory=
    old_ifs=$IFS
    IFS='
'
    for recovery_relative in $recovery_paths; do
        [ -n "$recovery_relative" ] || continue
        recovery_file=$unit_backup_root/$recovery_relative
        [ -f "$recovery_file" ] && [ ! -L "$recovery_file" ] &&
            [ "$(stat -c %u -- "$recovery_file")" -eq "$(id -u)" ] &&
            [ "$(stat -c %h -- "$recovery_file")" -eq 1 ] || { IFS=$old_ifs; return 1; }
        recovery_file_sha=$(sha256sum "$recovery_file" | cut -d' ' -f1) || { IFS=$old_ifs; return 1; }
        recovery_inventory=$recovery_inventory$recovery_file_sha'  '$recovery_relative'
'
    done
    IFS=$old_ifs
    printf '%s' "$recovery_inventory" | sha256sum | cut -d' ' -f1
}

durable_receipt_sha256() {
    durable_receipt=$1
    [ -f "$durable_receipt" ] && [ ! -L "$durable_receipt" ] &&
        [ "$(stat -c %u -- "$durable_receipt")" -eq "$(id -u)" ] &&
        [ "$(stat -c %h -- "$durable_receipt")" -eq 1 ] &&
        [ "$(stat -c %a -- "$durable_receipt")" = 400 ] || return 1
    sha256sum "$durable_receipt" | cut -d' ' -f1
}

prepared_journal_expected_text() {
    journal_source=$1
    journal_recovery_sha=$2
    journal_phase=$3
    expected_source_receipt_sha=none
    expected_target_receipt_sha=none
    case "$journal_phase" in
        source_backup_started)
            valid_status_boundary "$journal_source_status_before" || return 1
            [ "$journal_target_status_before" = none ] || return 1
            ;;
        source_backup_verified)
            valid_status_boundary "$journal_source_status_before" || return 1
            [ "$journal_target_status_before" = none ] || return 1
            expected_source_receipt_sha=$(durable_receipt_sha256 "$prebackup_receipt") || return 1
            ;;
        source_backup_invalidated)
            valid_status_boundary "$journal_source_status_before" || return 1
            [ "$journal_target_status_before" = none ] || return 1
            case "$journal_invalidated_source_receipt_sha" in *[!0-9a-f]*|'') return 1 ;; esac
            [ "${#journal_invalidated_source_receipt_sha}" -eq 64 ] || return 1
            expected_source_receipt_sha=$journal_invalidated_source_receipt_sha
            ;;
        target_backup_started)
            valid_status_boundary "$journal_source_status_before" || return 1
            valid_status_boundary "$journal_target_status_before" || return 1
            expected_source_receipt_sha=$(durable_receipt_sha256 "$prebackup_receipt") || return 1
            ;;
        target_backup_verified)
            valid_status_boundary "$journal_source_status_before" || return 1
            valid_status_boundary "$journal_target_status_before" || return 1
            expected_source_receipt_sha=$(durable_receipt_sha256 "$prebackup_receipt") || return 1
            expected_target_receipt_sha=$(durable_receipt_sha256 "$target_backup_receipt") || return 1
            ;;
        activation_complete)
            valid_status_boundary "$journal_source_status_before" || return 1
            valid_status_boundary "$journal_target_status_before" || return 1
            expected_source_receipt_sha=${4:-none}
            expected_target_receipt_sha=${5:-none}
            case "$expected_source_receipt_sha" in *[!0-9a-f]*|'') return 1 ;; esac
            case "$expected_target_receipt_sha" in *[!0-9a-f]*|'') return 1 ;; esac
            [ "${#expected_source_receipt_sha}" -eq 64 ] &&
                [ "${#expected_target_receipt_sha}" -eq 64 ] || return 1
            ;;
        preparing|prepared)
            [ "$journal_source_status_before" = none ] &&
                [ "$journal_target_status_before" = none ] || return 1
            ;;
        *) return 1 ;;
    esac
    printf '%s\n' \
        'schema=robin-highscores-activation-journal-v1' \
        'operation=rollback' \
        "source_commit=$journal_source" \
        "target_commit=$target_commit" \
        "target_sha256sums_sha256=$expected_sums_sha256" \
        "recovery_root=$unit_backup_root" \
        "unit_stage_root=$unit_stage_root" \
        "target_selector_stage=$temporary_link" \
        "source_selector_stage=$restore_link" \
        "recovery_snapshot_sha256=$journal_recovery_sha" \
        "source_backup_status_before_sha256=$journal_source_status_before" \
        "target_backup_status_before_sha256=$journal_target_status_before" \
        "source_backup_receipt_sha256=$expected_source_receipt_sha" \
        "target_backup_receipt_sha256=$expected_target_receipt_sha" \
        "phase=$journal_phase"
}

validate_prepared_journal_path() {
    journal_path=$1
    journal_source=$2
    [ -f "$journal_path" ] && [ ! -L "$journal_path" ] &&
        [ "$(stat -c %u -- "$journal_path")" -eq "$(id -u)" ] &&
        [ "$(stat -c %h -- "$journal_path")" -eq 1 ] &&
        [ "$(stat -c %a -- "$journal_path")" = 400 ] || return 1
    journal_phase=$(sed -n 's/^phase=//p' "$journal_path") || return 1
    journal_recovery_sha=$(sed -n 's/^recovery_snapshot_sha256=//p' "$journal_path") || return 1
    journal_source_status_before=$(sed -n 's/^source_backup_status_before_sha256=//p' "$journal_path") || return 1
    journal_target_status_before=$(sed -n 's/^target_backup_status_before_sha256=//p' "$journal_path") || return 1
    journal_source_receipt_sha=$(sed -n 's/^source_backup_receipt_sha256=//p' "$journal_path") || return 1
    journal_target_receipt_sha=$(sed -n 's/^target_backup_receipt_sha256=//p' "$journal_path") || return 1
    journal_invalidated_source_receipt_sha=$journal_source_receipt_sha
    case "$journal_phase" in
        preparing) [ "$journal_recovery_sha" = none ] || return 1 ;;
        prepared|source_backup_started|source_backup_verified|source_backup_invalidated|target_backup_started|target_backup_verified)
            [ "$(recovery_snapshot_sha256)" = "$journal_recovery_sha" ] || return 1
            ;;
        activation_complete) ;;
        *) return 1 ;;
    esac
    if [ "$journal_phase" != preparing ]; then
        case "$journal_recovery_sha" in *[!0-9a-f]*|'') return 1 ;; esac
        [ "${#journal_recovery_sha}" -eq 64 ] || return 1
    fi
    if [ "$journal_phase" = activation_complete ]; then
        expected_journal_text=$(prepared_journal_expected_text "$journal_source" \
            "$journal_recovery_sha" "$journal_phase" "$journal_source_receipt_sha" \
            "$journal_target_receipt_sha") || return 1
    else
        expected_journal_text=$(prepared_journal_expected_text "$journal_source" \
            "$journal_recovery_sha" "$journal_phase") || return 1
    fi
    [ "$(cat -- "$journal_path")" = "$expected_journal_text" ]
}

write_preparation_intent() {
    journal_source=$1
    [ ! -e "$prepared_journal" ] && [ ! -L "$prepared_journal" ] &&
        [ ! -e "$prepared_journal_temporary" ] && [ ! -L "$prepared_journal_temporary" ] || return 1
    prepared_journal_expected_text "$journal_source" none preparing >"$prepared_journal_temporary" || return 1
    prepared_journal_temporary_owned=1
    chmod 0400 -- "$prepared_journal_temporary" || return 1
    sync -f -- "$prepared_journal_temporary" || return 1
    mv -T -- "$prepared_journal_temporary" "$prepared_journal" || {
        [ ! -e "$prepared_journal_temporary" ] && [ ! -L "$prepared_journal_temporary" ] &&
            validate_prepared_journal "$journal_source" && [ "$journal_phase" = preparing ] || return 1
    }
    prepared_journal_temporary_owned=0
    prepared_journal_owned=1
    sync -f -- "$opt_root" || return 1
    validate_prepared_journal "$journal_source" && [ "$journal_phase" = preparing ]
}

abort_interrupted_preparation() {
    journal_source=$1
    validate_prepared_journal "$journal_source" && [ "$journal_phase" = preparing ] || return 1
    [ -L "$current_link" ] && [ "$(stat -c %u -- "$current_link")" -eq "$(id -u)" ] &&
        [ "$(readlink -- "$current_link")" = "releases/$journal_source" ] || return 1
    for unit in robin-highscores.target robin-highscores-api.service robin-highscores-worker.service \
        robin-highscores-backup.service robin-highscores-backup.timer; do
        [ -f "$user_unit_root/$unit" ] && [ ! -L "$user_unit_root/$unit" ] &&
            [ "$(stat -c %u -- "$user_unit_root/$unit")" -eq "$(id -u)" ] &&
            [ "$(stat -c %h -- "$user_unit_root/$unit")" -eq 1 ] &&
            [ "$(stat -c %a -- "$user_unit_root/$unit")" = 440 ] &&
            cmp -s -- "$user_unit_root/$unit" "$source_release_root/systemd/user/$unit" || return 1
    done
    if [ -e "$temporary_link" ] || [ -L "$temporary_link" ]; then
        remove_tracked_link "$temporary_link" "releases/$target_commit" || return 1
    fi
    if [ -e "$restore_link" ] || [ -L "$restore_link" ]; then
        remove_tracked_link "$restore_link" "releases/$journal_source" || return 1
    fi
    sync -f -- "$opt_root" || return 1
    if [ -e "$unit_stage_root" ] || [ -L "$unit_stage_root" ]; then
        remove_owned_tree "$unit_stage_root" "$user_unit_root" || return 1
        sync -f -- "$user_unit_root" || return 1
    fi
    if [ -e "$unit_backup_root" ] || [ -L "$unit_backup_root" ]; then
        remove_owned_tree "$unit_backup_root" "$user_unit_root" || return 1
    fi
    sync -f -- "$user_unit_root" || return 1
    rm -f -- "$prepared_journal" || return 1
    sync -f -- "$opt_root" || return 1
    prepared_journal_owned=0
    unit_stage_owned=0
    unit_backup_owned=0
    temporary_link_owned=0
    restore_link_owned=0
}

validate_prepared_journal() {
    journal_source=$1
    validate_prepared_journal_path "$prepared_journal" "$journal_source"
}

reconcile_prepared_journal_temporary() {
    [ -e "$prepared_journal_temporary" ] || [ -L "$prepared_journal_temporary" ] || return 0
    [ -f "$prepared_journal_temporary" ] && [ ! -L "$prepared_journal_temporary" ] &&
        [ "$(stat -c %u -- "$prepared_journal_temporary")" -eq "$(id -u)" ] &&
        [ "$(stat -c %h -- "$prepared_journal_temporary")" -eq 1 ] || return 1
    temporary_journal_source=$(sed -n 's/^source_commit=//p' "$prepared_journal_temporary" 2>/dev/null || printf invalid)
    temporary_journal_valid=1
    case "$temporary_journal_source" in
        *[!0-9a-f]*|'') temporary_journal_valid=0 ;;
        *) [ "${#temporary_journal_source}" -eq 40 ] &&
            [ -d "$release_root/$temporary_journal_source" ] &&
            [ ! -L "$release_root/$temporary_journal_source" ] || temporary_journal_valid=0 ;;
    esac
    if [ "$temporary_journal_valid" -eq 1 ]; then
        validate_prepared_journal_path "$prepared_journal_temporary" "$temporary_journal_source" || temporary_journal_valid=0
    fi
    if [ "$temporary_journal_valid" -eq 0 ]; then
        if [ -e "$prepared_journal" ] || [ -L "$prepared_journal" ]; then
            intent_source=$(sed -n 's/^source_commit=//p' "$prepared_journal" 2>/dev/null || printf invalid)
            validate_prepared_journal "$intent_source" || return 1
            case "$journal_phase" in
                preparing|prepared|source_backup_started|source_backup_verified|source_backup_invalidated|target_backup_started|target_backup_verified|activation_complete) ;;
                *) return 1 ;;
            esac
        else
            [ ! -e "$unit_stage_root" ] && [ ! -L "$unit_stage_root" ] &&
                [ ! -e "$unit_backup_root" ] && [ ! -L "$unit_backup_root" ] &&
                [ ! -e "$temporary_link" ] && [ ! -L "$temporary_link" ] &&
                [ ! -e "$restore_link" ] && [ ! -L "$restore_link" ] || return 1
        fi
        rm -f -- "$prepared_journal_temporary" && sync -f -- "$opt_root"
        return
    fi
    temporary_journal_phase=$journal_phase
    temporary_journal_recovery_sha=$journal_recovery_sha
    if [ -e "$prepared_journal" ] || [ -L "$prepared_journal" ]; then
        validate_prepared_journal "$temporary_journal_source" || return 1
        existing_journal_phase=$journal_phase
        existing_journal_recovery_sha=$journal_recovery_sha
        if [ "$existing_journal_phase:$temporary_journal_phase" != preparing:prepared ]; then
            [ "$existing_journal_recovery_sha" = "$temporary_journal_recovery_sha" ] || return 1
        fi
        case "$existing_journal_phase:$temporary_journal_phase" in
            preparing:prepared|prepared:source_backup_started|source_backup_started:source_backup_verified|source_backup_verified:source_backup_invalidated|source_backup_invalidated:source_backup_started|source_backup_verified:target_backup_started|target_backup_started:target_backup_verified|target_backup_verified:activation_complete)
                mv -T -- "$prepared_journal_temporary" "$prepared_journal" || {
                    [ ! -e "$prepared_journal_temporary" ] && [ ! -L "$prepared_journal_temporary" ] &&
                        validate_prepared_journal "$temporary_journal_source" &&
                        [ "$journal_phase" = "$temporary_journal_phase" ] || return 1
                }
                ;;
            preparing:preparing|prepared:prepared|source_backup_started:source_backup_started|source_backup_verified:source_backup_verified|source_backup_invalidated:source_backup_invalidated|target_backup_started:target_backup_started|target_backup_verified:target_backup_verified|activation_complete:activation_complete)
                rm -f -- "$prepared_journal_temporary" || return 1
                ;;
            *) return 1 ;;
        esac
    else
        mv -T -- "$prepared_journal_temporary" "$prepared_journal" || {
            [ ! -e "$prepared_journal_temporary" ] && [ ! -L "$prepared_journal_temporary" ] &&
                validate_prepared_journal "$temporary_journal_source" || return 1
        }
    fi
    prepared_journal_temporary_owned=0
    prepared_journal_owned=1
    sync -f -- "$opt_root" || return 1
    validate_prepared_journal "$temporary_journal_source"
}

write_prepared_journal() {
    journal_source=$1
    journal_recovery_sha=$(recovery_snapshot_sha256) || return 1
    if [ -e "$prepared_journal_temporary" ] || [ -L "$prepared_journal_temporary" ]; then
        [ -f "$prepared_journal_temporary" ] && [ ! -L "$prepared_journal_temporary" ] &&
            [ "$(stat -c %u -- "$prepared_journal_temporary")" -eq "$(id -u)" ] &&
            [ "$(stat -c %h -- "$prepared_journal_temporary")" -eq 1 ] || return 1
        rm -f -- "$prepared_journal_temporary" || return 1
        sync -f -- "$opt_root" || return 1
    fi
    prepared_journal_expected_text "$journal_source" "$journal_recovery_sha" prepared >"$prepared_journal_temporary" || return 1
    prepared_journal_temporary_owned=1
    chmod 0400 -- "$prepared_journal_temporary" || return 1
    sync -f -- "$prepared_journal_temporary" || return 1
    if [ -e "$prepared_journal" ] || [ -L "$prepared_journal" ]; then
        validate_prepared_journal "$journal_source" && [ "$journal_phase" = preparing ] || return 1
    fi
    if ! mv -T -- "$prepared_journal_temporary" "$prepared_journal"; then
        [ ! -e "$prepared_journal_temporary" ] && [ ! -L "$prepared_journal_temporary" ] &&
            validate_prepared_journal "$journal_source" || return 1
    fi
    prepared_journal_temporary_owned=0
    prepared_journal_owned=1
    sync -f -- "$opt_root" || return 1
    validate_prepared_journal "$journal_source"
}

advance_prepared_journal() {
    journal_source=$1
    expected_phase=$2
    next_phase=$3
    backup_boundary=${4:-none}
    validate_prepared_journal "$journal_source" && [ "$journal_phase" = "$expected_phase" ] || return 1
    journal_recovery_sha=$(sed -n 's/^recovery_snapshot_sha256=//p' "$prepared_journal") || return 1
    terminal_source_receipt_sha=$journal_source_receipt_sha
    terminal_target_receipt_sha=$journal_target_receipt_sha
    if [ -e "$prepared_journal_temporary" ] || [ -L "$prepared_journal_temporary" ]; then
        reconcile_prepared_journal_temporary || return 1
        validate_prepared_journal "$journal_source" && [ "$journal_phase" = "$next_phase" ] && return 0
        validate_prepared_journal "$journal_source" && [ "$journal_phase" = "$expected_phase" ] || return 1
        journal_recovery_sha=$(sed -n 's/^recovery_snapshot_sha256=//p' "$prepared_journal") || return 1
    fi
    case "$backup_boundary" in
        none) ;;
        source)
            case "$expected_phase:$next_phase" in
                prepared:source_backup_started|source_backup_invalidated:source_backup_started) ;;
                *) return 1 ;;
            esac
            journal_source_status_before=$(current_backup_status_sha256) || return 1
            journal_target_status_before=none
            ;;
        target)
            [ "$expected_phase:$next_phase" = source_backup_verified:target_backup_started ] || return 1
            journal_target_status_before=$(current_backup_status_sha256) || return 1
            ;;
        *) return 1 ;;
    esac
    if [ "$next_phase" = activation_complete ]; then
        prepared_journal_expected_text "$journal_source" "$journal_recovery_sha" "$next_phase" \
            "$terminal_source_receipt_sha" "$terminal_target_receipt_sha" \
            >"$prepared_journal_temporary" || return 1
    else
        prepared_journal_expected_text "$journal_source" "$journal_recovery_sha" "$next_phase" \
            >"$prepared_journal_temporary" || return 1
    fi
    prepared_journal_temporary_owned=1
    chmod 0400 -- "$prepared_journal_temporary" || return 1
    sync -f -- "$prepared_journal_temporary" || return 1
    if ! mv -T -- "$prepared_journal_temporary" "$prepared_journal"; then
        [ ! -e "$prepared_journal_temporary" ] && [ ! -L "$prepared_journal_temporary" ] &&
            validate_prepared_journal "$journal_source" && [ "$journal_phase" = "$next_phase" ] || return 1
    fi
    prepared_journal_temporary_owned=0
    sync -f -- "$opt_root" || return 1
    validate_prepared_journal "$journal_source" && [ "$journal_phase" = "$next_phase" ]
}

write_terminal_journal() {
    journal_source=$1
    advance_prepared_journal "$journal_source" target_backup_verified activation_complete
}

validate_active_target_state() {
    [ -L "$current_link" ] && [ "$(readlink -- "$current_link")" = "releases/$target_commit" ] || return 1
    for unit in robin-highscores.target robin-highscores-api.service robin-highscores-worker.service \
        robin-highscores-backup.service robin-highscores-backup.timer; do
        [ -f "$user_unit_root/$unit" ] && [ ! -L "$user_unit_root/$unit" ] &&
            [ "$(stat -c %u -- "$user_unit_root/$unit")" -eq "$(id -u)" ] &&
            [ "$(stat -c %h -- "$user_unit_root/$unit")" -eq 1 ] &&
            [ "$(stat -c %a -- "$user_unit_root/$unit")" = 440 ] &&
            cmp -s -- "$user_unit_root/$unit" "$target_release_root/systemd/user/$unit" || return 1
    done
    [ "$(systemctl --user is-enabled robin-highscores.target)" = enabled ] || return 1
    [ "$(systemctl --user is-enabled robin-highscores-backup.timer)" = enabled ] || return 1
    for unit in robin-highscores.target robin-highscores-api.service robin-highscores-worker.service \
        robin-highscores-backup.timer; do
        [ "$(systemctl --user show --property=ActiveState --value "$unit")" = active ] || return 1
    done
    [ "$(systemctl --user show --property=ActiveState --value robin-highscores-backup.service)" = inactive ] || return 1
    probe http://127.0.0.1:8787/healthz 5 && probe http://127.0.0.1:8787/readyz 5 || return 1
}

adopt_completed_rollback_without_journal() {
    validate_active_target_state || return 1
    for residual in "$prepared_journal" "$prepared_journal_temporary" \
        "$prebackup_receipt" "$prebackup_receipt_temporary" \
        "$target_backup_receipt" "$target_backup_receipt_temporary" \
        "$temporary_link" "$restore_link" "$unit_stage_root" "$unit_backup_root"; do
        [ ! -e "$residual" ] && [ ! -L "$residual" ] || return 1
    done
    sync -f -- "$user_unit_root" || return 1
    sync -f -- "$opt_root" || return 1
    validate_active_target_state
}

finish_terminal_cleanup() {
    journal_source=$1
    validate_active_target_state || return 1
    validate_prepared_journal "$journal_source" && [ "$journal_phase" = activation_complete ] || return 1
    terminal_recovery_sha=$journal_recovery_sha
    terminal_source_receipt_sha=$journal_source_receipt_sha
    terminal_target_receipt_sha=$journal_target_receipt_sha
    if [ -e "$temporary_link" ] || [ -L "$temporary_link" ]; then
        remove_tracked_link "$temporary_link" "releases/$target_commit" || return 1
    fi
    if [ -e "$restore_link" ] || [ -L "$restore_link" ]; then
        remove_tracked_link "$restore_link" "releases/$journal_source" || return 1
    fi
    if [ ! -e "$unit_backup_root" ] && [ ! -L "$unit_backup_root" ]; then
        [ ! -e "$unit_stage_root" ] && [ ! -L "$unit_stage_root" ] || return 1
    else
        [ "$(recovery_snapshot_sha256)" = "$terminal_recovery_sha" ] || return 1
    fi
    if [ -e "$unit_stage_root" ] || [ -L "$unit_stage_root" ]; then
        remove_owned_tree "$unit_stage_root" "$user_unit_root" || return 1
        sync -f -- "$user_unit_root" || return 1
    fi
    if [ -e "$unit_backup_root" ] || [ -L "$unit_backup_root" ]; then
        remove_owned_tree "$unit_backup_root" "$user_unit_root" || return 1
    fi
    sync -f -- "$user_unit_root" || return 1
    if [ ! -e "$target_backup_receipt" ] && [ ! -L "$target_backup_receipt" ]; then
        [ ! -e "$prebackup_receipt" ] && [ ! -L "$prebackup_receipt" ] || return 1
    fi
    if [ -e "$prebackup_receipt" ] || [ -L "$prebackup_receipt" ]; then
        validate_prebackup_receipt_identity "$journal_source" &&
            [ "$(durable_receipt_sha256 "$prebackup_receipt")" = "$terminal_source_receipt_sha" ] || return 1
        rm -f -- "$prebackup_receipt" || return 1
        sync -f -- "$opt_root" || return 1
    fi
    if [ -e "$target_backup_receipt" ] || [ -L "$target_backup_receipt" ]; then
        validate_backup_receipt_identity_path "$target_backup_receipt" "$target_commit" \
            "$target_vps_release_manifest_sha256" "$target_publication_lock_sha256" &&
            [ "$(durable_receipt_sha256 "$target_backup_receipt")" = "$terminal_target_receipt_sha" ] || return 1
        rm -f -- "$target_backup_receipt" || return 1
    fi
    sync -f -- "$opt_root" || return 1
    [ -f "$prepared_journal" ] && [ ! -L "$prepared_journal" ] &&
        [ "$(stat -c %u -- "$prepared_journal")" -eq "$(id -u)" ] &&
        [ "$(stat -c %h -- "$prepared_journal")" -eq 1 ] || return 1
    rm -f -- "$prepared_journal" || return 1
    prepared_journal_owned=0
    sync -f -- "$opt_root"
}

validate_backup_receipt_identity_path() {
    identity_receipt_path=$1
    identity_receipt_source=$2
    identity_receipt_vps_sha=$3
    identity_receipt_publication_sha=$4
    [ -f "$identity_receipt_path" ] && [ ! -L "$identity_receipt_path" ] &&
        [ "$(stat -c %u -- "$identity_receipt_path")" -eq "$(id -u)" ] &&
        [ "$(stat -c %h -- "$identity_receipt_path")" -eq 1 ] &&
        [ "$(stat -c %a -- "$identity_receipt_path")" = 400 ] || return 1
    identity_receipt_bytes=$(stat -c %s -- "$identity_receipt_path") || return 1
    [ "$identity_receipt_bytes" -gt 0 ] && [ "$identity_receipt_bytes" -le 131072 ] || return 1
    grep -Fq -- '"schema_version":2' "$identity_receipt_path" &&
        grep -Fq -- '"current_status":{' "$identity_receipt_path" &&
        grep -Fq -- "\"source_commit\":\"$identity_receipt_source\"" "$identity_receipt_path" &&
        grep -Fq -- "\"vps_release_manifest_sha256\":\"$identity_receipt_vps_sha\"" "$identity_receipt_path" &&
        grep -Fq -- "\"publication_lock_sha256\":\"$identity_receipt_publication_sha\"" "$identity_receipt_path"
}

run_transaction_backup_verifier() {
    receipt_root_fd=$1
    receipt_source=$2
    receipt_vps_sha=$3
    receipt_publication_sha=$4
    receipt_output=$5
    receipt_root=/proc/self/fd/$receipt_root_fd
    [ -f "$backup_status_path" ] && [ ! -L "$backup_status_path" ] &&
        [ "$(stat -c %u -- "$backup_status_path")" -eq "$(id -u)" ] &&
        [ "$(stat -c %h -- "$backup_status_path")" -eq 1 ] &&
        [ "$(stat -c %a -- "$backup_status_path")" = 400 ] || return 1
    "$receipt_root/bin/robin-highscores-admin" verify-transaction-backup \
        --backup-root-fd 3 \
        --status-envelope-fd 4 \
        --backup-authority-key-fd 5 \
        --expected-release-manifest-fd 6 \
        --expected-source-commit "$receipt_source" \
        --expected-vps-release-manifest-sha256 "$receipt_vps_sha" \
        --expected-publication-lock-sha256 "$receipt_publication_sha" \
        3< "$state_root/backups" \
        4< "$backup_status_path" \
        5< "$backup_authority_key" \
        6< "$receipt_root/$release_manifest_name" >"$receipt_output"
}

write_verified_backup_receipt() {
    receipt_root_fd=$1
    receipt_source=$2
    receipt_vps_sha=$3
    receipt_publication_sha=$4
    receipt_path=$5
    receipt_temporary=$6
    [ ! -e "$receipt_path" ] && [ ! -L "$receipt_path" ] &&
        [ ! -e "$receipt_temporary" ] && [ ! -L "$receipt_temporary" ] || return 1
    case "$receipt_temporary" in
        "$prebackup_receipt_temporary") prebackup_receipt_temporary_owned=1 ;;
        "$target_backup_receipt_temporary") target_backup_receipt_temporary_owned=1 ;;
        *) return 1 ;;
    esac
    run_transaction_backup_verifier "$receipt_root_fd" "$receipt_source" "$receipt_vps_sha" \
        "$receipt_publication_sha" "$receipt_temporary" || return 1
    chmod 0400 -- "$receipt_temporary" || return 1
    validate_backup_receipt_identity_path "$receipt_temporary" "$receipt_source" \
        "$receipt_vps_sha" "$receipt_publication_sha" || return 1
    sync -f -- "$receipt_temporary" || return 1
    mv -T -- "$receipt_temporary" "$receipt_path" || return 1
    case "$receipt_temporary" in
        "$prebackup_receipt_temporary") prebackup_receipt_temporary_owned=0 ;;
        "$target_backup_receipt_temporary") target_backup_receipt_temporary_owned=0 ;;
    esac
    sync -f -- "$opt_root" || return 1
    validate_backup_receipt_identity_path "$receipt_path" "$receipt_source" \
        "$receipt_vps_sha" "$receipt_publication_sha"
}

rerun_and_compare_backup_receipt() {
    receipt_root_fd=$1
    receipt_source=$2
    receipt_vps_sha=$3
    receipt_publication_sha=$4
    receipt_path=$5
    receipt_temporary=$6
    validate_backup_receipt_identity_path "$receipt_path" "$receipt_source" \
        "$receipt_vps_sha" "$receipt_publication_sha" || return 1
    [ ! -e "$receipt_temporary" ] && [ ! -L "$receipt_temporary" ] || return 1
    case "$receipt_temporary" in
        "$prebackup_receipt_temporary") prebackup_receipt_temporary_owned=1 ;;
        "$target_backup_receipt_temporary") target_backup_receipt_temporary_owned=1 ;;
        *) return 1 ;;
    esac
    run_transaction_backup_verifier "$receipt_root_fd" "$receipt_source" "$receipt_vps_sha" \
        "$receipt_publication_sha" "$receipt_temporary" || return 1
    chmod 0400 -- "$receipt_temporary" || return 1
    validate_backup_receipt_identity_path "$receipt_temporary" "$receipt_source" \
        "$receipt_vps_sha" "$receipt_publication_sha" || return 1
    cmp -s -- "$receipt_temporary" "$receipt_path" || return 1
    rm -f -- "$receipt_temporary" || return 1
    case "$receipt_temporary" in
        "$prebackup_receipt_temporary") prebackup_receipt_temporary_owned=0 ;;
        "$target_backup_receipt_temporary") target_backup_receipt_temporary_owned=0 ;;
    esac
    sync -f -- "$opt_root"
}

classify_backup_status_since() {
    status_boundary=$1
    current_status_sha=$(current_backup_status_sha256) || return 1
    valid_status_boundary "$current_status_sha" || return 1
    if [ "$current_status_sha" = "$status_boundary" ]; then
        backup_status_change=unchanged
    else
        backup_status_change=changed
    fi
}

wait_for_backup_service_inactive() {
    backup_wait=0
    while [ "$backup_wait" -lt 300 ]; do
        backup_state=$(systemctl --user show --property=ActiveState --value \
            robin-highscores-backup.service) || return 1
        case "$backup_state" in
            inactive) return 0 ;;
            active|activating|deactivating) backup_wait=$((backup_wait + 1)); sleep 1 ;;
            *) return 1 ;;
        esac
    done
    return 1
}

require_backup_capacity() {
    capacity_release_root=$1
    "$capacity_release_root/bin/robin-highscores-admin" \
        --config "$capacity_release_root/config/highscores-server.toml" estimate-backup-space \
        --release-manifest-path "$capacity_release_root/$release_manifest_name" \
        --backup-root "$state_root/backups" \
        --status-path "$backup_status_path" \
        --require-available \
        --restore-source-map "$secret_root/cursor-hmac.key=$secret_root/cursor-hmac.key" \
        --restore-source-map "$secret_root/competition-run-grant.key=$secret_root/competition-run-grant.key" \
        --restore-source-map "$secret_root/run-preflight-grant.key=$secret_root/run-preflight-grant.key" \
        --restore-source-map "$secret_root/moderation-bearer.token=$secret_root/moderation-bearer.token" \
        --restore-source-map "$user_unit_root/robin-highscores.target=$user_unit_root/robin-highscores.target" \
        --restore-source-map "$user_unit_root/robin-highscores-api.service=$user_unit_root/robin-highscores-api.service" \
        --restore-source-map "$user_unit_root/robin-highscores-worker.service=$user_unit_root/robin-highscores-worker.service" \
        --restore-source-map "$user_unit_root/robin-highscores-backup.service=$user_unit_root/robin-highscores-backup.service" \
        --restore-source-map "$user_unit_root/robin-highscores-backup.timer=$user_unit_root/robin-highscores-backup.timer" \
        >/dev/null
}

validate_prebackup_receipt_identity() {
    receipt_source=$1
    validate_backup_receipt_identity_path "$prebackup_receipt" "$receipt_source" \
        "$source_vps_release_manifest_sha256" "$source_publication_lock_sha256"
}

validate_prebackup_receipt() {
    receipt_source=$1
    rerun_and_compare_backup_receipt "$source_release_root_fd" "$receipt_source" \
        "$source_vps_release_manifest_sha256" "$source_publication_lock_sha256" \
        "$prebackup_receipt" "$prebackup_receipt_temporary"
}

reconcile_prebackup_receipt_temporary() {
    receipt_source=$1
    [ -e "$prebackup_receipt_temporary" ] || [ -L "$prebackup_receipt_temporary" ] || return 0
    if [ -e "$prebackup_receipt" ] || [ -L "$prebackup_receipt" ]; then
        validate_prebackup_receipt_identity "$receipt_source" || return 1
    fi
    [ -f "$prebackup_receipt_temporary" ] && [ ! -L "$prebackup_receipt_temporary" ] &&
        [ "$(stat -c %u -- "$prebackup_receipt_temporary")" -eq "$(id -u)" ] &&
        [ "$(stat -c %h -- "$prebackup_receipt_temporary")" -eq 1 ] || return 1
    rm -f -- "$prebackup_receipt_temporary" && sync -f -- "$opt_root"
}

write_prebackup_receipt() {
    receipt_source=$1
    write_verified_backup_receipt "$source_release_root_fd" "$receipt_source" \
        "$source_vps_release_manifest_sha256" "$source_publication_lock_sha256" \
        "$prebackup_receipt" "$prebackup_receipt_temporary"
}

validate_completed_backup_receipts() {
    validate_prepared_journal "$previous_commit" || return 1
    case "$journal_phase" in target_backup_verified|activation_complete) ;; *) return 1 ;; esac
    validate_prebackup_receipt_identity "$previous_commit" || return 1
    [ "$(durable_receipt_sha256 "$prebackup_receipt")" = "$journal_source_receipt_sha" ] || return 1
    validate_backup_receipt_identity_path "$target_backup_receipt" "$target_commit" \
        "$target_vps_release_manifest_sha256" "$target_publication_lock_sha256" || return 1
    [ "$(durable_receipt_sha256 "$target_backup_receipt")" = "$journal_target_receipt_sha" ] || return 1
    run_transaction_backup_verifier "$target_release_root_fd" "$target_commit" \
        "$target_vps_release_manifest_sha256" "$target_publication_lock_sha256" /dev/null || return 1
    ! cmp -s -- "$prebackup_receipt" "$target_backup_receipt"
}

converge_source_units_stopped() {
    for unit in robin-highscores.target robin-highscores-api.service robin-highscores-worker.service \
        robin-highscores-backup.service robin-highscores-backup.timer; do
        destination=$user_unit_root/$unit
        recovery_unit=$unit_backup_root/units/$unit
        interrupted_restore=$unit_stage_root/.restore-$unit
        source_stage=$unit_stage_root/.resume-source-$unit
        [ -f "$recovery_unit" ] && [ ! -L "$recovery_unit" ] &&
            [ "$(stat -c %u -- "$recovery_unit")" -eq "$(id -u)" ] &&
            [ "$(stat -c %h -- "$recovery_unit")" -eq 1 ] &&
            [ "$(stat -c %a -- "$recovery_unit")" = 440 ] &&
            cmp -s -- "$recovery_unit" "$source_release_root/systemd/user/$unit" || return 1
        [ -f "$destination" ] && [ ! -L "$destination" ] &&
            [ "$(stat -c %u -- "$destination")" -eq "$(id -u)" ] &&
            [ "$(stat -c %h -- "$destination")" -eq 1 ] &&
            [ "$(stat -c %a -- "$destination")" = 440 ] &&
            { cmp -s -- "$destination" "$recovery_unit" ||
                cmp -s -- "$destination" "$target_release_root/systemd/user/$unit"; } || return 1
        for source_temporary in "$interrupted_restore" "$source_stage"; do
            if [ -e "$source_temporary" ] || [ -L "$source_temporary" ]; then
                [ -f "$source_temporary" ] && [ ! -L "$source_temporary" ] &&
                    [ "$(stat -c %u -- "$source_temporary")" -eq "$(id -u)" ] &&
                    [ "$(stat -c %h -- "$source_temporary")" -eq 1 ] || return 1
                if ! cmp -s -- "$source_temporary" "$recovery_unit"; then
                    rm -f -- "$source_temporary" || return 1
                    sync -f -- "$unit_stage_root" || return 1
                fi
            fi
        done
        if ! cmp -s -- "$destination" "$recovery_unit"; then
            if [ -f "$interrupted_restore" ]; then
                mv -T -- "$interrupted_restore" "$destination" || return 1
            else
                if [ ! -f "$source_stage" ]; then
                    install -m 0440 -- "$recovery_unit" "$source_stage" || return 1
                    sync -f -- "$source_stage" || return 1
                fi
                mv -T -- "$source_stage" "$destination" || return 1
            fi
        fi
        for source_temporary in "$interrupted_restore" "$source_stage"; do
            if [ -e "$source_temporary" ] || [ -L "$source_temporary" ]; then
                [ -f "$source_temporary" ] && [ ! -L "$source_temporary" ] &&
                    cmp -s -- "$source_temporary" "$recovery_unit" &&
                    rm -f -- "$source_temporary" || return 1
            fi
        done
        sync -f -- "$destination" || return 1
    done
    sync -f -- "$unit_stage_root" || return 1
    sync -f -- "$user_unit_root" || return 1
    systemctl --user daemon-reload || return 1
    for unit in robin-highscores.target robin-highscores-api.service robin-highscores-worker.service \
        robin-highscores-backup.service robin-highscores-backup.timer; do
        cmp -s -- "$user_unit_root/$unit" "$source_release_root/systemd/user/$unit" || return 1
        [ "$(systemctl --user show --property=ActiveState --value "$unit")" = inactive ] || return 1
    done
}

restore_previous_state() {
    restore_failed=0
    for unit in robin-highscores.target robin-highscores-api.service robin-highscores-worker.service \
        robin-highscores-backup.service robin-highscores-backup.timer; do
        destination=$user_unit_root/$unit
        if [ -f "$unit_backup_root/units/$unit" ] && [ ! -L "$unit_backup_root/units/$unit" ]; then
            if [ -e "$destination" ] || [ -L "$destination" ]; then
                [ -f "$destination" ] && [ ! -L "$destination" ] &&
                    [ "$(stat -c %u -- "$destination")" -eq "$(id -u)" ] &&
                    [ "$(stat -c %h -- "$destination")" -eq 1 ] &&
                    [ "$(stat -c %a -- "$destination")" = 440 ] &&
                    { cmp -s -- "$destination" "$unit_backup_root/units/$unit" ||
                        cmp -s -- "$destination" "$target_release_root/systemd/user/$unit"; } || {
                    restore_failed=1
                    continue
                }
            fi
            restore_temporary=$unit_stage_root/.restore-$unit
            [ ! -e "$restore_temporary" ] && [ ! -L "$restore_temporary" ] || { restore_failed=1; continue; }
            install -m 0440 -- "$unit_backup_root/units/$unit" "$restore_temporary" || { restore_failed=1; continue; }
            mv -T -- "$restore_temporary" "$destination" || restore_failed=1
        elif [ -f "$unit_backup_root/absent/$unit" ]; then
            if [ -e "$destination" ] || [ -L "$destination" ]; then
                [ -f "$destination" ] && [ ! -L "$destination" ] &&
                    [ "$(stat -c %u -- "$destination")" -eq "$(id -u)" ] &&
                    [ "$(stat -c %h -- "$destination")" -eq 1 ] &&
                    [ "$(stat -c %a -- "$destination")" = 440 ] &&
                    cmp -s -- "$destination" "$target_release_root/systemd/user/$unit" &&
                    rm -f -- "$destination" || restore_failed=1
            fi
        else
            restore_failed=1
        fi
    done
    if [ -n "$previous_target" ]; then
        if [ -L "$current_link" ]; then
            actual_target=$(readlink -- "$current_link")
            [ "$actual_target" = "$previous_target" ] || [ "$actual_target" = "releases/$target_commit" ] || restore_failed=1
        elif [ -e "$current_link" ]; then
            restore_failed=1
        fi
        [ "$restore_failed" -ne 0 ] || mv -T -- "$restore_link" "$current_link" || restore_failed=1
    elif [ -e "$current_link" ] || [ -L "$current_link" ]; then
        [ -L "$current_link" ] && [ "$(readlink -- "$current_link")" = "releases/$target_commit" ] &&
            rm -f -- "$current_link" || restore_failed=1
    fi
    sync -f -- "$user_unit_root" || restore_failed=1
    sync -f -- "$opt_root" || restore_failed=1
    systemctl --user daemon-reload || restore_failed=1
    if [ -n "$previous_commit" ]; then
        for unit in robin-highscores.target robin-highscores-backup.timer; do
            if [ -f "$unit_backup_root/enabled/$unit" ]; then
                saved_enabled_state=$(sed -n '1p' "$unit_backup_root/enabled/$unit" 2>/dev/null || printf invalid)
                case "$saved_enabled_state" in
                    enabled) systemctl --user enable "$unit" >/dev/null || restore_failed=1 ;;
                    enabled-runtime) systemctl --user enable --runtime "$unit" >/dev/null || restore_failed=1 ;;
                    *) restore_failed=1 ;;
                esac
            else
                systemctl --user disable "$unit" >/dev/null || restore_failed=1
            fi
        done
        for unit in robin-highscores.target robin-highscores-api.service robin-highscores-worker.service \
            robin-highscores-backup.timer robin-highscores-backup.service; do
            systemctl --user stop "$unit" >/dev/null 2>&1 || restore_failed=1
        done
        for unit in robin-highscores.target robin-highscores-api.service robin-highscores-worker.service \
            robin-highscores-backup.timer robin-highscores-backup.service; do
            [ ! -f "$unit_backup_root/active/$unit" ] || systemctl --user start "$unit" || restore_failed=1
        done
        for unit in robin-highscores.target robin-highscores-api.service robin-highscores-worker.service \
            robin-highscores-backup.timer robin-highscores-backup.service; do
            if [ ! -f "$unit_backup_root/active/$unit" ] && [ "$unit" != robin-highscores-backup.service ]; then
                systemctl --user stop "$unit" >/dev/null 2>&1 || restore_failed=1
            fi
        done
        for wants_directory in "$user_unit_root/default.target.wants" "$user_unit_root/timers.target.wants"; do
            if [ -e "$wants_directory" ] || [ -L "$wants_directory" ]; then
                [ -d "$wants_directory" ] && [ ! -L "$wants_directory" ] &&
                    sync -f -- "$wants_directory" || restore_failed=1
            fi
        done
        if [ "$resume_source_backup_started" -eq 0 ] &&
            [ -f "$unit_backup_root/active/robin-highscores-api.service" ]; then
            probe http://127.0.0.1:8787/healthz 5 || restore_failed=1
            probe http://127.0.0.1:8787/readyz 5 || restore_failed=1
        fi
    fi
    return "$restore_failed"
}

converge_target_selected_stopped() {
    converge_failed=0
    systemctl --user disable robin-highscores.target >/dev/null 2>&1 || converge_failed=1
    systemctl --user disable robin-highscores-backup.timer >/dev/null 2>&1 || converge_failed=1
    for unit in robin-highscores.target robin-highscores-worker.service robin-highscores-api.service \
        robin-highscores-backup.timer robin-highscores-backup.service; do
        systemctl --user stop "$unit" >/dev/null 2>&1 || converge_failed=1
    done
    for unit in robin-highscores.target robin-highscores-api.service robin-highscores-worker.service \
        robin-highscores-backup.service robin-highscores-backup.timer; do
        destination=$user_unit_root/$unit
        staged_unit=$unit_stage_root/units/$unit
        destination_allowed=0
        if [ ! -e "$destination" ] && [ ! -L "$destination" ]; then
            destination_allowed=1
        elif [ -f "$destination" ] && [ ! -L "$destination" ] &&
            [ "$(stat -c %u -- "$destination")" -eq "$(id -u)" ] &&
            [ "$(stat -c %h -- "$destination")" -eq 1 ] &&
            [ "$(stat -c %a -- "$destination")" = 440 ]; then
            if cmp -s -- "$destination" "$target_release_root/systemd/user/$unit" ||
                cmp -s -- "$destination" "$unit_backup_root/units/$unit"; then
                destination_allowed=1
            fi
        fi
        if [ "$destination_allowed" -ne 1 ]; then
            converge_failed=1
            continue
        fi
        if [ -f "$staged_unit" ] && [ ! -L "$staged_unit" ] &&
            [ "$(stat -c %u -- "$staged_unit")" -eq "$(id -u)" ] &&
            [ "$(stat -c %h -- "$staged_unit")" -eq 1 ] &&
            [ "$(stat -c %a -- "$staged_unit")" = 440 ] &&
            cmp -s -- "$staged_unit" "$target_release_root/systemd/user/$unit"; then
            if ! mv -T -- "$staged_unit" "$destination"; then
                if [ -e "$staged_unit" ] || [ -L "$staged_unit" ] ||
                    [ ! -f "$destination" ] || [ -L "$destination" ] ||
                    ! cmp -s -- "$destination" "$target_release_root/systemd/user/$unit"; then
                    converge_failed=1
                    continue
                fi
            fi
        elif [ ! -f "$destination" ] || [ -L "$destination" ] ||
            [ "$(stat -c %u -- "$destination")" -ne "$(id -u)" ] ||
            [ "$(stat -c %h -- "$destination")" -ne 1 ] ||
            [ "$(stat -c %a -- "$destination")" != 440 ] ||
            ! cmp -s -- "$destination" "$target_release_root/systemd/user/$unit"; then
            converge_failed=1
            continue
        fi
        sync -f -- "$destination" || converge_failed=1
    done
    sync -f -- "$user_unit_root" || converge_failed=1
    systemctl --user daemon-reload || converge_failed=1
    if [ -L "$current_link" ] && [ "$(readlink -- "$current_link")" = "releases/$target_commit" ]; then
        :
    else
        current_replaceable=0
        if [ ! -e "$current_link" ] && [ ! -L "$current_link" ]; then
            current_replaceable=1
        elif [ -n "$previous_target" ] && [ -L "$current_link" ] &&
            [ "$(stat -c %u -- "$current_link")" -eq "$(id -u)" ] &&
            [ "$(readlink -- "$current_link")" = "$previous_target" ]; then
            current_replaceable=1
        fi
        if [ "$current_replaceable" -eq 1 ]; then
            if [ ! -e "$temporary_link" ] && [ ! -L "$temporary_link" ]; then
                if ln -s -- "releases/$target_commit" "$temporary_link"; then
                    temporary_link_owned=1
                    sync -f -- "$opt_root" || converge_failed=1
                else
                    converge_failed=1
                fi
            fi
            if [ "$temporary_link_owned" -eq 1 ] && [ -L "$temporary_link" ] &&
                [ "$(stat -c %u -- "$temporary_link")" -eq "$(id -u)" ] &&
                [ "$(readlink -- "$temporary_link")" = "releases/$target_commit" ]; then
                if ! mv -T -- "$temporary_link" "$current_link"; then
                    [ -L "$current_link" ] && [ "$(readlink -- "$current_link")" = "releases/$target_commit" ] &&
                        [ ! -e "$temporary_link" ] && [ ! -L "$temporary_link" ] || converge_failed=1
                fi
            else
                converge_failed=1
            fi
            if [ ! -e "$temporary_link" ] && [ ! -L "$temporary_link" ]; then temporary_link_owned=0; fi
        else
            converge_failed=1
        fi
    fi
    sync -f -- "$opt_root" || converge_failed=1
    for wants_directory in "$user_unit_root/default.target.wants" "$user_unit_root/timers.target.wants"; do
        if [ -e "$wants_directory" ] || [ -L "$wants_directory" ]; then
            [ -d "$wants_directory" ] && [ ! -L "$wants_directory" ] &&
                sync -f -- "$wants_directory" || converge_failed=1
        fi
    done
    for unit in robin-highscores.target robin-highscores-worker.service robin-highscores-api.service \
        robin-highscores-backup.timer robin-highscores-backup.service; do
        active_state=$(systemctl --user show --property=ActiveState --value "$unit" 2>/dev/null || printf 'unknown')
        [ "$active_state" = inactive ] || converge_failed=1
    done
    for unit in robin-highscores.target robin-highscores-backup.timer; do
        if enabled_state=$(systemctl --user is-enabled "$unit" 2>/dev/null); then
            [ "$enabled_state" = disabled ] || converge_failed=1
        else
            [ "$enabled_state" = disabled ] || converge_failed=1
        fi
    done
    return "$converge_failed"
}

cleanup() {
    status=$?
    trap - EXIT HUP INT TERM
    cleanup_failed=0
    if [ "$status" -ne 0 ] && [ "$activation_started" -eq 0 ] &&
        [ "$prebackup_timer_disabled" -eq 1 ]; then
        if [ -f "$unit_backup_root/enabled/robin-highscores-backup.timer" ]; then
            saved_enabled_state=$(sed -n '1p' "$unit_backup_root/enabled/robin-highscores-backup.timer" 2>/dev/null || printf invalid)
            case "$saved_enabled_state" in
                enabled) systemctl --user enable robin-highscores-backup.timer >/dev/null || cleanup_failed=1 ;;
                enabled-runtime) systemctl --user enable --runtime robin-highscores-backup.timer >/dev/null || cleanup_failed=1 ;;
                *) cleanup_failed=1 ;;
            esac
        else
            systemctl --user disable robin-highscores-backup.timer >/dev/null || cleanup_failed=1
        fi
        if [ -f "$unit_backup_root/active/robin-highscores-backup.timer" ]; then
            systemctl --user start robin-highscores-backup.timer || cleanup_failed=1
        else
            systemctl --user stop robin-highscores-backup.timer || cleanup_failed=1
        fi
        if [ -d "$user_unit_root/timers.target.wants" ] && [ ! -L "$user_unit_root/timers.target.wants" ]; then
            sync -f -- "$user_unit_root/timers.target.wants" || cleanup_failed=1
        fi
    fi
    if [ "$status" -ne 0 ] && [ "$activation_started" -eq 1 ] &&
        [ "$transaction_prepared" -eq 1 ] && [ "$rollback_complete" -eq 0 ]; then
        leave_source_selected_stopped=0
        if validate_prepared_journal "$previous_commit"; then
            case "$journal_phase" in
                source_backup_started|source_backup_invalidated|target_backup_started|target_backup_verified)
                    leave_source_selected_stopped=1
                    ;;
            esac
        else
            cleanup_failed=1
            preserve_recovery=1
            leave_source_selected_stopped=1
        fi
        prior_selector_proven=0
        if [ "$selection_attempted" -eq 1 ] && [ -L "$current_link" ] &&
            [ "$(readlink -- "$current_link" 2>/dev/null || printf unknown)" = "$previous_target" ] &&
            [ -L "$temporary_link" ] && [ "$(readlink -- "$temporary_link" 2>/dev/null || printf unknown)" = "releases/$target_commit" ]; then
            prior_selector_proven=1
        fi
        if [ "$target_selected" -eq 0 ] && [ "$leave_source_selected_stopped" -eq 1 ]; then
            [ -L "$current_link" ] && [ "$(readlink -- "$current_link")" = "$previous_target" ] || cleanup_failed=1
            systemctl --user disable robin-highscores.target >/dev/null 2>&1 || cleanup_failed=1
            systemctl --user disable robin-highscores-backup.timer >/dev/null 2>&1 || cleanup_failed=1
            for unit in robin-highscores.target robin-highscores-worker.service robin-highscores-api.service \
                robin-highscores-backup.timer robin-highscores-backup.service; do
                systemctl --user stop "$unit" >/dev/null 2>&1 || cleanup_failed=1
            done
            preserve_recovery=1
            echo "rollback target backup was not authenticated; source remains selected and stopped for resume" >&2
        elif [ "$target_selected" -eq 0 ] &&
            { [ "$selection_attempted" -eq 0 ] || [ "$prior_selector_proven" -eq 1 ]; }; then
            source_receipt_invalidated=0
            if validate_prepared_journal "$previous_commit" && [ "$journal_phase" = source_backup_verified ]; then
                if advance_prepared_journal "$previous_commit" source_backup_verified source_backup_invalidated; then
                    source_receipt_invalidated=1
                    preserve_recovery=1
                else
                    echo "rollback cleanup could not durably invalidate the reusable source BackupV4 receipt" >&2
                    cleanup_failed=1
                    preserve_recovery=1
                fi
            fi
            if [ "$cleanup_failed" -ne 0 ]; then
                systemctl --user disable robin-highscores.target >/dev/null 2>&1 || :
                systemctl --user disable robin-highscores-backup.timer >/dev/null 2>&1 || :
                for unit in robin-highscores.target robin-highscores-worker.service robin-highscores-api.service \
                    robin-highscores-backup.timer robin-highscores-backup.service; do
                    systemctl --user stop "$unit" >/dev/null 2>&1 || :
                done
            elif ! restore_previous_state; then
                echo "rollback cleanup could not restore the prior state" >&2
                preserve_recovery=1
                cleanup_failed=1
                systemctl --user disable robin-highscores.target >/dev/null 2>&1 || :
                systemctl --user disable robin-highscores-backup.timer >/dev/null 2>&1 || :
                for unit in robin-highscores.target robin-highscores-worker.service robin-highscores-api.service \
                    robin-highscores-backup.timer robin-highscores-backup.service; do
                    systemctl --user stop "$unit" >/dev/null 2>&1 || :
                done
            fi
            if [ "$source_receipt_invalidated" -eq 1 ] && [ "$cleanup_failed" -eq 0 ]; then
                echo "rollback source BackupV4 receipt was durably invalidated before source restart; resume requires a fresh backup" >&2
            fi
        else
            converge_target_selected_stopped || {
                echo "rollback cleanup could not converge the selected target to a coherent stopped state" >&2
                preserve_recovery=1
                cleanup_failed=1
            }
            if [ "$preserve_recovery" -eq 0 ]; then
                preserve_recovery=1
                echo "rollback target was selected; it remains selected but stopped for explicit authenticated resume" >&2
            fi
        fi
    fi
    if [ "$prepared_journal_owned" -eq 1 ]; then
        cleanup_journal_source=$(sed -n 's/^source_commit=//p' "$prepared_journal" 2>/dev/null || printf invalid)
        if validate_prepared_journal "$cleanup_journal_source"; then
            if [ "$journal_phase" = preparing ] ||
                { [ "$status" -ne 0 ] && [ "$rollback_mode" = resume ]; }; then
                preserve_recovery=1
            fi
        elif [ "$status" -ne 0 ]; then
            preserve_recovery=1
            cleanup_failed=1
        fi
    fi
    [ "$temporary_link_owned" -eq 0 ] || [ "$preserve_recovery" -eq 1 ] || remove_tracked_link "$temporary_link" "releases/$target_commit" || cleanup_failed=1
    [ "$restore_link_owned" -eq 0 ] || [ "$preserve_recovery" -eq 1 ] || remove_tracked_link "$restore_link" "$previous_target" || cleanup_failed=1
    if [ "$prepared_journal_owned" -eq 1 ] && [ "$preserve_recovery" -eq 0 ]; then
        validate_prepared_journal "$previous_commit" || cleanup_failed=1
        if [ "$cleanup_failed" -eq 0 ]; then
            if [ -e "$prebackup_receipt" ] || [ -L "$prebackup_receipt" ]; then
                validate_prebackup_receipt_identity "$previous_commit" &&
                    rm -f -- "$prebackup_receipt" || cleanup_failed=1
            fi
            if [ "$cleanup_failed" -eq 0 ]; then
                rm -f -- "$prepared_journal" || cleanup_failed=1
                prepared_journal_owned=0
                sync -f -- "$opt_root" || cleanup_failed=1
            fi
        fi
        if [ "$cleanup_failed" -ne 0 ]; then preserve_recovery=1; fi
    fi
    [ "$unit_stage_owned" -eq 0 ] || [ "$preserve_recovery" -eq 1 ] || remove_owned_tree "$unit_stage_root" "$user_unit_root" || cleanup_failed=1
    [ "$unit_backup_owned" -eq 0 ] || [ "$preserve_recovery" -eq 1 ] || remove_owned_tree "$unit_backup_root" "$user_unit_root" || cleanup_failed=1
    if [ "$prebackup_receipt_temporary_owned" -eq 1 ]; then
        [ -f "$prebackup_receipt_temporary" ] && [ ! -L "$prebackup_receipt_temporary" ] &&
            [ "$(stat -c %u -- "$prebackup_receipt_temporary")" -eq "$(id -u)" ] &&
            [ "$(stat -c %h -- "$prebackup_receipt_temporary")" -eq 1 ] &&
            rm -f -- "$prebackup_receipt_temporary" && sync -f -- "$opt_root" || cleanup_failed=1
    fi
    if [ "$target_backup_receipt_temporary_owned" -eq 1 ]; then
        [ -f "$target_backup_receipt_temporary" ] && [ ! -L "$target_backup_receipt_temporary" ] &&
            [ "$(stat -c %u -- "$target_backup_receipt_temporary")" -eq "$(id -u)" ] &&
            [ "$(stat -c %h -- "$target_backup_receipt_temporary")" -eq 1 ] &&
            rm -f -- "$target_backup_receipt_temporary" && sync -f -- "$opt_root" || cleanup_failed=1
    fi
    if [ "$prepared_journal_temporary_owned" -eq 1 ]; then
        [ -f "$prepared_journal_temporary" ] && [ ! -L "$prepared_journal_temporary" ] &&
            [ "$(stat -c %u -- "$prepared_journal_temporary")" -eq "$(id -u)" ] &&
            [ "$(stat -c %h -- "$prepared_journal_temporary")" -eq 1 ] &&
            rm -f -- "$prepared_journal_temporary" && sync -f -- "$opt_root" || cleanup_failed=1
    fi
    if [ "$cleanup_failed" -ne 0 ]; then
        echo "rollback cleanup failed; inspect exact private paths before retry" >&2
        [ "$preserve_recovery" -eq 0 ] || echo "preserved recovery artifacts: $unit_backup_root $unit_stage_root $restore_link" >&2
        [ "$status" -ne 0 ] || status=1
    fi
    exit "$status"
}
trap cleanup EXIT
trap 'trap - HUP; exit 129' HUP
trap 'trap - INT; exit 130' INT
trap 'trap - TERM; exit 143' TERM

[ -d "$opt_root" ] && [ ! -L "$opt_root" ] && [ "$(realpath -e -- "$opt_root")" = "$opt_root" ] &&
    [ "$(stat -c %u -- "$opt_root")" -eq "$(id -u)" ] && [ "$(stat -c %a -- "$opt_root")" = 750 ] || fail "unsafe install root"
[ -d "$release_root" ] && [ ! -L "$release_root" ] && [ "$(realpath -e -- "$release_root")" = "$release_root" ] &&
    [ "$(stat -c %u -- "$release_root")" -eq "$(id -u)" ] && [ "$(stat -c %a -- "$release_root")" = 750 ] || fail "unsafe release root"
[ -d "$user_unit_root" ] && [ ! -L "$user_unit_root" ] && [ "$(realpath -e -- "$user_unit_root")" = "$user_unit_root" ] &&
    [ "$(stat -c %u -- "$user_unit_root")" -eq "$(id -u)" ] && [ "$(stat -c %a -- "$user_unit_root")" = 750 ] || fail "unsafe user unit root"
if /usr/bin/flock -n "$activation_lock" /bin/true; then
    fail "canonical activation lock was not already held by the outer transaction"
fi
/usr/bin/flock -n "$activation_lock_number" ||
    fail "another deploy or rollback holds the inherited activation lock"
[ "$(stat -Lc %d:%i -- "$activation_lock_fd")" = "$activation_lock_identity" ] ||
    fail "inherited activation lock descriptor changed"

for managed_directory in "$state_root" "$state_root/database" "$state_root/replays" \
    "$state_root/campaign-states" "$state_root/backups" "$state_root/status" "$secret_root" \
    "$state_root/runtime-fence" "$raw_root"; do
    [ -d "$managed_directory" ] && [ ! -L "$managed_directory" ] &&
        [ "$(realpath -e -- "$managed_directory")" = "$managed_directory" ] &&
        [ "$(stat -c %u -- "$managed_directory")" -eq "$(id -u)" ] ||
        fail "rollback prerequisite directory is unsafe: $managed_directory"
done
for secret_name in cursor-hmac.key competition-run-grant.key run-preflight-grant.key \
    moderation-bearer.token backup-authority-hmac.key; do
    secret_path=$secret_root/$secret_name
    [ -f "$secret_path" ] && [ ! -L "$secret_path" ] &&
        [ "$(stat -c %u -- "$secret_path")" -eq "$(id -u)" ] &&
        [ "$(stat -c %h -- "$secret_path")" -eq 1 ] &&
        [ "$(stat -c %a -- "$secret_path")" = 400 ] || fail "rollback secret is unsafe: $secret_path"
done
[ "$(stat -c %s -- "$backup_authority_key")" -eq 32 ] ||
    fail "rollback backup-authority key must contain exactly 32 bytes"
runtime_fence=$state_root/runtime-fence
[ "$(stat -c %a -- "$runtime_fence")" = 500 ] ||
    fail "rollback runtime fence does not have exact mode 0500"
runtime_fence_device=$(stat -c %d -- "$runtime_fence") || fail "cannot inspect runtime fence device"
for fence_leaf in db-admission.lock db-quiescence.lock; do
    fence_path=$runtime_fence/$fence_leaf
    [ -f "$fence_path" ] && [ ! -L "$fence_path" ] &&
        [ "$(stat -c %u -- "$fence_path")" -eq "$(id -u)" ] &&
        [ "$(stat -c %d -- "$fence_path")" = "$runtime_fence_device" ] &&
        [ "$(stat -c %h -- "$fence_path")" -eq 1 ] &&
        [ "$(stat -c %a -- "$fence_path")" = 400 ] &&
        [ "$(stat -c %s -- "$fence_path")" -eq 0 ] ||
        fail "rollback runtime fence leaf is not exact: $fence_path"
done
[ "$(stat -c %a -- "$raw_root")" = 550 ] || fail "rollback raw-content parent does not have exact mode 0550"
validate_owned_tree "$raw_root" || fail "rollback raw-content authority crosses mounts/devices or has unsafe topology"
for edition in demo full; do
    edition_root=$raw_root/$edition
    [ -d "$edition_root" ] && [ ! -L "$edition_root" ] || fail "rollback raw-content edition is missing: $edition_root"
    [ "$(stat -c %a -- "$edition_root")" = 550 ] || fail "rollback raw-content edition does not have exact mode 0550: $edition_root"
    raw_violation=$(find "$edition_root" -xdev -mindepth 0 -perm /222 -print -quit) || fail "could not scan rollback raw-content modes"
    [ -z "$raw_violation" ] || fail "rollback raw-content authority is writable"
    raw_violation=$(find "$edition_root" -xdev -type d ! -perm 0550 -print -quit) || fail "could not scan rollback raw directory modes"
    [ -z "$raw_violation" ] || fail "rollback raw-content contains a directory without exact mode 0550"
    raw_violation=$(find "$edition_root" -xdev -type f ! -perm 0440 -print -quit) || fail "could not scan rollback raw file modes"
    [ -z "$raw_violation" ] || fail "rollback raw-content contains a file without exact mode 0440"
    raw_file=$(find "$edition_root" -xdev -type f -print -quit) || fail "could not scan rollback raw-content files"
    [ -n "$raw_file" ] || fail "rollback raw-content edition is empty: $edition_root"
done

if [ -e "$current_link" ] || [ -L "$current_link" ]; then
    [ -L "$current_link" ] || fail "current selector is not a symlink"
    previous_target=$(readlink -- "$current_link")
    case "$previous_target" in
        releases/[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f])
            previous_commit=${previous_target#releases/}
            [ -d "$release_root/$previous_commit" ] && [ ! -L "$release_root/$previous_commit" ] || fail "current release is missing"
            ;;
        *) fail "unsafe current selector" ;;
    esac
fi
[ -n "$previous_commit" ] || fail "rollback requires an existing selected release; use deploy for first activation"
selected_commit=$previous_commit

if [ "$selected_commit" != "$target_commit" ]; then
    retain_source_release_authority ||
        fail "could not retain and authenticate the exact currently selected V2 release"
else
    validate_pinned_release_root "$target_release_root_fd" "$target_release" "$target_commit" \
        "$target_vps_release_manifest_sha256" ||
        fail "currently selected rollback target differs from its retained V2 authority"
fi

# This is the rollback compatibility gate. It deliberately runs before
# reconciling journals, creating recovery trees, touching unit enablement, or
# stopping a service. The target's retained admin binary attests its retained
# release root and copies the live database under the immutable runtime fence.
# A mismatch therefore leaves every persistent byte and runtime unit unchanged.
if [ "$selected_commit" != "$target_commit" ]; then
    source_schema_receipt=$("$source_release_root/bin/robin-highscores-admin" \
        verify-live-database-schema-v2 \
        --candidate-release-root-fd "$source_release_root_fd" \
        --expected-vps-release-manifest-sha256 "$source_vps_release_manifest_sha256") ||
        fail "currently selected release does not exactly attest the live database schema; no state was changed"
    printf '%s' "$source_schema_receipt" | grep -Fq -- '"schema_version":2' ||
        fail "source live-schema verifier emitted a non-V2 receipt"
    printf '%s' "$source_schema_receipt" | grep -Fq -- \
        "\"source_commit\":\"$selected_commit\"" || fail "source live-schema receipt names another release"
    printf '%s' "$source_schema_receipt" | grep -Fq -- \
        "\"vps_release_manifest_sha256\":\"$source_vps_release_manifest_sha256\"" ||
        fail "source live-schema receipt differs from the retained current authority"
fi
validate_pinned_release_root "$target_release_root_fd" "$target_release" "$target_commit" \
    "$target_vps_release_manifest_sha256" ||
    fail "retained rollback target changed before live-schema verification"
target_schema_receipt=$("$target_release_root/bin/robin-highscores-admin" \
    verify-live-database-schema-v2 \
    --candidate-release-root-fd "$target_release_root_fd" \
    --expected-vps-release-manifest-sha256 "$target_vps_release_manifest_sha256") ||
    fail "rollback target is not exactly compatible with the live database schema; no state was changed"
printf '%s' "$target_schema_receipt" | grep -Fq -- \
    '"schema_version":2' || fail "live-schema verifier emitted a non-V2 receipt"
printf '%s' "$target_schema_receipt" | grep -Fq -- \
    "\"source_commit\":\"$target_commit\"" || fail "live-schema receipt names another release"
printf '%s' "$target_schema_receipt" | grep -Fq -- \
    "\"vps_release_manifest_sha256\":\"$target_vps_release_manifest_sha256\"" ||
    fail "live-schema receipt differs from the retained target authority"
validate_pinned_release_root "$target_release_root_fd" "$target_release" "$target_commit" \
    "$target_vps_release_manifest_sha256" ||
    fail "retained rollback target changed at the live-schema receipt boundary"

reconcile_prepared_journal_temporary ||
    fail "could not reconcile the exact interrupted rollback journal publication"
if [ -f "$prepared_journal" ] && [ ! -L "$prepared_journal" ]; then
    interrupted_source_commit=$(sed -n 's/^source_commit=//p' "$prepared_journal")
    validate_prepared_journal "$interrupted_source_commit" ||
        fail "durable rollback journal is not exact"
    if [ "$journal_phase" = preparing ]; then
        [ "$selected_commit" = "$interrupted_source_commit" ] ||
            fail "interrupted rollback preparation no longer has its exact source selected"
        previous_commit=$interrupted_source_commit
        previous_target=releases/$interrupted_source_commit
        retain_source_release_authority ||
            fail "could not retain interrupted rollback source authority"
        abort_interrupted_preparation "$interrupted_source_commit" ||
            fail "could not safely reconcile interrupted rollback preparation"
        rollback_mode=normal
    fi
fi
if [ "$rollback_mode" = resume ]; then
    if [ ! -e "$prepared_journal" ] && [ ! -L "$prepared_journal" ] &&
        [ "$selected_commit" = "$target_commit" ] &&
        adopt_completed_rollback_without_journal; then
        echo "release $target_commit is active and ready; interrupted final rollback directory sync completed"
        exit 0
    fi
    [ -f "$prepared_journal" ] && [ ! -L "$prepared_journal" ] ||
        fail "rollback resume requires its exact durable prepared journal"
    journal_source_commit=$(sed -n 's/^source_commit=//p' "$prepared_journal")
    case "$journal_source_commit" in *[!0-9a-f]*|'') fail "rollback journal has invalid source identity" ;; esac
    [ "${#journal_source_commit}" -eq 40 ] || fail "rollback journal has invalid source identity"
    validate_prepared_journal "$journal_source_commit" ||
        fail "rollback durable journal or prepared recovery snapshot changed"
    prepared_journal_owned=1
    [ -d "$release_root/$journal_source_commit" ] && [ ! -L "$release_root/$journal_source_commit" ] ||
        fail "rollback resume source release is missing"
    if [ "$selected_commit" = "$target_commit" ]; then
        target_selected=1
    elif [ "$selected_commit" != "$journal_source_commit" ]; then
        fail "rollback resume selector is neither exact source nor exact target"
    fi
    previous_commit=$journal_source_commit
    previous_target=releases/$journal_source_commit
    retain_source_release_authority ||
        fail "could not retain and authenticate the exact rollback source release"
    if [ "$target_selected" -eq 1 ]; then
        case "$journal_phase" in
            target_backup_verified|activation_complete) ;;
            *) fail "rollback selected its target before the exact target BackupV4 receipt was journaled" ;;
        esac
    fi
    if [ "$journal_phase" = target_backup_verified ] && [ "$target_selected" -eq 1 ] &&
        validate_active_target_state; then
        validate_completed_backup_receipts ||
            fail "completed rollback does not retain exact distinct verified source and target BackupV4 receipts"
        write_terminal_journal "$journal_source_commit" ||
            fail "could not durably recover the rollback activation-complete cleanup phase"
        rollback_complete=1
        preserve_recovery=1
        finish_terminal_cleanup "$journal_source_commit" ||
            fail "could not finish recovered active rollback terminal cleanup"
        preserve_recovery=0
        echo "release $target_commit is active and ready; interrupted terminal journal publication completed"
        exit 0
    fi
    if [ "$journal_phase" = activation_complete ]; then
        rollback_complete=1
        preserve_recovery=1
        finish_terminal_cleanup "$journal_source_commit" ||
            fail "could not finish active rollback terminal cleanup"
        preserve_recovery=0
        echo "release $target_commit is active and ready; interrupted terminal cleanup completed"
        exit 0
    fi
else
    [ "$previous_commit" != "$target_commit" ] || fail "target release is already selected; use --resume-target"
    retain_source_release_authority ||
        fail "could not retain and authenticate the exact rollback source release"
fi

if [ "$rollback_mode" = resume ]; then
    [ -d "$unit_stage_root" ] && [ ! -L "$unit_stage_root" ] &&
        [ -d "$unit_backup_root" ] && [ ! -L "$unit_backup_root" ] ||
        fail "rollback prepared journal is missing its exact recovery trees"
    validate_prepared_journal "$previous_commit" ||
        fail "rollback prepared journal or recovery snapshot changed"
    case "$target_selected:$journal_phase" in
        0:prepared|0:source_backup_started|0:source_backup_verified|0:source_backup_invalidated|0:target_backup_started|0:target_backup_verified|1:target_backup_verified) ;;
        *) fail "rollback journal phase is incompatible with its selected release" ;;
    esac
    validate_owned_tree "$unit_stage_root" && [ "$(stat -c %a -- "$unit_stage_root")" = 700 ] ||
        fail "rollback target staging is unsafe"
    for unit in robin-highscores.target robin-highscores-api.service robin-highscores-worker.service \
        robin-highscores-backup.service robin-highscores-backup.timer; do
        if [ -e "$unit_stage_root/units/$unit" ] || [ -L "$unit_stage_root/units/$unit" ]; then
            [ -f "$unit_stage_root/units/$unit" ] && [ ! -L "$unit_stage_root/units/$unit" ] &&
                [ "$(stat -c %u -- "$unit_stage_root/units/$unit")" -eq "$(id -u)" ] &&
                [ "$(stat -c %h -- "$unit_stage_root/units/$unit")" -eq 1 ] &&
                [ "$(stat -c %a -- "$unit_stage_root/units/$unit")" = 440 ] &&
                cmp -s -- "$unit_stage_root/units/$unit" "$target_release_root/systemd/user/$unit" ||
                fail "rollback target staging changed: $unit"
        else
            [ -f "$user_unit_root/$unit" ] && [ ! -L "$user_unit_root/$unit" ] &&
                [ "$(stat -c %u -- "$user_unit_root/$unit")" -eq "$(id -u)" ] &&
                [ "$(stat -c %h -- "$user_unit_root/$unit")" -eq 1 ] &&
                [ "$(stat -c %a -- "$user_unit_root/$unit")" = 440 ] &&
                { cmp -s -- "$user_unit_root/$unit" "$target_release_root/systemd/user/$unit" ||
                    cmp -s -- "$user_unit_root/$unit" "$unit_backup_root/units/$unit"; } ||
                fail "missing rollback target stage has neither exact source nor target destination: $unit"
            install -m 0440 -- "$target_release_root/systemd/user/$unit" "$unit_stage_root/units/$unit" ||
                fail "could not reconstruct rollback target staging: $unit"
            sync -f -- "$unit_stage_root/units/$unit"
        fi
        [ -f "$unit_backup_root/units/$unit" ] && [ ! -L "$unit_backup_root/units/$unit" ] &&
            [ "$(stat -c %u -- "$unit_backup_root/units/$unit")" -eq "$(id -u)" ] &&
            [ "$(stat -c %h -- "$unit_backup_root/units/$unit")" -eq 1 ] &&
            [ "$(stat -c %a -- "$unit_backup_root/units/$unit")" = 440 ] &&
            cmp -s -- "$unit_backup_root/units/$unit" "$source_release_root/systemd/user/$unit" ||
            fail "rollback source recovery unit changed: $unit"
    done
    unit_stage_owned=1
    unit_backup_owned=1
    prepared_journal_owned=1
    reuse_recovery_snapshot=1
fi
if [ "$reuse_recovery_snapshot" -eq 0 ]; then
    write_preparation_intent "$previous_commit" ||
        fail "could not make the rollback preparation intent durable before recovery mutation"
    [ ! -e "$unit_stage_root" ] && [ ! -L "$unit_stage_root" ] || fail "unit staging path exists"
    if mkdir -m 0700 -- "$unit_stage_root"; then unit_stage_owned=1; else fail "cannot create unit staging"; fi
    mkdir -m 0700 -- "$unit_stage_root/units"
    [ ! -e "$unit_backup_root" ] && [ ! -L "$unit_backup_root" ] || fail "unit backup path exists"
    if mkdir -m 0700 -- "$unit_backup_root"; then unit_backup_owned=1; else fail "cannot create unit backup"; fi
    mkdir -m 0700 -- "$unit_backup_root/units" "$unit_backup_root/absent" "$unit_backup_root/active" "$unit_backup_root/enabled"

    for unit in robin-highscores.target robin-highscores-api.service robin-highscores-worker.service \
        robin-highscores-backup.service robin-highscores-backup.timer; do
        destination=$user_unit_root/$unit
        install -m 0440 -- "$target_release_root/systemd/user/$unit" "$unit_stage_root/units/$unit"
        sync -f -- "$unit_stage_root/units/$unit"
        [ -f "$destination" ] && [ ! -L "$destination" ] &&
            [ "$(stat -c %u -- "$destination")" -eq "$(id -u)" ] &&
            [ "$(stat -c %h -- "$destination")" -eq 1 ] &&
            [ "$(stat -c %a -- "$destination")" = 440 ] &&
            cmp -s -- "$destination" "$source_release_root/systemd/user/$unit" ||
            fail "installed unit is not the exact rollback source: $unit"
        cp --archive -- "$source_release_root/systemd/user/$unit" "$unit_backup_root/units/$unit"
        sync -f -- "$unit_backup_root/units/$unit"
        active_state=$(systemctl --user show --property=ActiveState --value "$unit") || fail "cannot capture state: $unit"
        case "$active_state" in
            active)
                : >"$unit_backup_root/active/$unit"
                sync -f -- "$unit_backup_root/active/$unit"
                ;;
            inactive) ;;
            *) fail "transitional or failed unit: $unit" ;;
        esac
    done
fi
[ ! -f "$unit_backup_root/active/robin-highscores-backup.service" ] || fail "backup is active"
[ ! -f "$unit_backup_root/active/robin-highscores.target" ] ||
    [ -f "$unit_backup_root/active/robin-highscores-api.service" ] || fail "active target has inactive API"
[ ! -f "$unit_backup_root/active/robin-highscores-worker.service" ] ||
    [ -f "$unit_backup_root/active/robin-highscores-api.service" ] || fail "active worker has inactive API"
if [ "$reuse_recovery_snapshot" -eq 0 ]; then
for unit in robin-highscores.target robin-highscores-backup.timer; do
    if enabled_state=$(systemctl --user is-enabled "$unit" 2>/dev/null); then
        case "$enabled_state" in
            enabled|enabled-runtime)
                printf '%s\n' "$enabled_state" >"$unit_backup_root/enabled/$unit"
                sync -f -- "$unit_backup_root/enabled/$unit"
                ;;
            *) fail "unexpected enabled state: $unit" ;;
        esac
    else
        case "$enabled_state" in disabled|static|indirect|masked|not-found) ;; *) fail "cannot capture enabled state: $unit ($enabled_state)" ;; esac
    fi
done
fi

if [ -L "$temporary_link" ] && [ "$(stat -c %u -- "$temporary_link")" -eq "$(id -u)" ] &&
    [ "$(readlink -- "$temporary_link")" = "releases/$target_commit" ]; then
    [ "$reuse_recovery_snapshot" -eq 1 ] || fail "unexpected rollback target selector staging"
    temporary_link_owned=1
elif [ ! -e "$temporary_link" ] && [ ! -L "$temporary_link" ]; then
    if ln -s -- "releases/$target_commit" "$temporary_link"; then temporary_link_owned=1; else fail "cannot create temporary selector"; fi
else
    fail "unsafe rollback target selector staging"
fi
if [ -n "$previous_target" ]; then
    if [ -L "$restore_link" ] && [ "$(stat -c %u -- "$restore_link")" -eq "$(id -u)" ] &&
        [ "$(readlink -- "$restore_link")" = "$previous_target" ]; then
        [ "$reuse_recovery_snapshot" -eq 1 ] || fail "unexpected rollback source selector staging"
        restore_link_owned=1
    elif [ ! -e "$restore_link" ] && [ ! -L "$restore_link" ]; then
        if ln -s -- "$previous_target" "$restore_link"; then restore_link_owned=1; else fail "cannot create restore selector"; fi
    else
        fail "unsafe rollback source selector staging"
    fi
fi
sync -f -- "$unit_stage_root/units"
sync -f -- "$unit_stage_root"
for recovery_directory in "$unit_backup_root/units" "$unit_backup_root/absent" \
    "$unit_backup_root/active" "$unit_backup_root/enabled"; do
    sync -f -- "$recovery_directory"
done
sync -f -- "$unit_backup_root"
sync -f -- "$user_unit_root"
sync -f -- "$opt_root"
transaction_prepared=1
if [ "$reuse_recovery_snapshot" -eq 0 ]; then
    write_prepared_journal "$previous_commit" ||
        fail "could not make the exact prepared rollback journal durable"
fi

reuse_prebackup_receipt=0
resume_source_backup_started=0
if [ "$rollback_mode" = resume ] &&
    { [ -e "$prebackup_receipt_temporary" ] || [ -L "$prebackup_receipt_temporary" ]; }; then
    reconcile_prebackup_receipt_temporary "$previous_commit" ||
        fail "could not reconcile the interrupted rollback prebackup receipt publication"
fi
if [ "$rollback_mode" = resume ]; then
    validate_prepared_journal "$previous_commit" || fail "rollback backup phase journal changed"
    case "$journal_phase" in
        source_backup_started)
            resume_source_backup_started=1
            ;;
        source_backup_invalidated)
            resume_source_backup_started=1
            ;;
        source_backup_verified|target_backup_started|target_backup_verified)
            reuse_prebackup_receipt=1
            ;;
        prepared) ;;
        *) fail "rollback cannot resume source backup from this journal phase: $journal_phase" ;;
    esac
fi

if [ -n "$previous_commit" ]; then
    if [ "$reuse_prebackup_receipt" -eq 0 ]; then
        [ ! -e "$target_backup_receipt" ] && [ ! -L "$target_backup_receipt" ] &&
            [ ! -e "$target_backup_receipt_temporary" ] && [ ! -L "$target_backup_receipt_temporary" ] ||
            fail "target-backup receipt exists before rollback target selection"
        validate_pinned_release_root "$source_release_root_fd" "$source_release" "$previous_commit" \
            "$source_vps_release_manifest_sha256" ||
            fail "retained rollback source changed before its offline backup"
        if [ "$resume_source_backup_started" -eq 0 ] &&
            [ -f "$unit_backup_root/active/robin-highscores-api.service" ]; then
            probe http://127.0.0.1:8787/healthz 5 ||
                fail "active current release is not healthy before rollback quiescence"
            probe http://127.0.0.1:8787/readyz 5 ||
                fail "active current release is not ready before rollback quiescence"
        fi
        [ "$(stat -Lc %d:%i -- "$activation_lock_fd")" = "$activation_lock_identity" ] ||
            fail "inherited activation lock descriptor changed before rollback quiescence"
        [ -f "$activation_lock" ] && [ ! -L "$activation_lock" ] &&
            [ "$(stat -c %d:%i -- "$activation_lock")" = "$activation_lock_identity" ] ||
            fail "canonical activation lock path changed before rollback quiescence"
        activation_started=1
        prebackup_timer_disabled=1
        systemctl --user disable --now robin-highscores-backup.timer >/dev/null ||
            fail "cannot disable and stop old timer"
        if [ -d "$user_unit_root/timers.target.wants" ] && [ ! -L "$user_unit_root/timers.target.wants" ]; then
            sync -f -- "$user_unit_root/timers.target.wants"
        fi
        wait_for_backup_service_inactive ||
            fail "timed out waiting for a raced backup without interrupting it"
        systemctl --user disable --now robin-highscores.target >/dev/null ||
            fail "cannot disable and stop old target"
        for unit in robin-highscores.target robin-highscores-worker.service robin-highscores-api.service \
            robin-highscores-backup.timer robin-highscores-backup.service; do
            systemctl --user stop "$unit" || fail "cannot quiesce $unit before the offline rollback backup"
        done
        for unit in robin-highscores.target robin-highscores-api.service robin-highscores-worker.service \
            robin-highscores-backup.service robin-highscores-backup.timer; do
            [ "$(systemctl --user show --property=ActiveState --value "$unit")" = inactive ] ||
                fail "source writer remained active before the offline rollback backup: $unit"
        done
        for unit in robin-highscores.target robin-highscores-backup.timer; do
            if enabled_state=$(systemctl --user is-enabled "$unit" 2>/dev/null); then
                [ "$enabled_state" = disabled ] ||
                    fail "source writer remained enabled before the offline rollback backup: $unit"
            else
                [ "$enabled_state" = disabled ] ||
                    fail "source writer enablement could not be proved disabled before the offline rollback backup: $unit"
            fi
        done
        validate_prepared_journal "$previous_commit" || fail "source backup phase journal changed"
        if [ "$journal_phase" = source_backup_invalidated ]; then
            validate_pinned_release_root "$source_release_root_fd" "$source_release" "$previous_commit" \
                "$source_vps_release_manifest_sha256" ||
                fail "rollback source changed before interrupted restore convergence"
            validate_pinned_release_root "$target_release_root_fd" "$target_release" "$target_commit" \
                "$target_vps_release_manifest_sha256" ||
                fail "rollback target changed before interrupted restore convergence"
            converge_source_units_stopped ||
                fail "could not converge an interrupted cleanup to exact stopped source units"
            if [ -e "$prebackup_receipt" ] || [ -L "$prebackup_receipt" ]; then
                [ -f "$prebackup_receipt" ] && [ ! -L "$prebackup_receipt" ] &&
                    [ "$(durable_receipt_sha256 "$prebackup_receipt")" = "$journal_invalidated_source_receipt_sha" ] ||
                    fail "invalidated source BackupV4 receipt changed before retirement"
                rm -f -- "$prebackup_receipt" || fail "could not retire invalidated source BackupV4 receipt"
                sync -f -- "$opt_root" || fail "could not make source BackupV4 receipt retirement durable"
            fi
            [ ! -e "$prebackup_receipt" ] && [ ! -L "$prebackup_receipt" ] ||
                fail "invalidated source BackupV4 receipt remains reusable"
        fi
        validate_prepared_journal "$previous_commit" || fail "source backup phase journal changed"
        source_capacity_admitted=0
        case "$journal_phase" in
            prepared)
                validate_pinned_release_root "$source_release_root_fd" "$source_release" "$previous_commit" \
                    "$source_vps_release_manifest_sha256" ||
                    fail "rollback source changed before stopped capacity admission"
                require_backup_capacity "$source_release_root" ||
                    fail "typed stopped rollback backup capacity admission failed"
                validate_pinned_release_root "$source_release_root_fd" "$source_release" "$previous_commit" \
                    "$source_vps_release_manifest_sha256" ||
                    fail "rollback source changed at stopped capacity admission boundary"
                source_capacity_admitted=1
                advance_prepared_journal "$previous_commit" prepared source_backup_started source ||
                    fail "could not durably record the source BackupV4 start boundary"
                ;;
            source_backup_invalidated)
                validate_pinned_release_root "$source_release_root_fd" "$source_release" "$previous_commit" \
                    "$source_vps_release_manifest_sha256" ||
                    fail "rollback source changed before fresh stopped capacity admission"
                require_backup_capacity "$source_release_root" ||
                    fail "typed stopped rollback backup capacity admission failed"
                validate_pinned_release_root "$source_release_root_fd" "$source_release" "$previous_commit" \
                    "$source_vps_release_manifest_sha256" ||
                    fail "rollback source changed at fresh stopped capacity admission boundary"
                source_capacity_admitted=1
                advance_prepared_journal "$previous_commit" source_backup_invalidated source_backup_started source ||
                    fail "could not durably record a fresh source BackupV4 start after invalidation"
                ;;
            source_backup_started) ;;
            *) fail "source backup has an invalid start phase: $journal_phase" ;;
        esac
        if [ -f "$prebackup_receipt" ] && [ ! -L "$prebackup_receipt" ]; then
            classify_backup_status_since "$journal_source_status_before" ||
                fail "could not classify the exact source backup status identity"
            [ "$backup_status_change" = changed ] ||
                fail "source receipt exists without a newly published backup status"
            validate_prebackup_receipt "$previous_commit" ||
                fail "durable source receipt no longer matches typed verification"
        else
            [ ! -e "$prebackup_receipt" ] && [ ! -L "$prebackup_receipt" ] ||
                fail "unsafe source receipt path"
            classify_backup_status_since "$journal_source_status_before" ||
                fail "could not classify the exact source backup status identity"
            if [ "$backup_status_change" = unchanged ]; then
                if [ "$source_capacity_admitted" -eq 0 ]; then
                    validate_pinned_release_root "$source_release_root_fd" "$source_release" "$previous_commit" \
                        "$source_vps_release_manifest_sha256" ||
                        fail "rollback source changed before resumed stopped capacity admission"
                    require_backup_capacity "$source_release_root" ||
                        fail "typed stopped rollback backup capacity admission failed"
                    validate_pinned_release_root "$source_release_root_fd" "$source_release" "$previous_commit" \
                        "$source_vps_release_manifest_sha256" ||
                        fail "rollback source changed at resumed stopped capacity admission boundary"
                fi
                systemctl --user start robin-highscores-backup.service ||
                    fail "fresh pre-rollback backup failed"
                wait_for_backup_service_inactive ||
                    fail "offline rollback source backup service did not return to inactive"
            fi
            classify_backup_status_since "$journal_source_status_before" ||
                fail "could not classify the post-backup source status identity"
            [ "$backup_status_change" = changed ] ||
                fail "source BackupV4 did not publish a new authenticated status"
            write_prebackup_receipt "$previous_commit" ||
                fail "fresh rollback prebackup receipt could not be authenticated and made durable"
        fi
        advance_prepared_journal "$previous_commit" source_backup_started source_backup_verified ||
            fail "could not bind the source BackupV4 receipt into the durable rollback journal"
        validate_pinned_release_root "$source_release_root_fd" "$source_release" "$previous_commit" \
            "$source_vps_release_manifest_sha256" ||
            fail "retained rollback source changed at its verified-backup receipt boundary"
    fi
    backup_quiesced=1
    [ "$(stat -Lc %d:%i -- "$activation_lock_fd")" = "$activation_lock_identity" ] ||
        fail "inherited activation lock descriptor changed before rollback activation"
    [ -f "$activation_lock" ] && [ ! -L "$activation_lock" ] &&
        [ "$(stat -c %d:%i -- "$activation_lock")" = "$activation_lock_identity" ] ||
        fail "canonical activation lock path changed before rollback activation"
    activation_started=1
    systemctl --user disable --now robin-highscores-backup.timer >/dev/null ||
        fail "cannot keep rollback backup timer disabled during activation"
    wait_for_backup_service_inactive ||
        fail "timed out waiting for the rollback backup service to become inactive"
    systemctl --user disable --now robin-highscores.target >/dev/null ||
        fail "cannot keep rollback target disabled during activation"
    for unit in robin-highscores.target robin-highscores-worker.service robin-highscores-api.service \
        robin-highscores-backup.timer robin-highscores-backup.service; do
        systemctl --user stop "$unit" || fail "cannot stop $unit"
    done
    for unit in robin-highscores.target robin-highscores-api.service robin-highscores-worker.service \
        robin-highscores-backup.service robin-highscores-backup.timer; do
        [ "$(systemctl --user show --property=ActiveState --value "$unit")" = inactive ] || fail "unit remains active: $unit"
    done
    for unit in robin-highscores.target robin-highscores-backup.timer; do
        if enabled_state=$(systemctl --user is-enabled "$unit" 2>/dev/null); then
            [ "$enabled_state" = disabled ] || fail "rollback unit remains enabled while stopped: $unit"
        else
            [ "$enabled_state" = disabled ] || fail "rollback unit enablement is not exactly disabled: $unit"
        fi
    done
    for wants_directory in "$user_unit_root/default.target.wants" "$user_unit_root/timers.target.wants"; do
        if [ -e "$wants_directory" ] || [ -L "$wants_directory" ]; then
            [ -d "$wants_directory" ] && [ ! -L "$wants_directory" ] || fail "unsafe enablement directory"
            sync -f -- "$wants_directory"
        fi
    done
fi

validate_prepared_journal "$previous_commit" || fail "rollback BackupV4 journal binding changed"
case "$journal_phase" in
    source_backup_verified|target_backup_started|target_backup_verified) ;;
    *) fail "rollback source BackupV4 receipt is not bound before target selection" ;;
esac
validate_pinned_release_root "$source_release_root_fd" "$source_release" "$previous_commit" \
    "$source_vps_release_manifest_sha256" ||
    fail "retained rollback source changed before target selection"
validate_pinned_release_root "$target_release_root_fd" "$target_release" "$target_commit" \
    "$target_vps_release_manifest_sha256" ||
    fail "retained rollback target changed before target selection"

for unit in robin-highscores.target robin-highscores-api.service robin-highscores-worker.service \
    robin-highscores-backup.service robin-highscores-backup.timer; do
    destination=$user_unit_root/$unit
    staged_unit=$unit_stage_root/units/$unit
    [ -f "$destination" ] && [ ! -L "$destination" ] &&
        [ "$(stat -c %u -- "$destination")" -eq "$(id -u)" ] &&
        [ "$(stat -c %h -- "$destination")" -eq 1 ] &&
        [ "$(stat -c %a -- "$destination")" = 440 ] &&
        { cmp -s -- "$destination" "$unit_backup_root/units/$unit" ||
            cmp -s -- "$destination" "$target_release_root/systemd/user/$unit"; } ||
        fail "managed unit changed before rollback replacement: $unit"
    if ! mv -T -- "$staged_unit" "$destination"; then
        [ ! -e "$staged_unit" ] && [ ! -L "$staged_unit" ] &&
            [ -f "$destination" ] && [ ! -L "$destination" ] &&
            [ "$(stat -c %u -- "$destination")" -eq "$(id -u)" ] &&
            [ "$(stat -c %h -- "$destination")" -eq 1 ] &&
            [ "$(stat -c %a -- "$destination")" = 440 ] &&
            cmp -s -- "$destination" "$target_release_root/systemd/user/$unit" ||
            fail "could not atomically replace rollback unit: $unit"
    fi
    sync -f -- "$destination"
done
sync -f -- "$user_unit_root"
systemctl --user daemon-reload
[ -L "$current_link" ] && [ "$(stat -c %u -- "$current_link")" -eq "$(id -u)" ] ||
    fail "current selector is unsafe before rollback target backup"
if [ "$target_selected" -eq 0 ]; then
    [ "$(readlink -- "$current_link")" = "$previous_target" ] ||
        fail "source selector changed before rollback target backup"
else
    [ "$(readlink -- "$current_link")" = "releases/$target_commit" ] ||
        fail "selected rollback target changed before receipt revalidation"
fi

if [ -e "$target_backup_receipt_temporary" ] || [ -L "$target_backup_receipt_temporary" ]; then
    [ -f "$target_backup_receipt_temporary" ] && [ ! -L "$target_backup_receipt_temporary" ] &&
        [ "$(stat -c %u -- "$target_backup_receipt_temporary")" -eq "$(id -u)" ] &&
        [ "$(stat -c %h -- "$target_backup_receipt_temporary")" -eq 1 ] ||
        fail "unsafe interrupted target-backup receipt"
    rm -f -- "$target_backup_receipt_temporary" || fail "cannot reconcile target-backup receipt"
    sync -f -- "$opt_root"
fi
validate_prepared_journal "$previous_commit" || fail "rollback target BackupV4 journal binding changed"
target_capacity_admitted=0
case "$journal_phase" in
    source_backup_verified)
        [ "$target_selected" -eq 0 ] || fail "rollback target was selected before target BackupV4 admission"
        validate_pinned_release_root "$target_release_root_fd" "$target_release" "$target_commit" \
            "$target_vps_release_manifest_sha256" ||
            fail "retained rollback target changed before stopped capacity admission"
        require_backup_capacity "$target_release_root" ||
            fail "initial typed rollback target-backup capacity admission failed"
        validate_pinned_release_root "$target_release_root_fd" "$target_release" "$target_commit" \
            "$target_vps_release_manifest_sha256" ||
            fail "retained rollback target changed before its readiness backup"
        target_capacity_admitted=1
        advance_prepared_journal "$previous_commit" source_backup_verified target_backup_started target ||
            fail "could not durably record the target BackupV4 start boundary"
        ;;
    target_backup_started)
        [ "$target_selected" -eq 0 ] || fail "rollback target was selected before its BackupV4 receipt"
        ;;
    target_backup_verified) ;;
    *) fail "rollback target BackupV4 has no valid journal predecessor" ;;
esac

if [ "$journal_phase" = target_backup_started ]; then
    if [ -f "$target_backup_receipt" ] && [ ! -L "$target_backup_receipt" ]; then
        classify_backup_status_since "$journal_target_status_before" ||
            fail "could not classify the exact target backup status identity"
        [ "$backup_status_change" = changed ] ||
            fail "target receipt exists without a newly published backup status"
        rerun_and_compare_backup_receipt "$target_release_root_fd" "$target_commit" \
            "$target_vps_release_manifest_sha256" "$target_publication_lock_sha256" \
            "$target_backup_receipt" "$target_backup_receipt_temporary" ||
            fail "durable target BackupV4 receipt no longer matches a fresh typed verification"
    else
        [ ! -e "$target_backup_receipt" ] && [ ! -L "$target_backup_receipt" ] ||
            fail "unsafe target-backup receipt path"
        classify_backup_status_since "$journal_target_status_before" ||
            fail "could not classify the exact target backup status identity"
        if [ "$backup_status_change" = unchanged ]; then
            if [ "$target_capacity_admitted" -eq 0 ]; then
                validate_pinned_release_root "$target_release_root_fd" "$target_release" "$target_commit" \
                    "$target_vps_release_manifest_sha256" ||
                    fail "retained rollback target changed before resumed stopped capacity admission"
                require_backup_capacity "$target_release_root" ||
                    fail "resumed typed rollback target-backup capacity admission failed"
                validate_pinned_release_root "$target_release_root_fd" "$target_release" "$target_commit" \
                    "$target_vps_release_manifest_sha256" ||
                    fail "retained rollback target changed at resumed stopped capacity admission boundary"
            fi
            systemctl --user start robin-highscores-backup.service ||
                fail "offline rollback target backup failed"
            wait_for_backup_service_inactive ||
                fail "offline rollback target backup service did not return to inactive"
        fi
        classify_backup_status_since "$journal_target_status_before" ||
            fail "could not classify the post-backup target status identity"
        [ "$backup_status_change" = changed ] ||
            fail "target BackupV4 did not publish a new authenticated status"
        write_verified_backup_receipt "$target_release_root_fd" "$target_commit" \
            "$target_vps_release_manifest_sha256" "$target_publication_lock_sha256" \
            "$target_backup_receipt" "$target_backup_receipt_temporary" ||
            fail "fresh rollback target BackupV4 receipt could not be verified and made durable"
    fi
    advance_prepared_journal "$previous_commit" target_backup_started target_backup_verified ||
        fail "could not bind the target BackupV4 receipt into the durable rollback journal"
else
    rerun_and_compare_backup_receipt "$target_release_root_fd" "$target_commit" \
        "$target_vps_release_manifest_sha256" "$target_publication_lock_sha256" \
        "$target_backup_receipt" "$target_backup_receipt_temporary" ||
        fail "journaled target BackupV4 receipt no longer matches a fresh typed verification"
fi
validate_prepared_journal "$previous_commit" && [ "$journal_phase" = target_backup_verified ] ||
    fail "rollback target BackupV4 receipt changed before target selection"
cmp -s -- "$prebackup_receipt" "$target_backup_receipt" &&
    fail "rollback source and target backups did not produce distinct verified receipts"
validate_pinned_release_root "$target_release_root_fd" "$target_release" "$target_commit" \
    "$target_vps_release_manifest_sha256" ||
    fail "retained rollback target changed at its verified-backup receipt boundary"

if [ "$target_selected" -eq 0 ]; then
    [ -L "$current_link" ] && [ "$(readlink -- "$current_link")" = "$previous_target" ] ||
        fail "source selector changed before authenticated rollback target selection"
    selection_attempted=1
    if mv -T -- "$temporary_link" "$current_link"; then
        target_selected=1
        temporary_link_owned=0
    else
        if [ -L "$current_link" ] && [ "$(readlink -- "$current_link")" = "releases/$target_commit" ]; then
            target_selected=1
            [ -e "$temporary_link" ] || [ -L "$temporary_link" ] || temporary_link_owned=0
        fi
        fail "cannot select rollback target"
    fi
fi
sync -f -- "$opt_root" || fail "selector durability is uncertain"
systemctl --user start robin-highscores-api.service
probe http://127.0.0.1:8787/healthz 30 || fail "target API failed liveness after its offline backup"
probe http://127.0.0.1:8787/readyz 30 || fail "target API failed readiness against its offline backup"
systemctl --user start robin-highscores-worker.service
systemctl --user start robin-highscores.target
systemctl --user enable robin-highscores.target >/dev/null
systemctl --user enable --now robin-highscores-backup.timer >/dev/null
for wants_directory in "$user_unit_root/default.target.wants" "$user_unit_root/timers.target.wants"; do
    [ -d "$wants_directory" ] && [ ! -L "$wants_directory" ] || fail "expected enablement directory is unsafe or missing"
    sync -f -- "$wants_directory"
done
for unit in robin-highscores.target robin-highscores-backup.timer; do
    [ "$(systemctl --user is-enabled "$unit")" = enabled ] || fail "final rollback unit is not persistently enabled: $unit"
done
for unit in robin-highscores.target robin-highscores-api.service robin-highscores-worker.service \
    robin-highscores-backup.timer; do
    [ "$(systemctl --user show --property=ActiveState --value "$unit")" = active ] || fail "final rollback unit is not active: $unit"
done
[ "$(systemctl --user show --property=ActiveState --value robin-highscores-backup.service)" = inactive ] ||
    fail "rollback backup service did not return to inactive"
probe http://127.0.0.1:8787/healthz 5 || fail "final liveness failed"
probe http://127.0.0.1:8787/readyz 5 || fail "final readiness failed"

write_terminal_journal "$previous_commit" ||
    fail "could not durably record the rollback activation-complete cleanup phase"
rollback_complete=1
preserve_recovery=1
finish_terminal_cleanup "$previous_commit" ||
    fail "rollback target is active but idempotent terminal cleanup did not complete"
preserve_recovery=0
echo "release $target_commit is active and ready"
[ -z "$previous_commit" ] || echo "replaced release remains installed: $previous_commit"
