#!/bin/sh
set -eu
PATH=/usr/bin:/bin
export PATH

# Install and activate one reviewed release entirely as the robinhood SSH user.
# This script never elevates privileges or changes global nginx/systemd state.

usage() {
    echo "usage: $0 ABSOLUTE_RELEASES_COMMIT_PARTIAL EXPECTED_40HEX_COMMIT EXPECTED_SHA256SUMS_SHA256 EXPECTED_DEPLOY_BOOTSTRAP_SHA256SUMS_SHA256 /proc/self/fd/MANIFEST_FD /proc/self/fd/VALIDATOR_FD /proc/self/fd/MANIFESTCTL_FD /proc/self/fd/PLAN_FD /proc/self/fd/CANDIDATE_ROOT_FD /proc/self/fd/LOCK_FD EXPECTED_PLAN_SHA256 EXPECTED_VPS_MANIFEST_SHA256" >&2
    echo "   or: $0 --resume-installed ABSOLUTE_RELEASE_COMMIT EXPECTED_40HEX_COMMIT EXPECTED_SHA256SUMS_SHA256 EXPECTED_DEPLOY_BOOTSTRAP_SHA256SUMS_SHA256 /proc/self/fd/MANIFEST_FD /proc/self/fd/VALIDATOR_FD /proc/self/fd/MANIFESTCTL_FD /proc/self/fd/PLAN_FD /proc/self/fd/CANDIDATE_ROOT_FD /proc/self/fd/LOCK_FD EXPECTED_PLAN_SHA256 EXPECTED_VPS_MANIFEST_SHA256" >&2
    exit 64
}

fail() {
    echo "deployment failed: $*" >&2
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
    12:*)
        deploy_mode=candidate
        bundle_arg=$1
        expected_commit=$2
        expected_sums_sha256=$3
        expected_bootstrap_sums_sha256=$4
        bootstrap_manifest_fd=$5
        validator_fd=$6
        manifest_tool_fd=$7
        plan_fd=$8
        candidate_root_fd=$9
        activation_lock_fd=${10}
        expected_plan_sha256=${11}
        expected_vps_manifest_sha256=${12}
        ;;
    13:--resume-installed)
        deploy_mode=resume
        bundle_arg=$2
        expected_commit=$3
        expected_sums_sha256=$4
        expected_bootstrap_sums_sha256=$5
        bootstrap_manifest_fd=$6
        validator_fd=$7
        manifest_tool_fd=$8
        plan_fd=$9
        candidate_root_fd=${10}
        activation_lock_fd=${11}
        expected_plan_sha256=${12}
        expected_vps_manifest_sha256=${13}
        ;;
    *) usage ;;
esac

expected_user=robinhood
expected_home=/home/robinhood
opt_root=/home/robinhood/.local/opt/robin-highscores
release_root=$opt_root/releases
incoming_root=$opt_root/incoming
current_link=$opt_root/current
state_root=$expected_home/.local/share/robin-highscores
secret_root=$state_root/api-secrets
raw_root=$state_root/raw-content
backup_status_path=$state_root/status/backup-status.json
backup_root=$state_root/backups
backup_authority_key=$secret_root/backup-authority-hmac.key
runtime_fence=$state_root/runtime-fence
user_unit_root=$expected_home/.config/systemd/user
activation_lock=$opt_root/activation.lock
bootstrap_manifest_name=DEPLOY_BOOTSTRAP_SHA256SUMS

[ "$(id -un)" = "$expected_user" ] || fail "must run as the robinhood account"
[ "$(id -u)" -ne 0 ] || fail "must not run as root"
passwd_home=$(getent passwd "$expected_user" | cut -d: -f6)
[ "$passwd_home" = "$expected_home" ] || fail "robinhood account has an unexpected home directory"
[ "${HOME:-}" = "$expected_home" ] || fail "HOME must be exactly /home/robinhood"

case "$bundle_arg" in
    /*) ;;
    *) fail "bundle path must be absolute" ;;
esac
case "$expected_commit" in
    *[!0-9a-f]*|'') fail "expected commit must be lowercase hexadecimal" ;;
esac
[ "${#expected_commit}" -eq 40 ] || fail "expected commit must contain exactly 40 hexadecimal digits"
for expected_digest in "$expected_sums_sha256" "$expected_bootstrap_sums_sha256" \
    "$expected_plan_sha256" "$expected_vps_manifest_sha256"; do
    case "$expected_digest" in
        *[!0-9a-f]*|'') fail "expected digest must be lowercase hexadecimal" ;;
    esac
    [ "${#expected_digest}" -eq 64 ] || fail "expected digest must contain exactly 64 hexadecimal digits"
done

valid_proc_fd_path "$0" && valid_proc_fd_path "$bootstrap_manifest_fd" &&
    valid_proc_fd_path "$validator_fd" && valid_proc_fd_path "$manifest_tool_fd" &&
    valid_proc_fd_path "$plan_fd" && valid_proc_fd_path "$candidate_root_fd" &&
    valid_proc_fd_path "$activation_lock_fd" ||
    fail "deploy, bootstrap manifest, validator, manifest tool, plan, candidate root, and activation lock must be explicit inherited descriptors"
[ -f "$0" ] && [ "$(stat -Lc %u -- "$0")" -eq "$(id -u)" ] &&
    [ "$(stat -Lc %h -- "$0")" -eq 1 ] && [ "$(stat -Lc %a -- "$0")" = 500 ] ||
    fail "deploy descriptor has unsafe metadata"
[ -f "$bootstrap_manifest_fd" ] && [ "$(stat -Lc %u -- "$bootstrap_manifest_fd")" -eq "$(id -u)" ] &&
    [ "$(stat -Lc %h -- "$bootstrap_manifest_fd")" -eq 1 ] &&
    [ "$(stat -Lc %a -- "$bootstrap_manifest_fd")" = 400 ] ||
    fail "bootstrap-manifest descriptor has unsafe metadata"
[ -f "$validator_fd" ] && [ "$(stat -Lc %u -- "$validator_fd")" -eq "$(id -u)" ] &&
    [ "$(stat -Lc %h -- "$validator_fd")" -eq 1 ] && [ "$(stat -Lc %a -- "$validator_fd")" = 500 ] ||
    fail "validator descriptor has unsafe metadata"
[ -f "$manifest_tool_fd" ] && [ "$(stat -Lc %u -- "$manifest_tool_fd")" -eq "$(id -u)" ] &&
    [ "$(stat -Lc %h -- "$manifest_tool_fd")" -eq 1 ] && [ "$(stat -Lc %a -- "$manifest_tool_fd")" = 550 ] ||
    fail "manifest-tool descriptor has unsafe metadata"
[ -f "$plan_fd" ] && [ "$(stat -Lc %u -- "$plan_fd")" -eq "$(id -u)" ] &&
    [ "$(stat -Lc %h -- "$plan_fd")" -eq 1 ] && [ "$(stat -Lc %a -- "$plan_fd")" = 400 ] ||
    fail "plan descriptor has unsafe metadata"
[ -d "$candidate_root_fd" ] && [ "$(stat -Lc %u -- "$candidate_root_fd")" -eq "$(id -u)" ] &&
    [ "$(stat -Lc %a -- "$candidate_root_fd")" = 550 ] ||
    fail "candidate-root descriptor has unsafe metadata"
[ -f "$activation_lock_fd" ] && [ "$(stat -Lc %u -- "$activation_lock_fd")" -eq "$(id -u)" ] &&
    [ "$(stat -Lc %h -- "$activation_lock_fd")" -eq 1 ] && [ "$(stat -Lc %a -- "$activation_lock_fd")" = 600 ] ||
    fail "activation-lock descriptor has unsafe metadata"
activation_lock_number=${activation_lock_fd#/proc/self/fd/}
activation_lock_identity=$(stat -Lc %d:%i -- "$activation_lock_fd") || fail "cannot inspect activation-lock descriptor"
[ -f "$activation_lock" ] && [ ! -L "$activation_lock" ] &&
    [ "$(stat -c %d:%i -- "$activation_lock")" = "$activation_lock_identity" ] ||
    fail "activation-lock descriptor is not the canonical activation.lock inode"
[ "$(sha256sum "$bootstrap_manifest_fd" | cut -d' ' -f1)" = "$expected_bootstrap_sums_sha256" ] ||
    fail "trusted bootstrap checksum manifest differs from the out-of-band digest"
bootstrap_digest_for() {
    bootstrap_name=$1
    bootstrap_digest=$(awk -v wanted="$bootstrap_name" '$2 == wanted { print $1 }' "$bootstrap_manifest_fd")
    case "$bootstrap_digest" in *[!0-9a-f]*|'') return 1 ;; esac
    [ "${#bootstrap_digest}" -eq 64 ] || return 1
    [ "$(awk -v wanted="$bootstrap_name" '$2 == wanted { count += 1 } END { print count + 0 }' "$bootstrap_manifest_fd")" -eq 1 ] || return 1
    printf '%s\n' "$bootstrap_digest"
}
[ "$(wc -l < "$bootstrap_manifest_fd" | tr -d ' ')" -eq 3 ] || fail "bootstrap manifest must contain exactly three entries"
bootstrap_digest_for rollback-release.sh >/dev/null || fail "bootstrap manifest omits rollback-release.sh"
[ "$(sha256sum "$0" | cut -d' ' -f1)" = "$(bootstrap_digest_for deploy-release.sh)" ] ||
    fail "deploy descriptor differs from the pinned bootstrap manifest"
[ "$(sha256sum "$validator_fd" | cut -d' ' -f1)" = "$(bootstrap_digest_for validate-release-bundle.sh)" ] ||
    fail "validator descriptor differs from the pinned bootstrap manifest"

if [ "$deploy_mode" = candidate ]; then
    [ "$bundle_arg" = "$release_root/$expected_commit.partial" ] ||
        fail "bundle path must be exactly $release_root/$expected_commit.partial"
else
    [ "$bundle_arg" = "$release_root/$expected_commit" ] ||
        fail "resume path must be exactly $release_root/$expected_commit"
fi
bundle=$(realpath -e -- "$bundle_arg")
[ "$bundle" = "$bundle_arg" ] || fail "bundle path must be normalized and contain no symlink component"
[ -d "$bundle" ] && [ ! -L "$bundle" ] || fail "bundle root must be a real directory"
bundle_device=$(stat -c %d -- "$bundle")
bundle_inode=$(stat -c %i -- "$bundle")
bundle_uid=$(stat -c %u -- "$bundle")
[ "$bundle_uid" -eq "$(id -u)" ] || fail "bundle root is not owned by robinhood"
[ "$(stat -c %d:%i -- "$bundle")" = "$(stat -Lc %d:%i -- "$candidate_root_fd")" ] ||
    fail "bundle path does not name the retained candidate-root descriptor"
[ "$(sha256sum "$plan_fd" | cut -d' ' -f1)" = "$expected_plan_sha256" ] ||
    fail "retained VPS plan differs from its out-of-band digest"
[ "$(sha256sum "$candidate_root_fd/vps-release-manifest-v2.json" | cut -d' ' -f1)" = "$expected_vps_manifest_sha256" ] ||
    fail "retained candidate differs from its out-of-band VPS manifest digest"

ensure_candidate_root_authority() {
    [ -d "$candidate_root_fd" ] &&
        [ "$(stat -Lc %d:%i -- "$candidate_root_fd")" = "$bundle_device:$bundle_inode" ] &&
        [ "$(sha256sum "$candidate_root_fd/vps-release-manifest-v2.json" | cut -d' ' -f1)" = "$expected_vps_manifest_sha256" ] || return 1
    candidate_names=0
    for candidate_name in "$release_root/$expected_commit.partial" "$release"; do
        if [ -e "$candidate_name" ] || [ -L "$candidate_name" ]; then
            [ -d "$candidate_name" ] && [ ! -L "$candidate_name" ] &&
                [ "$(stat -c %d:%i -- "$candidate_name")" = "$bundle_device:$bundle_inode" ] || return 1
            candidate_names=$((candidate_names + 1))
        fi
    done
    [ "$candidate_names" -eq 1 ]
}

consume_vps_sources() {
    ensure_candidate_root_authority || return 1
    consumed_manifest=$("$manifest_tool_fd" consume-vps-sources-v2 \
        "$plan_fd" "$expected_plan_sha256" "$expected_vps_manifest_sha256" \
        --candidate-root-fd "${candidate_root_fd#/proc/self/fd/}" \
        --activation-lock-fd "$activation_lock_number") || return 1
    [ "$consumed_manifest" = "$expected_vps_manifest_sha256" ] || return 1
    ensure_candidate_root_authority
}

for command_path in /usr/bin/bash /usr/bin/bwrap /usr/bin/prlimit /usr/bin/curl /usr/bin/findmnt /usr/bin/flock /usr/bin/mv; do
    [ -f "$command_path" ] && [ ! -L "$command_path" ] && [ -x "$command_path" ] ||
        fail "required host executable is unavailable: $command_path"
done
[ "$(sha256sum /usr/bin/mv | cut -d' ' -f1)" = a781be46e6f27ca5d7e119225429a4a12ef941eea15ed2c44eea0c8bca7e4ebe ] ||
    fail "host /usr/bin/mv does not match the reviewed Debian 12 no-replace implementation"

tree_mount_target() {
    inspected_root=$1
    mount_inventory=$(/usr/bin/findmnt -Rrn -o TARGET --target "$inspected_root") || return 1
    old_ifs=$IFS
    IFS='
'
    for mount_target in $mount_inventory; do
        case "$mount_target" in
            "$inspected_root"|"$inspected_root"/*) printf '%s\n' "$mount_target" ;;
        esac
    done
    IFS=$old_ifs
}

validate_owned_tree() {
    inspected_root=$1
    allow_links=$2
    [ -d "$inspected_root" ] && [ ! -L "$inspected_root" ] || return 1
    inspected_uid=$(stat -c %u -- "$inspected_root") || return 1
    inspected_device=$(stat -c %d -- "$inspected_root") || return 1
    [ "$inspected_uid" -eq "$(id -u)" ] || return 1
    mount_target=$(tree_mount_target "$inspected_root") || return 1
    [ -z "$mount_target" ] || return 1
    device_inventory=$(find "$inspected_root" -xdev -printf '%D\n') || return 1
    [ "$(printf '%s\n' "$device_inventory" | LC_ALL=C sort -u)" = "$inspected_device" ] || return 1
    ownership_violation=$(find "$inspected_root" -xdev -mindepth 0 ! -uid "$inspected_uid" -print -quit) || return 1
    [ -z "$ownership_violation" ] || return 1
    hardlink_violation=$(find "$inspected_root" -xdev -type f -links +1 -print -quit) || return 1
    [ -z "$hardlink_violation" ] || return 1
    if [ "$allow_links" -eq 1 ]; then
        topology_violation=$(find "$inspected_root" -xdev -mindepth 1 ! -type d ! -type f ! -type l -print -quit) || return 1
    else
        topology_violation=$(find "$inspected_root" -xdev -mindepth 1 ! -type d ! -type f -print -quit) || return 1
    fi
    [ -z "$topology_violation" ]
}

remove_owned_tree() {
    cleanup_root=$1
    cleanup_parent=$2
    [ "$(dirname -- "$cleanup_root")" = "$cleanup_parent" ] || return 1
    if [ ! -e "$cleanup_root" ]; then
        [ ! -L "$cleanup_root" ] || return 1
        return 0
    fi
    validate_owned_tree "$cleanup_root" 1 || return 1
    find "$cleanup_root" -xdev -type d -exec chmod u+rwx -- {} + || return 1
    rm -rf --one-file-system -- "$cleanup_root" || return 1
    [ ! -e "$cleanup_root" ] && [ ! -L "$cleanup_root" ]
}

remove_tracked_link() {
    cleanup_link=$1
    expected_target=$2
    if [ -L "$cleanup_link" ]; then
        [ "$(stat -c %u -- "$cleanup_link")" -eq "$(id -u)" ] || return 1
        [ "$(readlink -- "$cleanup_link")" = "$expected_target" ] || return 1
        rm -f -- "$cleanup_link" || return 1
    elif [ -e "$cleanup_link" ]; then
        return 1
    fi
}

validate_release_descriptor_tree() {
    descriptor_root=$1
    /bin/sh "$validator_fd" "$descriptor_root" "$expected_commit" "$expected_sums_sha256" "$manifest_tool_fd"
}

umask 077
release=$release_root/$expected_commit
unit_stage_root=$user_unit_root/.robin-highscores-deploy-$expected_commit.stage
unit_backup_root=$user_unit_root/.robin-highscores-deploy-$expected_commit.recovery
temporary_link=$opt_root/.current-deploy-$expected_commit
restore_link=$opt_root/.current-deploy-$expected_commit.restore
prebackup_receipt=$opt_root/.deploy-source-backup-$expected_commit.receipt-v2
prebackup_receipt_temporary=$opt_root/.deploy-source-backup-$expected_commit.receipt-v2.new
target_backup_receipt=$opt_root/.deploy-target-backup-$expected_commit.receipt-v2
target_backup_receipt_temporary=$opt_root/.deploy-target-backup-$expected_commit.receipt-v2.new
authority_journal=$opt_root/.deploy-authority-$expected_commit
authority_journal_temporary=$opt_root/.deploy-authority-$expected_commit.new
prepared_journal=$opt_root/.deploy-prepared-$expected_commit
prepared_journal_temporary=$opt_root/.deploy-prepared-$expected_commit.new
release_installed=0
activation_started=0
migration_started=0
migration_completed=0
activation_complete=0
current_switched=0
current_durability_uncertain=0
transaction_prepared=0
previous_commit=
previous_target=
recovery_previous_target=
unit_stage_owned=0
unit_backup_owned=0
temporary_link_owned=0
restore_link_owned=0
preserve_recovery=0
resume_post_migration=0
prebackup_timer_disabled=0
backup_quiesced=0
prebackup_receipt_valid=0
prebackup_receipt_temporary_owned=0
target_backup_receipt_temporary_owned=0
authority_journal_owned=0
authority_journal_temporary_owned=0
resume_with_recovery=0
prepared_journal_owned=0
prepared_journal_temporary_owned=0

recovery_snapshot_sha256() {
    validate_owned_tree "$unit_backup_root" 0 || return 1
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

prepared_journal_expected_text() {
    journal_source=$1
    journal_recovery_sha=$2
    journal_phase=$3
    journal_candidate_device=$4
    journal_candidate_inode=$5
    printf '%s\n' \
        'schema=robin-highscores-activation-journal-v1' \
        'operation=deploy' \
        "source_commit=$journal_source" \
        "target_commit=$expected_commit" \
        "target_sha256sums_sha256=$expected_sums_sha256" \
        "recovery_root=$unit_backup_root" \
        "unit_stage_root=$unit_stage_root" \
        "target_selector_stage=$temporary_link" \
        "source_selector_stage=$restore_link" \
        "candidate_device=$journal_candidate_device" \
        "candidate_inode=$journal_candidate_inode" \
        "recovery_snapshot_sha256=$journal_recovery_sha" \
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
    journal_candidate_device=$(sed -n 's/^candidate_device=//p' "$journal_path") || return 1
    journal_candidate_inode=$(sed -n 's/^candidate_inode=//p' "$journal_path") || return 1
    case "$journal_candidate_device:$journal_candidate_inode" in
        none:none) ;;
        *[!0-9:]*|:*) return 1 ;;
    esac
    case "$journal_phase" in
        preparing) [ "$journal_recovery_sha" = none ] || return 1 ;;
        prepared) [ "$(recovery_snapshot_sha256)" = "$journal_recovery_sha" ] || return 1 ;;
        activation_complete) ;;
        *) return 1 ;;
    esac
    if [ "$journal_phase" != preparing ]; then
        case "$journal_recovery_sha" in *[!0-9a-f]*|'') return 1 ;; esac
        [ "${#journal_recovery_sha}" -eq 64 ] || return 1
    fi
    [ "$(cat -- "$journal_path")" = "$(prepared_journal_expected_text "$journal_source" "$journal_recovery_sha" "$journal_phase" "$journal_candidate_device" "$journal_candidate_inode")" ]
}

write_preparation_intent() {
    journal_source=$1
    journal_candidate_device=$bundle_device
    journal_candidate_inode=$bundle_inode
    [ ! -e "$prepared_journal" ] && [ ! -L "$prepared_journal" ] &&
        [ ! -e "$prepared_journal_temporary" ] && [ ! -L "$prepared_journal_temporary" ] || return 1
    prepared_journal_expected_text "$journal_source" none preparing \
        "$journal_candidate_device" "$journal_candidate_inode" >"$prepared_journal_temporary" || return 1
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
    [ ! -e "$release" ] && [ ! -L "$release" ] || return 1
    if [ "$journal_source" = none ]; then
        [ ! -e "$current_link" ] && [ ! -L "$current_link" ] || return 1
    else
        [ -L "$current_link" ] && [ "$(stat -c %u -- "$current_link")" -eq "$(id -u)" ] &&
            [ "$(readlink -- "$current_link")" = "releases/$journal_source" ] || return 1
    fi
    for unit in robin-highscores.target robin-highscores-api.service robin-highscores-worker.service \
        robin-highscores-backup.service robin-highscores-backup.timer; do
        if [ "$journal_source" = none ]; then
            [ ! -e "$user_unit_root/$unit" ] && [ ! -L "$user_unit_root/$unit" ] || return 1
        else
            [ -f "$user_unit_root/$unit" ] && [ ! -L "$user_unit_root/$unit" ] &&
                [ "$(stat -c %u -- "$user_unit_root/$unit")" -eq "$(id -u)" ] &&
                [ "$(stat -c %h -- "$user_unit_root/$unit")" -eq 1 ] &&
                [ "$(stat -c %a -- "$user_unit_root/$unit")" = 440 ] &&
                cmp -s -- "$user_unit_root/$unit" "$release_root/$journal_source/systemd/user/$unit" || return 1
        fi
    done
    if [ -e "$temporary_link" ] || [ -L "$temporary_link" ]; then
        remove_tracked_link "$temporary_link" "releases/$expected_commit" || return 1
    fi
    if [ -e "$restore_link" ] || [ -L "$restore_link" ]; then
        [ "$journal_source" != none ] || return 1
        remove_tracked_link "$restore_link" "releases/$journal_source" || return 1
    fi
    sync -f -- "$opt_root" || return 1
    if [ -e "$unit_stage_root" ] || [ -L "$unit_stage_root" ]; then
        remove_owned_tree "$unit_stage_root" "$user_unit_root" || return 1
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
        none) ;;
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
                preparing|prepared|activation_complete) ;;
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
    temporary_journal_candidate_device=$journal_candidate_device
    temporary_journal_candidate_inode=$journal_candidate_inode
    if [ -e "$prepared_journal" ] || [ -L "$prepared_journal" ]; then
        validate_prepared_journal "$temporary_journal_source" || return 1
        existing_journal_phase=$journal_phase
        existing_journal_recovery_sha=$journal_recovery_sha
        if [ "$existing_journal_phase:$temporary_journal_phase" != preparing:prepared ]; then
            [ "$existing_journal_recovery_sha" = "$temporary_journal_recovery_sha" ] || return 1
        fi
        [ "$journal_candidate_device" = "$temporary_journal_candidate_device" ] &&
            [ "$journal_candidate_inode" = "$temporary_journal_candidate_inode" ] || return 1
        case "$existing_journal_phase:$temporary_journal_phase" in
            preparing:prepared|prepared:activation_complete)
                mv -T -- "$prepared_journal_temporary" "$prepared_journal" || {
                    [ ! -e "$prepared_journal_temporary" ] && [ ! -L "$prepared_journal_temporary" ] &&
                        validate_prepared_journal "$temporary_journal_source" &&
                        [ "$journal_phase" = "$temporary_journal_phase" ] || return 1
                }
                ;;
            preparing:preparing|prepared:prepared|activation_complete:activation_complete)
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
    journal_candidate_device=$bundle_device
    journal_candidate_inode=$bundle_inode
    if [ -e "$prepared_journal_temporary" ] || [ -L "$prepared_journal_temporary" ]; then
        [ -f "$prepared_journal_temporary" ] && [ ! -L "$prepared_journal_temporary" ] &&
            [ "$(stat -c %u -- "$prepared_journal_temporary")" -eq "$(id -u)" ] &&
            [ "$(stat -c %h -- "$prepared_journal_temporary")" -eq 1 ] || return 1
        rm -f -- "$prepared_journal_temporary" || return 1
        sync -f -- "$opt_root" || return 1
    fi
    prepared_journal_expected_text "$journal_source" "$journal_recovery_sha" prepared \
        "$journal_candidate_device" "$journal_candidate_inode" >"$prepared_journal_temporary" || return 1
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

write_terminal_journal() {
    journal_source=$1
    validate_prepared_journal "$journal_source" || return 1
    [ "$journal_phase" = prepared ] || return 1
    journal_recovery_sha=$(sed -n 's/^recovery_snapshot_sha256=//p' "$prepared_journal") || return 1
    if [ -e "$prepared_journal_temporary" ] || [ -L "$prepared_journal_temporary" ]; then
        reconcile_prepared_journal_temporary || return 1
        validate_prepared_journal "$journal_source" && [ "$journal_phase" = activation_complete ] && return 0
        validate_prepared_journal "$journal_source" && [ "$journal_phase" = prepared ] || return 1
        journal_recovery_sha=$(sed -n 's/^recovery_snapshot_sha256=//p' "$prepared_journal") || return 1
    fi
    prepared_journal_expected_text "$journal_source" "$journal_recovery_sha" activation_complete \
        "$journal_candidate_device" "$journal_candidate_inode" >"$prepared_journal_temporary" || return 1
    prepared_journal_temporary_owned=1
    chmod 0400 -- "$prepared_journal_temporary" || return 1
    sync -f -- "$prepared_journal_temporary" || return 1
    if ! mv -T -- "$prepared_journal_temporary" "$prepared_journal"; then
        [ ! -e "$prepared_journal_temporary" ] && [ ! -L "$prepared_journal_temporary" ] &&
            validate_prepared_journal "$journal_source" && [ "$journal_phase" = activation_complete ] || return 1
    fi
    prepared_journal_temporary_owned=0
    sync -f -- "$opt_root" || return 1
    validate_prepared_journal "$journal_source" && [ "$journal_phase" = activation_complete ]
}

finish_terminal_cleanup() {
    journal_source=$1
    [ -L "$current_link" ] && [ "$(readlink -- "$current_link")" = "releases/$expected_commit" ] || return 1
    for unit in robin-highscores.target robin-highscores-api.service robin-highscores-worker.service \
        robin-highscores-backup.service robin-highscores-backup.timer; do
        [ -f "$user_unit_root/$unit" ] && [ ! -L "$user_unit_root/$unit" ] &&
            [ "$(stat -c %u -- "$user_unit_root/$unit")" -eq "$(id -u)" ] &&
            [ "$(stat -c %h -- "$user_unit_root/$unit")" -eq 1 ] &&
            [ "$(stat -c %a -- "$user_unit_root/$unit")" = 440 ] &&
            cmp -s -- "$user_unit_root/$unit" "$release/systemd/user/$unit" || return 1
    done
    [ "$(systemctl --user is-enabled robin-highscores.target)" = enabled ] || return 1
    [ "$(systemctl --user is-enabled robin-highscores-backup.timer)" = enabled ] || return 1
    for unit in robin-highscores.target robin-highscores-api.service robin-highscores-worker.service \
        robin-highscores-backup.timer; do
        [ "$(systemctl --user show --property=ActiveState --value "$unit")" = active ] || return 1
    done
    [ "$(systemctl --user show --property=ActiveState --value robin-highscores-backup.service)" = inactive ] || return 1
    probe http://127.0.0.1:8787/healthz 5 && probe http://127.0.0.1:8787/readyz 5 || return 1
    if [ -e "$temporary_link" ] || [ -L "$temporary_link" ]; then
        remove_tracked_link "$temporary_link" "releases/$expected_commit" || return 1
    fi
    if [ -e "$restore_link" ] || [ -L "$restore_link" ]; then
        [ "$journal_source" != none ] || return 1
        remove_tracked_link "$restore_link" "releases/$journal_source" || return 1
    fi
    if [ -e "$unit_stage_root" ] || [ -L "$unit_stage_root" ]; then
        remove_owned_tree "$unit_stage_root" "$user_unit_root" || return 1
    fi
    if [ -e "$unit_backup_root" ] || [ -L "$unit_backup_root" ]; then
        remove_owned_tree "$unit_backup_root" "$user_unit_root" || return 1
    fi
    sync -f -- "$user_unit_root" || return 1
    if [ -e "$authority_journal" ] || [ -L "$authority_journal" ]; then
        validate_authority_journal && [ "$authority_source" = "$journal_source" ] &&
            [ "$authority_runtime" = present ] && [ "$authority_schema" = verified ] &&
            [ "$authority_target_receipt" != none ] && [ "$authority_phase" = target_verified ] || return 1
        if [ -e "$prebackup_receipt" ] || [ -L "$prebackup_receipt" ]; then
            [ "$authority_source_receipt" != none ] &&
                validate_receipt_identity "$prebackup_receipt" "$authority_source_receipt" || return 1
            rm -f -- "$prebackup_receipt" || return 1
            sync -f -- "$opt_root" || return 1
        fi
        if [ -e "$target_backup_receipt" ] || [ -L "$target_backup_receipt" ]; then
            validate_receipt_identity "$target_backup_receipt" "$authority_target_receipt" || return 1
            rm -f -- "$target_backup_receipt" || return 1
        fi
        # Make all receipt retirement durable while the exact authority journal
        # still exists.  After this sync, absence of the journal implies that no
        # receipt can reappear across a crash or reboot.
        sync -f -- "$opt_root" || return 1
        rm -f -- "$authority_journal" || return 1
        authority_journal_owned=0
        sync -f -- "$opt_root" || return 1
    else
        # activation_complete is the durable terminal authority.  Once receipt and
        # authority cleanup has begun, a crash may leave all three paths absent;
        # exact live-state validation above makes that terminal prefix resumable.
        [ ! -e "$prebackup_receipt" ] && [ ! -L "$prebackup_receipt" ] &&
            [ ! -e "$target_backup_receipt" ] && [ ! -L "$target_backup_receipt" ] || return 1
    fi
    [ -f "$prepared_journal" ] && [ ! -L "$prepared_journal" ] &&
        [ "$(stat -c %u -- "$prepared_journal")" -eq "$(id -u)" ] &&
        [ "$(stat -c %h -- "$prepared_journal")" -eq 1 ] || return 1
    rm -f -- "$prepared_journal" || return 1
    prepared_journal_owned=0
    sync -f -- "$opt_root"
}

valid_digest() {
    case "$1" in *[!0-9a-f]*|'') return 1 ;; esac
    [ "${#1}" -eq 64 ]
}

backup_status_sha256() {
    if [ ! -e "$backup_status_path" ] && [ ! -L "$backup_status_path" ]; then
        printf '%s\n' absent
        return
    fi
    [ -f "$backup_status_path" ] && [ ! -L "$backup_status_path" ] &&
        [ "$(stat -c %u -- "$backup_status_path")" -eq "$(id -u)" ] &&
        [ "$(stat -c %h -- "$backup_status_path")" -eq 1 ] &&
        [ "$(stat -c %a -- "$backup_status_path")" = 400 ] || return 1
    backup_status_size=$(stat -c %s -- "$backup_status_path") || return 1
    [ "$backup_status_size" -gt 0 ] && [ "$backup_status_size" -le 1048576 ] || return 1
    sha256sum "$backup_status_path" | cut -d' ' -f1
}

require_fresh_backup_status() {
    backup_status_before=$1
    [ "$backup_status_before" != none ] || return 1
    backup_status_after=$(backup_status_sha256) || return 1
    valid_digest "$backup_status_after" && [ "$backup_status_after" != "$backup_status_before" ]
}

release_publication_lock_sha256() {
    receipt_release=$1
    receipt_vps_sha=$2
    receipt_publication_sha=$(/usr/bin/bash -c '
        set -eu
        exec {release_manifest_fd}<"$1"
        exec "$2" project-vps-publication-lock-v2 \
            --release-manifest-fd "$release_manifest_fd" \
            --expected-vps-release-manifest-sha256 "$3"
    ' robin-vps-publication-projection \
        "$receipt_release/vps-release-manifest-v2.json" "$manifest_tool_fd" "$receipt_vps_sha") || return 1
    valid_digest "$receipt_publication_sha" || return 1
    printf '%s\n' "$receipt_publication_sha"
}

transaction_backup_receipt() {
    receipt_admin=$1
    receipt_release=$2
    receipt_source=$3
    receipt_vps_sha=$4
    receipt_publication_sha=$5
    valid_digest "$receipt_vps_sha" && valid_digest "$receipt_publication_sha" || return 1
    case "$receipt_source" in *[!0-9a-f]*|'') return 1 ;; esac
    [ "${#receipt_source}" -eq 40 ] || return 1
    /usr/bin/bash -c '
        set -eu
        exec {backup_root_fd}<"$1"
        exec {status_fd}<"$2"
        exec {authority_key_fd}<"$3"
        exec {release_manifest_fd}<"$4"
        exec "$5" verify-transaction-backup \
            --backup-root-fd "$backup_root_fd" \
            --status-envelope-fd "$status_fd" \
            --backup-authority-key-fd "$authority_key_fd" \
            --expected-release-manifest-fd "$release_manifest_fd" \
            --expected-source-commit "$6" \
            --expected-vps-release-manifest-sha256 "$7" \
            --expected-publication-lock-sha256 "$8"
    ' robin-backup-verifier \
        "$backup_root" "$backup_status_path" "$backup_authority_key" \
        "$receipt_release/vps-release-manifest-v2.json" "$receipt_admin" \
        "$receipt_source" "$receipt_vps_sha" "$receipt_publication_sha"
}

validate_receipt_identity() {
    receipt_path=$1
    receipt_sha=$2
    valid_digest "$receipt_sha" || return 1
    [ -f "$receipt_path" ] && [ ! -L "$receipt_path" ] &&
        [ "$(stat -c %u -- "$receipt_path")" -eq "$(id -u)" ] &&
        [ "$(stat -c %h -- "$receipt_path")" -eq 1 ] &&
        [ "$(stat -c %a -- "$receipt_path")" = 400 ] || return 1
    receipt_size=$(stat -c %s -- "$receipt_path") || return 1
    [ "$receipt_size" -gt 0 ] && [ "$receipt_size" -le 131072 ] || return 1
    [ "$(sha256sum "$receipt_path" | cut -d' ' -f1)" = "$receipt_sha" ]
}

authenticate_transaction_backup_receipt() {
    receipt_path=$1
    receipt_admin=$2
    receipt_release=$3
    receipt_source=$4
    receipt_vps_sha=$5
    receipt_status_before=$6
    require_fresh_backup_status "$receipt_status_before" || return 1
    [ -f "$receipt_path" ] && [ ! -L "$receipt_path" ] &&
        [ "$(stat -c %u -- "$receipt_path")" -eq "$(id -u)" ] &&
        [ "$(stat -c %h -- "$receipt_path")" -eq 1 ] &&
        [ "$(stat -c %a -- "$receipt_path")" = 400 ] || return 1
    receipt_size=$(stat -c %s -- "$receipt_path") || return 1
    [ "$receipt_size" -gt 0 ] && [ "$receipt_size" -le 131072 ] || return 1
    receipt_publication_sha=$(release_publication_lock_sha256 "$receipt_release" "$receipt_vps_sha") || return 1
    verified_receipt=$(transaction_backup_receipt "$receipt_admin" "$receipt_release" \
        "$receipt_source" "$receipt_vps_sha" "$receipt_publication_sha") || return 1
    [ "$(cat -- "$receipt_path")" = "$verified_receipt" ] || return 1
    receipt_sha=$(sha256sum "$receipt_path" | cut -d' ' -f1) || return 1
    valid_digest "$receipt_sha"
}

validate_transaction_backup_receipt() {
    expected_receipt_path=$1
    expected_receipt_admin=$2
    expected_receipt_release=$3
    expected_receipt_source=$4
    expected_receipt_vps_sha=$5
    expected_receipt_sha=$6
    expected_receipt_status_before=$7
    valid_digest "$expected_receipt_sha" || return 1
    authenticate_transaction_backup_receipt "$expected_receipt_path" "$expected_receipt_admin" \
        "$expected_receipt_release" "$expected_receipt_source" "$expected_receipt_vps_sha" \
        "$expected_receipt_status_before" &&
        [ "$receipt_sha" = "$expected_receipt_sha" ]
}

reconcile_transaction_backup_receipt_temporary() {
    receipt_path=$1
    receipt_temporary=$2
    receipt_admin=$3
    receipt_release=$4
    receipt_source=$5
    receipt_vps_sha=$6
    receipt_status_before=$7
    [ -e "$receipt_temporary" ] || [ -L "$receipt_temporary" ] || return 0
    [ -f "$receipt_temporary" ] && [ ! -L "$receipt_temporary" ] &&
        [ "$(stat -c %u -- "$receipt_temporary")" -eq "$(id -u)" ] &&
        [ "$(stat -c %h -- "$receipt_temporary")" -eq 1 ] || return 1
    temporary_mode=$(stat -c %a -- "$receipt_temporary") || return 1
    case "$temporary_mode" in 400|600) ;; *) return 1 ;; esac
    temporary_size=$(stat -c %s -- "$receipt_temporary") || return 1
    if [ "$temporary_size" -eq 0 ] || [ "$temporary_size" -gt 131072 ]; then
        rm -f -- "$receipt_temporary" && sync -f -- "$opt_root"
        return
    fi
    receipt_publication_sha=$(release_publication_lock_sha256 "$receipt_release" "$receipt_vps_sha") || return 1
    verified_receipt=$(transaction_backup_receipt "$receipt_admin" "$receipt_release" \
        "$receipt_source" "$receipt_vps_sha" "$receipt_publication_sha") || return 1
    require_fresh_backup_status "$receipt_status_before" || return 1
    if [ "$(cat -- "$receipt_temporary")" != "$verified_receipt" ]; then
        rm -f -- "$receipt_temporary" && sync -f -- "$opt_root"
        return
    fi
    chmod 0400 -- "$receipt_temporary" || return 1
    sync -f -- "$receipt_temporary" || return 1
    if [ -e "$receipt_path" ] || [ -L "$receipt_path" ]; then
        authenticate_transaction_backup_receipt "$receipt_path" "$receipt_admin" "$receipt_release" \
            "$receipt_source" "$receipt_vps_sha" "$receipt_status_before" || return 1
        [ "$(cat -- "$receipt_path")" = "$(cat -- "$receipt_temporary")" ] || return 1
        rm -f -- "$receipt_temporary" || return 1
    elif ! mv -T -- "$receipt_temporary" "$receipt_path"; then
        [ ! -e "$receipt_temporary" ] && [ ! -L "$receipt_temporary" ] &&
            authenticate_transaction_backup_receipt "$receipt_path" "$receipt_admin" \
                "$receipt_release" "$receipt_source" "$receipt_vps_sha" \
                "$receipt_status_before" || return 1
    fi
    sync -f -- "$opt_root" || return 1
    authenticate_transaction_backup_receipt "$receipt_path" "$receipt_admin" "$receipt_release" \
        "$receipt_source" "$receipt_vps_sha" "$receipt_status_before" >/dev/null
}

write_transaction_backup_receipt() {
    receipt_path=$1
    receipt_temporary=$2
    receipt_admin=$3
    receipt_release=$4
    receipt_source=$5
    receipt_vps_sha=$6
    receipt_status_before=$7
    receipt_publication_sha=$(release_publication_lock_sha256 "$receipt_release" "$receipt_vps_sha") || return 1
    reconcile_transaction_backup_receipt_temporary "$receipt_path" "$receipt_temporary" \
        "$receipt_admin" "$receipt_release" "$receipt_source" "$receipt_vps_sha" \
        "$receipt_status_before" || return 1
    if [ -e "$receipt_path" ] || [ -L "$receipt_path" ]; then
        authenticate_transaction_backup_receipt "$receipt_path" "$receipt_admin" "$receipt_release" \
            "$receipt_source" "$receipt_vps_sha" "$receipt_status_before"
        return
    fi
    require_fresh_backup_status "$receipt_status_before" || return 1
    [ ! -e "$receipt_temporary" ] && [ ! -L "$receipt_temporary" ] || return 1
    if [ "$receipt_temporary" = "$prebackup_receipt_temporary" ]; then
        prebackup_receipt_temporary_owned=1
    elif [ "$receipt_temporary" = "$target_backup_receipt_temporary" ]; then
        target_backup_receipt_temporary_owned=1
    else
        return 1
    fi
    transaction_backup_receipt "$receipt_admin" "$receipt_release" "$receipt_source" \
        "$receipt_vps_sha" "$receipt_publication_sha" >"$receipt_temporary" || return 1
    chmod 0400 -- "$receipt_temporary" || return 1
    sync -f -- "$receipt_temporary" || return 1
    receipt_sha=$(sha256sum "$receipt_temporary" | cut -d' ' -f1) || return 1
    valid_digest "$receipt_sha" || return 1
    if [ -e "$receipt_path" ] || [ -L "$receipt_path" ]; then
        validate_transaction_backup_receipt "$receipt_path" "$receipt_admin" "$receipt_release" \
            "$receipt_source" "$receipt_vps_sha" "$receipt_sha" \
            "$receipt_status_before" || return 1
        rm -f -- "$receipt_temporary" || return 1
    elif ! mv -T -- "$receipt_temporary" "$receipt_path"; then
        [ ! -e "$receipt_temporary" ] && [ ! -L "$receipt_temporary" ] &&
            validate_transaction_backup_receipt "$receipt_path" "$receipt_admin" "$receipt_release" \
                "$receipt_source" "$receipt_vps_sha" "$receipt_sha" \
                "$receipt_status_before" || return 1
    fi
    if [ "$receipt_temporary" = "$prebackup_receipt_temporary" ]; then
        prebackup_receipt_temporary_owned=0
    else
        target_backup_receipt_temporary_owned=0
    fi
    sync -f -- "$opt_root" || return 1
    validate_transaction_backup_receipt "$receipt_path" "$receipt_admin" "$receipt_release" \
        "$receipt_source" "$receipt_vps_sha" "$receipt_sha" "$receipt_status_before"
}

try_adopt_transaction_backup_receipt() {
    adopt_path=$1
    adopt_temporary=$2
    adopt_admin=$3
    adopt_release=$4
    adopt_source=$5
    adopt_vps_sha=$6
    adopt_status_before=$7
    adopt_status_current=$(backup_status_sha256) || return 1
    if [ "$adopt_status_current" = "$adopt_status_before" ]; then
        # No backup ran after the durable *_backup_started boundary.  Return a
        # distinct status so the caller may admit and run exactly one backup.
        return 2
    fi
    # A changed status is authority-bearing external state.  It must authenticate
    # as this exact transaction; never overwrite a foreign or corrupt change.
    valid_digest "$adopt_status_current" || return 1
    if write_transaction_backup_receipt "$adopt_path" "$adopt_temporary" \
        "$adopt_admin" "$adopt_release" "$adopt_source" "$adopt_vps_sha" \
        "$adopt_status_before"; then
        return 0
    fi
    if [ "$adopt_temporary" = "$prebackup_receipt_temporary" ]; then
        adopt_owned=$prebackup_receipt_temporary_owned
    elif [ "$adopt_temporary" = "$target_backup_receipt_temporary" ]; then
        adopt_owned=$target_backup_receipt_temporary_owned
    else
        return 1
    fi
    if [ "$adopt_owned" -eq 1 ]; then
        [ -f "$adopt_temporary" ] && [ ! -L "$adopt_temporary" ] &&
            [ "$(stat -c %u -- "$adopt_temporary")" -eq "$(id -u)" ] &&
            [ "$(stat -c %h -- "$adopt_temporary")" -eq 1 ] &&
            rm -f -- "$adopt_temporary" && sync -f -- "$opt_root" || return 1
        if [ "$adopt_temporary" = "$prebackup_receipt_temporary" ]; then
            prebackup_receipt_temporary_owned=0
        else
            target_backup_receipt_temporary_owned=0
        fi
    fi
    return 1
}

authority_journal_expected_text() {
    printf '%s\n' \
        'schema=robin-highscores-deploy-authority-v1' \
        'operation=deploy' \
        "source_commit=$1" \
        "target_commit=$expected_commit" \
        "target_vps_release_manifest_sha256=$expected_vps_manifest_sha256" \
        "runtime_authority_state=$2" \
        "source_backup_receipt_sha256=$3" \
        "live_schema_version=$4" \
        "target_backup_receipt_sha256=$5" \
        "backup_status_before_sha256=$6" \
        "phase=$7"
}

validate_authority_journal_path() {
    inspected_authority_journal=$1
    [ -f "$inspected_authority_journal" ] && [ ! -L "$inspected_authority_journal" ] &&
        [ "$(stat -c %u -- "$inspected_authority_journal")" -eq "$(id -u)" ] &&
        [ "$(stat -c %h -- "$inspected_authority_journal")" -eq 1 ] &&
        [ "$(stat -c %a -- "$inspected_authority_journal")" = 400 ] || return 1
    authority_source=$(sed -n 's/^source_commit=//p' "$inspected_authority_journal") || return 1
    authority_runtime=$(sed -n 's/^runtime_authority_state=//p' "$inspected_authority_journal") || return 1
    authority_source_receipt=$(sed -n 's/^source_backup_receipt_sha256=//p' "$inspected_authority_journal") || return 1
    authority_schema=$(sed -n 's/^live_schema_version=//p' "$inspected_authority_journal") || return 1
    authority_target_receipt=$(sed -n 's/^target_backup_receipt_sha256=//p' "$inspected_authority_journal") || return 1
    authority_backup_status_before=$(sed -n 's/^backup_status_before_sha256=//p' "$inspected_authority_journal") || return 1
    authority_phase=$(sed -n 's/^phase=//p' "$inspected_authority_journal") || return 1
    case "$authority_source" in
        none) ;;
        *[!0-9a-f]*|'') return 1 ;;
        *) [ "${#authority_source}" -eq 40 ] || return 1 ;;
    esac
    case "$authority_runtime" in unobserved|initializing|present) ;; *) return 1 ;; esac
    for authority_digest in "$authority_source_receipt" "$authority_target_receipt"; do
        [ "$authority_digest" = none ] || valid_digest "$authority_digest" || return 1
    done
    case "$authority_backup_status_before" in
        none|absent) ;;
        *) valid_digest "$authority_backup_status_before" || return 1 ;;
    esac
    case "$authority_schema" in none|verified) ;; *) return 1 ;; esac
    authority_tuple_stage "$authority_source" "$authority_runtime" \
        "$authority_source_receipt" "$authority_schema" \
        "$authority_target_receipt" "$authority_backup_status_before" \
        "$authority_phase" || return 1
    [ "$(cat -- "$inspected_authority_journal")" = "$(authority_journal_expected_text \
        "$authority_source" "$authority_runtime" "$authority_source_receipt" \
        "$authority_schema" "$authority_target_receipt" \
        "$authority_backup_status_before" "$authority_phase")" ]
}

validate_authority_journal() {
    validate_authority_journal_path "$authority_journal"
}

authority_tuple_stage() {
    tuple_source=$1
    tuple_runtime=$2
    tuple_source_receipt=$3
    tuple_schema=$4
    tuple_target_receipt=$5
    tuple_backup_status_before=$6
    tuple_phase=$7
    if [ "$tuple_source" = none ]; then
        case "$tuple_runtime:$tuple_source_receipt:$tuple_schema:$tuple_target_receipt:$tuple_backup_status_before:$tuple_phase" in
            unobserved:none:none:none:none:prepared) authority_stage=0 ;;
            initializing:none:none:none:none:prepared) authority_stage=1 ;;
            present:none:none:none:none:runtime_ready) authority_stage=2 ;;
            present:none:verified:none:none:schema_verified) authority_stage=3 ;;
            present:none:verified:none:*:target_backup_started)
                [ "$tuple_backup_status_before" != none ] || return 1
                authority_stage=4
                ;;
            present:none:verified:*:*:target_verified)
                valid_digest "$tuple_target_receipt" || return 1
                [ "$tuple_backup_status_before" != none ] || return 1
                authority_stage=5
                ;;
            *) return 1 ;;
        esac
    else
        case "$tuple_runtime:$tuple_schema:$tuple_target_receipt:$tuple_backup_status_before:$tuple_phase" in
            unobserved:none:none:none:prepared)
                [ "$tuple_source_receipt" = none ] || return 1
                authority_stage=0
                ;;
            initializing:none:none:none:prepared)
                [ "$tuple_source_receipt" = none ] || return 1
                authority_stage=1
                ;;
            present:none:none:none:runtime_ready)
                [ "$tuple_source_receipt" = none ] || return 1
                authority_stage=2
                ;;
            present:none:none:*:source_backup_started)
                [ "$tuple_source_receipt" = none ] || return 1
                [ "$tuple_backup_status_before" != none ] || return 1
                authority_stage=3
                ;;
            present:none:none:*:source_verified)
                valid_digest "$tuple_source_receipt" || return 1
                [ "$tuple_backup_status_before" != none ] || return 1
                authority_stage=4
                ;;
            present:verified:none:*:schema_verified)
                valid_digest "$tuple_source_receipt" || return 1
                [ "$tuple_backup_status_before" != none ] || return 1
                authority_stage=5
                ;;
            present:verified:none:*:target_backup_started)
                valid_digest "$tuple_source_receipt" || return 1
                [ "$tuple_backup_status_before" != none ] || return 1
                authority_stage=6
                ;;
            present:verified:*:*:target_verified)
                valid_digest "$tuple_source_receipt" && valid_digest "$tuple_target_receipt" || return 1
                [ "$tuple_backup_status_before" != none ] || return 1
                authority_stage=7
                ;;
            *) return 1 ;;
        esac
    fi
}

authority_transition_allowed() {
    old_source=$1
    old_runtime=$2
    old_source_receipt=$3
    old_schema=$4
    old_target_receipt=$5
    old_backup_status_before=$6
    old_phase=$7
    new_source=$8
    new_runtime=$9
    shift 9
    new_source_receipt=$1
    new_schema=$2
    new_target_receipt=$3
    new_backup_status_before=$4
    new_phase=$5
    [ "$old_source" = "$new_source" ] || return 1
    authority_tuple_stage "$old_source" "$old_runtime" "$old_source_receipt" \
        "$old_schema" "$old_target_receipt" "$old_backup_status_before" \
        "$old_phase" || return 1
    old_stage=$authority_stage
    authority_tuple_stage "$new_source" "$new_runtime" "$new_source_receipt" \
        "$new_schema" "$new_target_receipt" "$new_backup_status_before" \
        "$new_phase" || return 1
    new_stage=$authority_stage
    if [ "$new_stage" -eq "$old_stage" ]; then
        [ "$old_runtime:$old_source_receipt:$old_schema:$old_target_receipt:$old_backup_status_before:$old_phase" = \
            "$new_runtime:$new_source_receipt:$new_schema:$new_target_receipt:$new_backup_status_before:$new_phase" ]
    else
        [ "$new_stage" -eq $((old_stage + 1)) ] || return 1
        case "$old_source:$old_stage:$new_stage" in
            *:0:1|*:1:2)
                [ "$old_source_receipt:$old_schema:$old_target_receipt:$old_backup_status_before" = \
                    "$new_source_receipt:$new_schema:$new_target_receipt:$new_backup_status_before" ]
                ;;
            none:2:3)
                [ "$old_source_receipt:$old_target_receipt:$old_backup_status_before" = \
                    "$new_source_receipt:$new_target_receipt:$new_backup_status_before" ]
                ;;
            none:3:4)
                [ "$old_runtime:$old_source_receipt:$old_schema:$old_target_receipt" = \
                    "$new_runtime:$new_source_receipt:$new_schema:$new_target_receipt" ] &&
                    [ "$old_backup_status_before" = none ] &&
                    [ "$new_backup_status_before" != none ]
                ;;
            none:4:5)
                [ "$old_source_receipt:$old_schema:$old_backup_status_before" = \
                    "$new_source_receipt:$new_schema:$new_backup_status_before" ] &&
                    [ "$old_target_receipt" = none ]
                ;;
            *:2:3)
                [ "$old_runtime:$old_source_receipt:$old_schema:$old_target_receipt" = \
                    "$new_runtime:$new_source_receipt:$new_schema:$new_target_receipt" ] &&
                    [ "$old_backup_status_before" = none ] &&
                    [ "$new_backup_status_before" != none ]
                ;;
            *:3:4)
                [ "$old_schema:$old_target_receipt:$old_backup_status_before" = \
                    "$new_schema:$new_target_receipt:$new_backup_status_before" ] &&
                    [ "$old_source_receipt" = none ]
                ;;
            *:4:5)
                [ "$old_source_receipt:$old_target_receipt:$old_backup_status_before" = \
                    "$new_source_receipt:$new_target_receipt:$new_backup_status_before" ]
                ;;
            *:5:6)
                [ "$old_runtime:$old_source_receipt:$old_schema:$old_target_receipt" = \
                    "$new_runtime:$new_source_receipt:$new_schema:$new_target_receipt" ] &&
                    [ "$new_backup_status_before" != none ] &&
                    [ "$old_backup_status_before" != "$new_backup_status_before" ]
                ;;
            *:6:7)
                [ "$old_source_receipt:$old_schema:$old_backup_status_before" = \
                    "$new_source_receipt:$new_schema:$new_backup_status_before" ] &&
                    [ "$old_target_receipt" = none ]
                ;;
            *) return 1 ;;
        esac
    fi
}

reconcile_authority_journal_temporary() {
    [ -e "$authority_journal_temporary" ] || [ -L "$authority_journal_temporary" ] || return 0
    validate_authority_journal_path "$authority_journal_temporary" || return 1
    temporary_authority_source=$authority_source
    temporary_authority_runtime=$authority_runtime
    temporary_authority_source_receipt=$authority_source_receipt
    temporary_authority_schema=$authority_schema
    temporary_authority_target_receipt=$authority_target_receipt
    temporary_authority_backup_status_before=$authority_backup_status_before
    temporary_authority_phase=$authority_phase
    if [ -e "$authority_journal" ] || [ -L "$authority_journal" ]; then
        validate_authority_journal || return 1
        authority_transition_allowed \
            "$authority_source" "$authority_runtime" "$authority_source_receipt" \
            "$authority_schema" "$authority_target_receipt" \
            "$authority_backup_status_before" "$authority_phase" \
            "$temporary_authority_source" "$temporary_authority_runtime" \
            "$temporary_authority_source_receipt" "$temporary_authority_schema" \
            "$temporary_authority_target_receipt" \
            "$temporary_authority_backup_status_before" \
            "$temporary_authority_phase" || return 1
    else
        authority_tuple_stage "$temporary_authority_source" "$temporary_authority_runtime" \
            "$temporary_authority_source_receipt" "$temporary_authority_schema" \
            "$temporary_authority_target_receipt" \
            "$temporary_authority_backup_status_before" \
            "$temporary_authority_phase" || return 1
        [ "$authority_stage" -eq 0 ] || return 1
    fi
    if ! mv -T -- "$authority_journal_temporary" "$authority_journal"; then
        [ ! -e "$authority_journal_temporary" ] && [ ! -L "$authority_journal_temporary" ] || return 1
    fi
    sync -f -- "$opt_root" || return 1
    validate_authority_journal &&
        [ "$authority_source" = "$temporary_authority_source" ] &&
        [ "$authority_runtime" = "$temporary_authority_runtime" ] &&
        [ "$authority_source_receipt" = "$temporary_authority_source_receipt" ] &&
        [ "$authority_schema" = "$temporary_authority_schema" ] &&
        [ "$authority_target_receipt" = "$temporary_authority_target_receipt" ] &&
        [ "$authority_backup_status_before" = "$temporary_authority_backup_status_before" ] &&
        [ "$authority_phase" = "$temporary_authority_phase" ]
}

publish_authority_journal() {
    journal_source=$1
    journal_runtime=$2
    journal_source_receipt=$3
    journal_schema=$4
    journal_target_receipt=$5
    journal_backup_status_before=$6
    journal_phase=$7
    reconcile_authority_journal_temporary || return 1
    [ ! -e "$authority_journal_temporary" ] && [ ! -L "$authority_journal_temporary" ] || return 1
    authority_journal_expected_text "$journal_source" "$journal_runtime" "$journal_source_receipt" \
        "$journal_schema" "$journal_target_receipt" "$journal_backup_status_before" \
        "$journal_phase" >"$authority_journal_temporary" || return 1
    authority_journal_temporary_owned=1
    chmod 0400 -- "$authority_journal_temporary" || return 1
    sync -f -- "$authority_journal_temporary" || return 1
    validate_authority_journal_path "$authority_journal_temporary" &&
        [ "$authority_source" = "$journal_source" ] &&
        [ "$authority_runtime" = "$journal_runtime" ] &&
        [ "$authority_source_receipt" = "$journal_source_receipt" ] &&
        [ "$authority_schema" = "$journal_schema" ] &&
        [ "$authority_target_receipt" = "$journal_target_receipt" ] &&
        [ "$authority_backup_status_before" = "$journal_backup_status_before" ] &&
        [ "$authority_phase" = "$journal_phase" ] || return 1
    if [ -e "$authority_journal" ] || [ -L "$authority_journal" ]; then
        validate_authority_journal || return 1
        if [ "$(cat -- "$authority_journal")" = "$(cat -- "$authority_journal_temporary")" ]; then
            rm -f -- "$authority_journal_temporary" || return 1
            authority_journal_temporary_owned=0
            authority_journal_owned=1
            sync -f -- "$opt_root" || return 1
            validate_authority_journal
            return
        fi
        authority_transition_allowed \
            "$authority_source" "$authority_runtime" "$authority_source_receipt" \
            "$authority_schema" "$authority_target_receipt" \
            "$authority_backup_status_before" "$authority_phase" \
            "$journal_source" "$journal_runtime" "$journal_source_receipt" \
            "$journal_schema" "$journal_target_receipt" "$journal_backup_status_before" \
            "$journal_phase" || return 1
    else
        authority_tuple_stage "$journal_source" "$journal_runtime" "$journal_source_receipt" \
            "$journal_schema" "$journal_target_receipt" "$journal_backup_status_before" \
            "$journal_phase" || return 1
        [ "$authority_stage" -eq 0 ] || return 1
    fi
    if ! mv -T -- "$authority_journal_temporary" "$authority_journal"; then
        [ ! -e "$authority_journal_temporary" ] && [ ! -L "$authority_journal_temporary" ] || return 1
    fi
    authority_journal_temporary_owned=0
    authority_journal_owned=1
    sync -f -- "$opt_root" || return 1
    validate_authority_journal &&
        [ "$authority_source" = "$journal_source" ] &&
        [ "$authority_runtime" = "$journal_runtime" ] &&
        [ "$authority_source_receipt" = "$journal_source_receipt" ] &&
        [ "$authority_schema" = "$journal_schema" ] &&
        [ "$authority_target_receipt" = "$journal_target_receipt" ] &&
        [ "$authority_backup_status_before" = "$journal_backup_status_before" ] &&
        [ "$authority_phase" = "$journal_phase" ]
}

estimate_backup_space() {
    estimate_admin=$1
    estimate_release=$2
    estimate_revalidates_candidate=0
    if [ "$estimate_release" = "$release" ]; then
        ensure_candidate_root_authority || return 1
        [ -d "$release" ] && [ ! -L "$release" ] &&
            [ "$(stat -c %d:%i -- "$release")" = "$bundle_device:$bundle_inode" ] || return 1
        estimate_revalidates_candidate=1
    fi
    "$estimate_admin" --config "$estimate_release/config/highscores-server.toml" \
        estimate-backup-space \
        --release-manifest-path "$estimate_release/vps-release-manifest-v2.json" \
        --backup-root "$backup_root" \
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
        >/dev/null || {
        [ "$estimate_revalidates_candidate" -eq 0 ] || ensure_candidate_root_authority
        return 1
    }
    [ "$estimate_revalidates_candidate" -eq 0 ] || {
        ensure_candidate_root_authority &&
            [ -d "$release" ] && [ ! -L "$release" ] &&
            [ "$(stat -c %d:%i -- "$release")" = "$bundle_device:$bundle_inode" ]
    }
}

runtime_authority_probe() {
    runtime_state=$1
    runtime_output=$("$candidate_root_fd/bin/robin-highscores-admin" \
        probe-runtime-authority-v2 \
        --candidate-release-root-fd "${candidate_root_fd#/proc/self/fd/}" \
        --expected-vps-release-manifest-sha256 "$expected_vps_manifest_sha256" \
        --backup-authority-state "$runtime_state") || return 1
    [ "$runtime_output" = "{\"backup_authority_state\":\"$runtime_state\",\"schema_version\":2,\"source_commit\":\"$expected_commit\",\"vps_release_manifest_sha256\":\"$expected_vps_manifest_sha256\"}" ]
}

initialize_or_adopt_runtime_authority() {
    runtime_source=$1
    if [ "$runtime_source" != none ]; then
        runtime_authority_probe present || return 1
        "$candidate_root_fd/bin/robin-highscores-admin" \
            --config "$candidate_root_fd/config/highscores-server.toml" \
            complete-backup-authority-key-v2 \
            --source-commit "$expected_commit" \
            --activation-lock-fd "$activation_lock_number" \
            --candidate-release-root-fd "${candidate_root_fd#/proc/self/fd/}" \
            --expected-vps-release-manifest-sha256 "$expected_vps_manifest_sha256" \
            >/dev/null || return 1
        return 0
    fi
    if runtime_authority_probe present 2>/dev/null; then
        "$candidate_root_fd/bin/robin-highscores-admin" \
            --config "$candidate_root_fd/config/highscores-server.toml" \
            complete-backup-authority-key-v2 \
            --source-commit "$expected_commit" \
            --activation-lock-fd "$activation_lock_number" \
            --candidate-release-root-fd "${candidate_root_fd#/proc/self/fd/}" \
            --expected-vps-release-manifest-sha256 "$expected_vps_manifest_sha256" \
            >/dev/null || return 1
        return 0
    fi
    key_intent=$secret_root/.backup-authority-hmac-key.intent-v1.json
    key_intent_temporary=$secret_root/.backup-authority-hmac-key.intent-v1.json.new
    runtime_staging=$state_root/.runtime-fence-$expected_commit.partial
    runtime_intent=$state_root/.runtime-fence-init-v1.json
    runtime_intent_temporary=$state_root/.runtime-fence-init-v1.json.new
    runtime_intent_writing=$state_root/.runtime-fence-init-v1.json.writing
    if [ ! -e "$backup_authority_key" ] && [ ! -L "$backup_authority_key" ] &&
        [ ! -e "$key_intent" ] && [ ! -L "$key_intent" ] &&
        [ ! -e "$key_intent_temporary" ] && [ ! -L "$key_intent_temporary" ] &&
        [ ! -e "$runtime_fence" ] && [ ! -L "$runtime_fence" ] &&
        [ ! -e "$runtime_staging" ] && [ ! -L "$runtime_staging" ] &&
        [ ! -e "$runtime_intent" ] && [ ! -L "$runtime_intent" ] &&
        [ ! -e "$runtime_intent_temporary" ] && [ ! -L "$runtime_intent_temporary" ] &&
        [ ! -e "$runtime_intent_writing" ] && [ ! -L "$runtime_intent_writing" ]; then
        runtime_authority_probe absent || return 1
    fi
    "$candidate_root_fd/bin/robin-highscores-admin" \
        --config "$candidate_root_fd/config/highscores-server.toml" \
        initialize-backup-authority-key-v2 \
        --source-commit "$expected_commit" \
        --activation-lock-fd "$activation_lock_number" \
        >/dev/null || return 1
    "$manifest_tool_fd" initialize-vps-runtime-fence-v1 "$expected_commit" \
        --activation-lock-fd "$activation_lock_number" || return 1
    runtime_authority_probe present || return 1
    "$candidate_root_fd/bin/robin-highscores-admin" \
        --config "$candidate_root_fd/config/highscores-server.toml" \
        complete-backup-authority-key-v2 \
        --source-commit "$expected_commit" \
        --activation-lock-fd "$activation_lock_number" \
        --candidate-release-root-fd "${candidate_root_fd#/proc/self/fd/}" \
        --expected-vps-release-manifest-sha256 "$expected_vps_manifest_sha256" \
        >/dev/null
}

verify_live_schema() {
    schema_admin=$1
    schema_release=$2
    schema_vps_sha=$3
    if [ "$schema_release" = "$candidate_root_fd" ]; then
        schema_output=$("$schema_admin" verify-live-database-schema-v2 \
            --candidate-release-root-fd "${candidate_root_fd#/proc/self/fd/}" \
            --expected-vps-release-manifest-sha256 "$schema_vps_sha") || return 1
    else
        schema_output=$(/usr/bin/bash -c '
            set -eu
            exec {release_root_fd}<"$1"
            exec "$2" verify-live-database-schema-v2 \
                --candidate-release-root-fd "$release_root_fd" \
                --expected-vps-release-manifest-sha256 "$3"
        ' robin-live-schema "$schema_release" "$schema_admin" "$schema_vps_sha") || return 1
    fi
    [ -n "$schema_output" ]
}

restore_pre_migration_state() {
    restore_failed=0
    for unit in \
        robin-highscores.target \
        robin-highscores-api.service \
        robin-highscores-worker.service \
        robin-highscores-backup.service \
        robin-highscores-backup.timer
    do
        destination=$user_unit_root/$unit
        if [ -f "$unit_backup_root/units/$unit" ] && [ ! -L "$unit_backup_root/units/$unit" ]; then
            if [ -e "$destination" ] || [ -L "$destination" ]; then
                [ -f "$destination" ] && [ ! -L "$destination" ] &&
                    [ "$(stat -c %u -- "$destination")" -eq "$(id -u)" ] &&
                    [ "$(stat -c %h -- "$destination")" -eq 1 ] &&
                    [ "$(stat -c %a -- "$destination")" = 440 ] &&
                    { cmp -s -- "$destination" "$unit_backup_root/units/$unit" ||
                        cmp -s -- "$destination" "$release/systemd/user/$unit"; } || {
                    restore_failed=1
                    continue
                }
            fi
            restore_temporary=$unit_stage_root/.restore-$unit
            [ ! -e "$restore_temporary" ] && [ ! -L "$restore_temporary" ] || {
                restore_failed=1
                continue
            }
            install -m 0440 -- "$unit_backup_root/units/$unit" "$restore_temporary" || {
                restore_failed=1
                continue
            }
            mv -T -- "$restore_temporary" "$destination" || restore_failed=1
        elif [ -f "$unit_backup_root/absent/$unit" ]; then
            if [ -e "$destination" ] || [ -L "$destination" ]; then
                [ -f "$destination" ] && [ ! -L "$destination" ] &&
                    [ "$(stat -c %u -- "$destination")" -eq "$(id -u)" ] &&
                    [ "$(stat -c %h -- "$destination")" -eq 1 ] &&
                    [ "$(stat -c %a -- "$destination")" = 440 ] &&
                    cmp -s -- "$destination" "$release/systemd/user/$unit" &&
                    rm -f -- "$destination" || restore_failed=1
            fi
        else
            restore_failed=1
        fi
    done
    if [ -n "$previous_target" ]; then
        if [ -L "$current_link" ]; then
            actual_target=$(readlink -- "$current_link")
            [ "$actual_target" = "$previous_target" ] ||
                [ "$actual_target" = "releases/$expected_commit" ] || restore_failed=1
        elif [ -e "$current_link" ]; then
            restore_failed=1
        fi
        if [ "$restore_failed" -eq 0 ] && [ -L "$restore_link" ] &&
            [ "$(readlink -- "$restore_link")" = "$previous_target" ]; then
            mv -T -- "$restore_link" "$current_link" || restore_failed=1
        elif [ ! -L "$current_link" ] || [ "$(readlink -- "$current_link")" != "$previous_target" ]; then
            restore_failed=1
        fi
    elif [ -e "$current_link" ] || [ -L "$current_link" ]; then
        [ -L "$current_link" ] &&
            [ "$(stat -c %u -- "$current_link")" -eq "$(id -u)" ] &&
            [ "$(readlink -- "$current_link")" = "releases/$expected_commit" ] &&
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
        for unit in \
            robin-highscores.target \
            robin-highscores-api.service \
            robin-highscores-worker.service \
            robin-highscores-backup.timer \
            robin-highscores-backup.service
        do
            systemctl --user stop "$unit" >/dev/null 2>&1 || restore_failed=1
        done
        for unit in \
            robin-highscores.target \
            robin-highscores-api.service \
            robin-highscores-worker.service \
            robin-highscores-backup.timer \
            robin-highscores-backup.service
        do
            if [ -f "$unit_backup_root/active/$unit" ]; then
                systemctl --user start "$unit" || restore_failed=1
            fi
        done
        for unit in \
            robin-highscores-backup.service \
            robin-highscores-backup.timer \
            robin-highscores-worker.service \
            robin-highscores-api.service \
            robin-highscores.target
        do
            if [ ! -f "$unit_backup_root/active/$unit" ]; then
                systemctl --user stop "$unit" >/dev/null 2>&1 || restore_failed=1
            fi
        done
        for wants_directory in "$user_unit_root/default.target.wants" "$user_unit_root/timers.target.wants"; do
            if [ -e "$wants_directory" ] || [ -L "$wants_directory" ]; then
                [ -d "$wants_directory" ] && [ ! -L "$wants_directory" ] &&
                    sync -f -- "$wants_directory" || restore_failed=1
            fi
        done
        if [ -f "$unit_backup_root/active/robin-highscores-api.service" ]; then
            probe http://127.0.0.1:8787/healthz 5 || restore_failed=1
            probe http://127.0.0.1:8787/readyz 5 || restore_failed=1
        fi
    fi
    return "$restore_failed"
}

converge_new_selected_stopped() {
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
            if cmp -s -- "$destination" "$release/systemd/user/$unit"; then
                destination_allowed=1
            elif [ -f "$unit_backup_root/units/$unit" ] &&
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
            cmp -s -- "$staged_unit" "$release/systemd/user/$unit"; then
            if ! mv -T -- "$staged_unit" "$destination"; then
                if [ -e "$staged_unit" ] || [ -L "$staged_unit" ] ||
                    [ ! -f "$destination" ] || [ -L "$destination" ] ||
                    ! cmp -s -- "$destination" "$release/systemd/user/$unit"; then
                    converge_failed=1
                    continue
                fi
            fi
        elif [ ! -f "$destination" ] || [ -L "$destination" ] ||
            [ "$(stat -c %u -- "$destination")" -ne "$(id -u)" ] ||
            [ "$(stat -c %h -- "$destination")" -ne 1 ] ||
            [ "$(stat -c %a -- "$destination")" != 440 ] ||
            ! cmp -s -- "$destination" "$release/systemd/user/$unit"; then
            converge_failed=1
            continue
        fi
        sync -f -- "$destination" || converge_failed=1
    done
    sync -f -- "$user_unit_root" || converge_failed=1
    systemctl --user daemon-reload || converge_failed=1
    if [ -L "$current_link" ] && [ "$(readlink -- "$current_link")" = "releases/$expected_commit" ]; then
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
                if ln -s -- "releases/$expected_commit" "$temporary_link"; then
                    temporary_link_owned=1
                    sync -f -- "$opt_root" || converge_failed=1
                else
                    converge_failed=1
                fi
            fi
            if [ "$temporary_link_owned" -eq 1 ] && [ -L "$temporary_link" ] &&
                [ "$(stat -c %u -- "$temporary_link")" -eq "$(id -u)" ] &&
                [ "$(readlink -- "$temporary_link")" = "releases/$expected_commit" ]; then
                if ! mv -T -- "$temporary_link" "$current_link"; then
                    [ -L "$current_link" ] &&
                        [ "$(readlink -- "$current_link")" = "releases/$expected_commit" ] &&
                        [ ! -e "$temporary_link" ] && [ ! -L "$temporary_link" ] || converge_failed=1
                fi
            else
                converge_failed=1
            fi
            if [ ! -e "$temporary_link" ] && [ ! -L "$temporary_link" ]; then
                temporary_link_owned=0
            fi
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

probe() {
    endpoint=$1
    attempts=$2
    count=0
    while [ "$count" -lt "$attempts" ]; do
        if /usr/bin/curl --fail --silent --show-error --max-time 2 "$endpoint" >/dev/null 2>&1; then
            return 0
        fi
        count=$((count + 1))
        sleep 1
    done
    return 1
}

validate_empty_clean_directory() {
    clean_directory=$1
    [ -d "$clean_directory" ] && [ ! -L "$clean_directory" ] &&
        [ "$(realpath -e -- "$clean_directory")" = "$clean_directory" ] &&
        [ "$(stat -c %u -- "$clean_directory")" -eq "$(id -u)" ] &&
        [ "$(stat -c %a -- "$clean_directory")" = 700 ] &&
        validate_owned_tree "$clean_directory" 0 || return 1
    clean_directory_entry=$(find "$clean_directory" -mindepth 1 -print -quit) || return 1
    [ -z "$clean_directory_entry" ]
}

validate_clean_first_deploy_state() {
    for parent in "$opt_root" "$incoming_root" "$release_root"; do
        [ -d "$parent" ] && [ ! -L "$parent" ] &&
            [ "$(realpath -e -- "$parent")" = "$parent" ] &&
            [ "$(stat -c %u -- "$parent")" -eq "$(id -u)" ] &&
            [ "$(stat -c %a -- "$parent")" = 750 ] || return 1
    done
    validate_owned_tree "$opt_root" 0 && validate_owned_tree "$incoming_root" 0 &&
        validate_owned_tree "$release_root" 0 || return 1
    clean_opt_marks=$(find "$opt_root" -mindepth 1 -maxdepth 1 -printf .) || return 1
    [ "$clean_opt_marks" = ... ] || return 1
    [ -f "$activation_lock" ] && [ ! -L "$activation_lock" ] &&
        [ "$(stat -c %u -- "$activation_lock")" -eq "$(id -u)" ] &&
        [ "$(stat -c %a -- "$activation_lock")" = 600 ] &&
        [ "$(stat -c %h -- "$activation_lock")" -eq 1 ] || return 1
    if [ "$deploy_mode" = candidate ]; then
        clean_release_marks=$(find "$release_root" -mindepth 1 -maxdepth 1 -printf .) || return 1
        [ "$clean_release_marks" = . ] &&
            [ -d "$release_root/$expected_commit.partial" ] &&
            [ ! -L "$release_root/$expected_commit.partial" ] || return 1
        clean_incoming_marks=$(find "$incoming_root" -mindepth 1 -maxdepth 1 -printf .) || return 1
        [ "$clean_incoming_marks" = . ] &&
            [ -d "$incoming_root/.sources-$expected_commit" ] &&
            [ ! -L "$incoming_root/.sources-$expected_commit" ] || return 1
    else
        clean_release_marks=$(find "$release_root" -mindepth 1 -maxdepth 1 -printf .) || return 1
        [ "$clean_release_marks" = . ] &&
            [ -d "$release_root/$expected_commit" ] &&
            [ ! -L "$release_root/$expected_commit" ] || return 1
        clean_incoming_marks=$(find "$incoming_root" -mindepth 1 -maxdepth 1 -printf .) || return 1
        case "$clean_incoming_marks" in
            '') ;;
            .) [ -d "$incoming_root/.sources-$expected_commit" ] &&
                [ ! -L "$incoming_root/.sources-$expected_commit" ] || return 1 ;;
            *) return 1 ;;
        esac
    fi
    [ ! -e "$state_root/root-once" ] && [ ! -L "$state_root/root-once" ] || return 1
    [ -d "$state_root" ] && [ ! -L "$state_root" ] &&
        [ "$(realpath -e -- "$state_root")" = "$state_root" ] &&
        [ "$(stat -c %u -- "$state_root")" -eq "$(id -u)" ] &&
        [ "$(stat -c %a -- "$state_root")" = 700 ] || return 1
    clean_state_marks=$(find "$state_root" -mindepth 1 -maxdepth 1 -printf .) || return 1
    [ "$clean_state_marks" = ..... ] || return 1
    for clean_directory in "$state_root/database" "$state_root/replays" \
        "$state_root/campaign-states"; do
        validate_empty_clean_directory "$clean_directory" || return 1
    done
    [ ! -e "$state_root/backups" ] && [ ! -L "$state_root/backups" ] &&
        [ ! -e "$state_root/status" ] && [ ! -L "$state_root/status" ] || return 1
    [ -d "$secret_root" ] && [ ! -L "$secret_root" ] &&
        [ "$(stat -c %u -- "$secret_root")" -eq "$(id -u)" ] &&
        [ "$(stat -c %a -- "$secret_root")" = 700 ] || return 1
    clean_secret_marks=$(find "$secret_root" -mindepth 1 -maxdepth 1 -printf .) || return 1
    [ "$clean_secret_marks" = .... ] || return 1
    for secret_name in cursor-hmac.key competition-run-grant.key run-preflight-grant.key moderation-bearer.token; do
        secret_path=$secret_root/$secret_name
        [ -f "$secret_path" ] && [ ! -L "$secret_path" ] &&
            [ "$(stat -c %u -- "$secret_path")" -eq "$(id -u)" ] &&
            [ "$(stat -c %a -- "$secret_path")" = 400 ] &&
            [ "$(stat -c %h -- "$secret_path")" -eq 1 ] || return 1
    done
    [ -d "$raw_root" ] && [ ! -L "$raw_root" ] &&
        [ "$(realpath -e -- "$raw_root")" = "$raw_root" ] &&
        [ "$(stat -c %a -- "$raw_root")" = 550 ] &&
        validate_owned_tree "$raw_root" 0 || return 1
    clean_raw_marks=$(find "$raw_root" -mindepth 1 -maxdepth 1 -printf .) || return 1
    [ "$clean_raw_marks" = .. ] || return 1
    for edition in demo full; do
        edition_root=$raw_root/$edition
        [ -d "$edition_root" ] && [ ! -L "$edition_root" ] &&
            [ "$(stat -c %a -- "$edition_root")" = 550 ] || return 1
        clean_raw_violation=$(find "$edition_root" -mindepth 0 -perm /222 -print -quit) || return 1
        [ -z "$clean_raw_violation" ] || return 1
        clean_raw_file=$(find "$edition_root" -type f -print -quit) || return 1
        [ -n "$clean_raw_file" ] || return 1
        clean_raw_mode_violation=$(find "$edition_root" -type d ! -perm 0550 -print -quit) || return 1
        [ -z "$clean_raw_mode_violation" ] || return 1
        clean_raw_mode_violation=$(find "$edition_root" -type f ! -perm 0440 -print -quit) || return 1
        [ -z "$clean_raw_mode_violation" ] || return 1
    done
    if [ -e "$user_unit_root" ] || [ -L "$user_unit_root" ]; then
        [ -d "$user_unit_root" ] && [ ! -L "$user_unit_root" ] &&
            [ "$(realpath -e -- "$user_unit_root")" = "$user_unit_root" ] &&
            [ "$(stat -c %u -- "$user_unit_root")" -eq "$(id -u)" ] &&
            [ "$(stat -c %a -- "$user_unit_root")" = 750 ] || return 1
    fi
    for unit in robin-highscores.target robin-highscores-api.service robin-highscores-worker.service \
        robin-highscores-backup.service robin-highscores-backup.timer; do
        [ ! -e "$user_unit_root/$unit" ] && [ ! -L "$user_unit_root/$unit" ] || return 1
        for wants_directory in default.target.wants timers.target.wants; do
            [ ! -e "$user_unit_root/$wants_directory/$unit" ] &&
                [ ! -L "$user_unit_root/$wants_directory/$unit" ] || return 1
        done
        clean_active_state=$(systemctl --user show --property=ActiveState --value "$unit" 2>/dev/null || printf not-found)
        case "$clean_active_state" in inactive|not-found) ;; *) return 1 ;; esac
        if clean_enabled_state=$(systemctl --user is-enabled "$unit" 2>/dev/null); then
            case "$clean_enabled_state" in disabled|not-found|static|indirect|masked) ;; *) return 1 ;; esac
        else
            case "$clean_enabled_state" in ''|disabled|not-found|static|indirect|masked) ;; *) return 1 ;; esac
        fi
    done
}

validate_current_absent_recovery_topology() {
    validate_owned_tree "$opt_root" 1 && validate_owned_tree "$incoming_root" 0 &&
        validate_owned_tree "$release_root" 0 || return 1
    recovery_opt_marks=...
    for allowed_path in "$prepared_journal" "$prepared_journal_temporary" \
        "$prebackup_receipt" "$prebackup_receipt_temporary" \
        "$target_backup_receipt" "$target_backup_receipt_temporary" \
        "$authority_journal" "$authority_journal_temporary" \
        "$temporary_link" "$restore_link"; do
        if [ -e "$allowed_path" ] || [ -L "$allowed_path" ]; then
            recovery_opt_marks=$recovery_opt_marks.
        fi
    done
    observed_opt_marks=$(find "$opt_root" -mindepth 1 -maxdepth 1 -printf .) || return 1
    [ "$observed_opt_marks" = "$recovery_opt_marks" ] || return 1

    recovery_release_marks=
    for allowed_path in "$release" "$release_root/$expected_commit.partial"; do
        if [ -e "$allowed_path" ] || [ -L "$allowed_path" ]; then
            recovery_release_marks=$recovery_release_marks.
        fi
    done
    observed_release_marks=$(find "$release_root" -mindepth 1 -maxdepth 1 -printf .) || return 1
    [ "$observed_release_marks" = "$recovery_release_marks" ] || return 1

    recovery_incoming_marks=
    for allowed_path in \
        "$incoming_root/.sources-$expected_commit" \
        "$incoming_root/.sources-$expected_commit.consuming" \
        "$incoming_root/.sources-$expected_commit.consume-v1.json" \
        "$incoming_root/.sources-$expected_commit.consume-v1.json.new" \
        "$incoming_root/.sources-$expected_commit.consume-v1.complete.json" \
        "$incoming_root/.sources-$expected_commit.consume-v1.complete.json.new"
    do
        if [ -e "$allowed_path" ] || [ -L "$allowed_path" ]; then
            recovery_incoming_marks=$recovery_incoming_marks.
        fi
    done
    observed_incoming_marks=$(find "$incoming_root" -mindepth 1 -maxdepth 1 -printf .) || return 1
    [ "$observed_incoming_marks" = "$recovery_incoming_marks" ] || return 1

    recovery_unit_marks=
    for allowed_path in "$unit_stage_root" "$unit_backup_root"; do
        if [ -e "$allowed_path" ] || [ -L "$allowed_path" ]; then
            recovery_unit_marks=$recovery_unit_marks.
        fi
    done
    observed_unit_marks=$(find "$user_unit_root" -mindepth 1 -maxdepth 1 \
        \( -name '.robin-highscores-deploy-*' -o -name '.robin-highscores-rollback-*' \) -printf .) || return 1
    [ "$observed_unit_marks" = "$recovery_unit_marks" ]
}

cleanup() {
    status=$?
    trap - EXIT HUP INT TERM
    cleanup_failed=0
    cleanup_keep_stopped=0
    for cleanup_authority_path in "$authority_journal" "$authority_journal_temporary"; do
        if [ -e "$cleanup_authority_path" ] || [ -L "$cleanup_authority_path" ]; then
            if validate_authority_journal_path "$cleanup_authority_path"; then
                case "$authority_phase" in
                    source_backup_started|source_verified|schema_verified|target_backup_started|target_verified)
                        cleanup_keep_stopped=1
                        ;;
                esac
            else
                cleanup_keep_stopped=1
                cleanup_failed=1
            fi
        fi
    done
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
    if [ "$status" -ne 0 ] && [ "$activation_started" -eq 1 ] && [ "$activation_complete" -eq 0 ]; then
        if [ "$cleanup_keep_stopped" -eq 1 ]; then
            systemctl --user disable robin-highscores.target >/dev/null 2>&1 || cleanup_failed=1
            systemctl --user disable robin-highscores-backup.timer >/dev/null 2>&1 || cleanup_failed=1
            for unit in robin-highscores.target robin-highscores-worker.service robin-highscores-api.service \
                robin-highscores-backup.timer robin-highscores-backup.service; do
                systemctl --user stop "$unit" >/dev/null 2>&1 || cleanup_failed=1
            done
            for wants_directory in "$user_unit_root/default.target.wants" "$user_unit_root/timers.target.wants"; do
                if [ -e "$wants_directory" ] || [ -L "$wants_directory" ]; then
                    [ -d "$wants_directory" ] && [ ! -L "$wants_directory" ] &&
                        sync -f -- "$wants_directory" || cleanup_failed=1
                fi
            done
            preserve_recovery=1
            echo "backup or migration authority is durable; services remain disabled and stopped for authenticated resume" >&2
        elif [ "$migration_started" -eq 0 ] && [ "$transaction_prepared" -eq 1 ]; then
            if ! restore_pre_migration_state; then
                echo "deployment cleanup failed to restore the pre-migration unit/current/runtime state" >&2
                preserve_recovery=1
                cleanup_failed=1
                systemctl --user disable robin-highscores.target >/dev/null 2>&1 || :
                systemctl --user disable robin-highscores-backup.timer >/dev/null 2>&1 || :
                for unit in robin-highscores.target robin-highscores-worker.service robin-highscores-api.service \
                    robin-highscores-backup.timer robin-highscores-backup.service; do
                    systemctl --user stop "$unit" >/dev/null 2>&1 || :
                done
            fi
        elif [ "$migration_completed" -eq 0 ]; then
            systemctl --user disable robin-highscores.target >/dev/null 2>&1 || cleanup_failed=1
            systemctl --user disable robin-highscores-backup.timer >/dev/null 2>&1 || cleanup_failed=1
            for unit in robin-highscores.target robin-highscores-worker.service robin-highscores-api.service \
                robin-highscores-backup.timer robin-highscores-backup.service; do
                systemctl --user stop "$unit" >/dev/null 2>&1 || cleanup_failed=1
            done
            preserve_recovery=1
            echo "migration failed; the prior release and units remain selected but stopped because schema compatibility is uncertain" >&2
        else
            converge_new_selected_stopped || {
                echo "deployment could not converge the migrated release to a coherent selected/stopped state" >&2
                preserve_recovery=1
                cleanup_failed=1
            }
            if [ "$preserve_recovery" -eq 0 ]; then
                preserve_recovery=1
                echo "migration completed; the new release is selected but stopped after activation failure" >&2
            fi
        fi
    fi
    if [ "$prepared_journal_owned" -eq 1 ]; then
        cleanup_journal_source=$(sed -n 's/^source_commit=//p' "$prepared_journal" 2>/dev/null || printf invalid)
        if validate_prepared_journal "$cleanup_journal_source" && [ "$journal_phase" = preparing ]; then
            preserve_recovery=1
        fi
    fi
    if [ -e "$authority_journal" ] || [ -L "$authority_journal" ]; then
        validate_authority_journal || cleanup_failed=1
        preserve_recovery=1
    fi
    if [ "$temporary_link_owned" -eq 1 ] && [ "$preserve_recovery" -eq 0 ]; then
        remove_tracked_link "$temporary_link" "releases/$expected_commit" || cleanup_failed=1
    fi
    if [ "$restore_link_owned" -eq 1 ] && [ "$preserve_recovery" -eq 0 ]; then
        remove_tracked_link "$restore_link" "$recovery_previous_target" || cleanup_failed=1
    fi
    if [ "$prepared_journal_owned" -eq 1 ] && [ "$preserve_recovery" -eq 0 ]; then
        cleanup_journal_source=$(sed -n 's/^source_commit=//p' "$prepared_journal" 2>/dev/null || printf invalid)
        validate_prepared_journal "$cleanup_journal_source" || cleanup_failed=1
        if [ "$cleanup_failed" -eq 0 ]; then
            if rm -f -- "$prepared_journal" && sync -f -- "$opt_root"; then
                prepared_journal_owned=0
            else
                cleanup_failed=1
            fi
        fi
    fi
    if [ "$prepared_journal_owned" -eq 1 ]; then
        preserve_recovery=1
    fi
    if [ "$unit_stage_owned" -eq 1 ] && [ "$preserve_recovery" -eq 0 ]; then
        remove_owned_tree "$unit_stage_root" "$user_unit_root" || cleanup_failed=1
    fi
    if [ "$unit_backup_owned" -eq 1 ] && [ "$preserve_recovery" -eq 0 ]; then
        remove_owned_tree "$unit_backup_root" "$user_unit_root" || cleanup_failed=1
    fi
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
    if [ "$authority_journal_temporary_owned" -eq 1 ]; then
        [ -f "$authority_journal_temporary" ] && [ ! -L "$authority_journal_temporary" ] &&
            [ "$(stat -c %u -- "$authority_journal_temporary")" -eq "$(id -u)" ] &&
            [ "$(stat -c %h -- "$authority_journal_temporary")" -eq 1 ] &&
            rm -f -- "$authority_journal_temporary" && sync -f -- "$opt_root" || cleanup_failed=1
    fi
    if [ "$prepared_journal_temporary_owned" -eq 1 ]; then
        [ -f "$prepared_journal_temporary" ] && [ ! -L "$prepared_journal_temporary" ] &&
            [ "$(stat -c %u -- "$prepared_journal_temporary")" -eq "$(id -u)" ] &&
            [ "$(stat -c %h -- "$prepared_journal_temporary")" -eq 1 ] &&
            rm -f -- "$prepared_journal_temporary" && sync -f -- "$opt_root" || cleanup_failed=1
    fi
    if [ "$cleanup_failed" -ne 0 ]; then
        echo "deployment cleanup failed; inspect exact private temporary paths before retry" >&2
        if [ "$preserve_recovery" -eq 1 ]; then
            echo "preserved recovery artifacts: $unit_backup_root $unit_stage_root $restore_link" >&2
        fi
        [ "$status" -ne 0 ] || status=1
    fi
    exit "$status"
}
trap cleanup EXIT
trap 'trap - HUP; exit 129' HUP
trap 'trap - INT; exit 130' INT
trap 'trap - TERM; exit 143' TERM

systemctl --user show-environment >/dev/null 2>&1 || fail "systemd user manager is unavailable"
if [ -e "$current_link" ] || [ -L "$current_link" ]; then
    [ -L "$current_link" ] || fail "current selector exists but is not a symlink"
    previous_target=$(readlink -- "$current_link")
    case "$previous_target" in
        releases/[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f])
            previous_commit=${previous_target#releases/}
            [ -d "$release_root/$previous_commit" ] && [ ! -L "$release_root/$previous_commit" ] ||
                fail "current selector names a missing or linked release"
            ;;
        *) fail "current selector has an unsafe target" ;;
    esac
else
    clean_recovery_authorized=0
    if [ -f "$prepared_journal" ] && [ ! -L "$prepared_journal" ]; then
        clean_recovery_source=$(sed -n 's/^source_commit=//p' "$prepared_journal")
        [ "$clean_recovery_source" = none ] ||
            fail "current is absent but the durable recovery journal claims a source release"
        validate_prepared_journal "$clean_recovery_source" ||
            fail "current is absent and its durable recovery journal is not exact"
        validate_current_absent_recovery_topology ||
            fail "current is absent and foreign or ambiguous transaction residue exists"
        clean_recovery_authorized=1
    elif [ -e "$prepared_journal" ] || [ -L "$prepared_journal" ]; then
        fail "current is absent and its recovery journal path is unsafe"
    fi
    if [ "$clean_recovery_authorized" -eq 0 ]; then
        validate_clean_first_deploy_state ||
            fail "current is absent but the host is not an exact clean first-deploy state"
    fi
fi

ensure_candidate_root_authority || fail "candidate path changed before transaction validation"
validate_owned_tree "$candidate_root_fd/." 0 || fail "retained candidate has unsafe ownership/topology"
[ -d "$release_root" ] && [ ! -L "$release_root" ] &&
    [ "$(realpath -e -- "$release_root")" = "$release_root" ] &&
    [ "$(stat -c %u -- "$release_root")" -eq "$(id -u)" ] &&
    [ "$(stat -c %a -- "$release_root")" = 750 ] ||
    fail "pre-provisioned release root must be canonical, owner-controlled, and mode 0750"
/usr/bin/flock -n "$activation_lock_number" ||
    fail "another deploy or rollback transaction holds the inherited activation lock"
[ "$(stat -Lc %d:%i -- "$activation_lock_fd")" = "$activation_lock_identity" ] ||
    fail "inherited activation lock descriptor changed"
if [ "$deploy_mode" = candidate ]; then
    [ ! -e "$release" ] && [ ! -L "$release" ] || fail "release already exists; use explicit --resume-installed only after authenticating the exact final"
    ensure_candidate_root_authority || fail "release candidate differs from its retained descriptor"
    candidate_tree=$candidate_root_fd
else
    [ -d "$release" ] && [ ! -L "$release" ] && [ "$(stat -c %a -- "$release")" = 550 ] ||
        fail "resume target is not an immutable mode-0550 release"
    ensure_candidate_root_authority || fail "installed resume target differs from its retained descriptor"
    validate_release_descriptor_tree "$candidate_root_fd/." || fail "trusted descriptor validation rejected the installed release"
    release_installed=1
    candidate_tree=$candidate_root_fd
fi
for managed_directory in "$state_root" "$secret_root"; do
    [ -d "$managed_directory" ] && [ ! -L "$managed_directory" ] &&
        [ "$(realpath -e -- "$managed_directory")" = "$managed_directory" ] &&
        [ "$(stat -c %u -- "$managed_directory")" -eq "$(id -u)" ] &&
        [ "$(stat -c %g -- "$managed_directory")" -eq "$(id -g)" ] &&
        [ "$(stat -c %a -- "$managed_directory")" = 700 ] ||
        fail "managed state directory is not exact and will not be repaired: $managed_directory"
done
database_directory_modes=700
replay_directory_modes=700
campaign_directory_modes=700
if [ -n "$previous_commit" ]; then
    database_directory_modes=2770
    replay_directory_modes=2770
    campaign_directory_modes=2770
elif [ "$deploy_mode" = resume ] && { [ -e "$authority_journal" ] || [ -L "$authority_journal" ]; }; then
    [ "$clean_recovery_authorized" -eq 1 ] ||
        fail "runtime-directory admission is not bound to current-absent recovery"
    mode_journal_source=$clean_recovery_source
    validate_prepared_journal "$mode_journal_source" && [ "$journal_phase" = prepared ] ||
        fail "runtime-directory admission lacks an exact durable prepared journal"
    validate_authority_journal ||
        fail "runtime-directory admission lacks an exact durable authority journal"
    [ "$authority_source" = "$mode_journal_source" ] ||
        fail "runtime-directory authority differs from the prepared transaction"
    case "$authority_phase" in
        prepared)
            ;;
        runtime_ready)
            database_directory_modes='700 2770'
            ;;
        schema_verified)
            [ "$authority_schema" = verified ] ||
                fail "post-migration runtime-directory authority lacks a schema proof"
            database_directory_modes=2770
            replay_directory_modes='700 2770'
            campaign_directory_modes='700 2770'
            ;;
        target_backup_started|target_verified)
            [ "$authority_schema" = verified ] ||
                fail "post-migration runtime-directory authority lacks a schema proof"
            database_directory_modes=2770
            replay_directory_modes=2770
            campaign_directory_modes=2770
            ;;
        *)
            fail "runtime-directory authority has no exact mode contract"
            ;;
    esac
fi
for managed_directory_and_modes in \
    "$state_root/database:$database_directory_modes" \
    "$state_root/replays:$replay_directory_modes" \
    "$state_root/campaign-states:$campaign_directory_modes"
do
    managed_directory=${managed_directory_and_modes%%:*}
    managed_modes=${managed_directory_and_modes#*:}
    managed_mode=$(stat -c %a -- "$managed_directory" 2>/dev/null || printf invalid)
    case " $managed_modes " in
        *" $managed_mode "*) ;;
        *) fail "managed runtime directory has the wrong mode for the authenticated transaction phase: $managed_directory" ;;
    esac
    [ -d "$managed_directory" ] && [ ! -L "$managed_directory" ] &&
        [ "$(realpath -e -- "$managed_directory")" = "$managed_directory" ] &&
        [ "$(stat -c %u -- "$managed_directory")" -eq "$(id -u)" ] &&
        [ "$(stat -c %g -- "$managed_directory")" -eq "$(id -g)" ] ||
        fail "managed runtime directory is not exact for the authenticated transaction phase: $managed_directory"
done
for managed_directory in "$opt_root" "$release_root"; do
    [ -d "$managed_directory" ] && [ ! -L "$managed_directory" ] &&
        [ "$(realpath -e -- "$managed_directory")" = "$managed_directory" ] &&
        [ "$(stat -c %u -- "$managed_directory")" -eq "$(id -u)" ] &&
        [ "$(stat -c %a -- "$managed_directory")" = 750 ] ||
        fail "managed install directory is not exact and will not be repaired: $managed_directory"
done
if [ -e "$user_unit_root" ] || [ -L "$user_unit_root" ]; then
    [ -d "$user_unit_root" ] && [ ! -L "$user_unit_root" ] &&
        [ "$(realpath -e -- "$user_unit_root")" = "$user_unit_root" ] &&
        [ "$(stat -c %u -- "$user_unit_root")" -eq "$(id -u)" ] &&
        [ "$(stat -c %a -- "$user_unit_root")" = 750 ] ||
        fail "managed user-unit root is not exact and will not be repaired"
elif [ -n "$previous_commit" ]; then
    fail "managed user-unit root is missing on upgrade and will not be repaired"
fi
for managed_directory in "$backup_root" "$state_root/status"; do
    if [ -e "$managed_directory" ] || [ -L "$managed_directory" ]; then
        [ -d "$managed_directory" ] && [ ! -L "$managed_directory" ] &&
            [ "$(realpath -e -- "$managed_directory")" = "$managed_directory" ] &&
            [ "$(stat -c %u -- "$managed_directory")" -eq "$(id -u)" ] &&
            [ "$(stat -c %a -- "$managed_directory")" = 700 ] ||
            fail "backup state directory is not exact and will not be repaired: $managed_directory"
    elif [ -n "$previous_commit" ]; then
        fail "backup state directory is missing on upgrade and will not be repaired: $managed_directory"
    fi
done

for secret_name in cursor-hmac.key competition-run-grant.key run-preflight-grant.key moderation-bearer.token; do
    secret_path=$secret_root/$secret_name
    [ -f "$secret_path" ] && [ ! -L "$secret_path" ] || fail "required VPS-generated secret is missing: $secret_path"
    [ "$(stat -c %U "$secret_path")" = "$expected_user" ] &&
        [ "$(stat -c %a "$secret_path")" = 400 ] && [ "$(stat -c %h "$secret_path")" -eq 1 ] ||
        fail "secret has unsafe ownership, mode, or link count: $secret_path"
done

[ -d "$raw_root" ] && [ ! -L "$raw_root" ] || fail "raw-content parent is missing or symlinked"
[ "$(realpath -e -- "$raw_root")" = "$raw_root" ] || fail "raw-content parent has a symlinked ancestor"
[ "$(stat -c %a -- "$raw_root")" = 550 ] || fail "raw-content parent does not have exact mode 0550"
validate_owned_tree "$raw_root" 0 || fail "raw-content authority crosses mounts/devices or has unsafe topology"
for edition in demo full; do
    edition_root=$raw_root/$edition
    [ -d "$edition_root" ] && [ ! -L "$edition_root" ] || fail "manually installed read-only raw root is missing: $edition_root"
    [ "$(stat -c %U "$edition_root")" = "$expected_user" ] || fail "raw root has the wrong owner: $edition_root"
    [ "$(stat -c %a -- "$edition_root")" = 550 ] || fail "raw edition root does not have exact mode 0550: $edition_root"
    raw_violation=$(find "$edition_root" -mindepth 1 ! -type d ! -type f -print -quit) || fail "could not scan raw-root topology: $edition_root"
    [ -z "$raw_violation" ] || fail "raw root contains a link or special file: $edition_root"
    raw_violation=$(find "$edition_root" -type f -links +1 -print -quit) || fail "could not scan raw-root links: $edition_root"
    [ -z "$raw_violation" ] || fail "raw root contains a hard-linked file: $edition_root"
    raw_violation=$(find "$edition_root" -mindepth 0 -perm /222 -print -quit) || fail "could not scan raw-root modes: $edition_root"
    [ -z "$raw_violation" ] || fail "raw root is writable: $edition_root"
    raw_violation=$(find "$edition_root" -type d ! -perm 0550 -print -quit) || fail "could not scan raw directory modes: $edition_root"
    [ -z "$raw_violation" ] || fail "raw root contains a directory without exact mode 0550: $edition_root"
    raw_violation=$(find "$edition_root" -type f ! -perm 0440 -print -quit) || fail "could not scan raw file modes: $edition_root"
    [ -z "$raw_violation" ] || fail "raw root contains a file without exact mode 0440: $edition_root"
    raw_violation=$(find "$edition_root" -mindepth 0 ! -user "$expected_user" -print -quit) || fail "could not scan raw-root ownership: $edition_root"
    [ -z "$raw_violation" ] || fail "raw root contains mixed ownership: $edition_root"
    raw_file=$(find "$edition_root" -type f -print -quit) || fail "could not scan raw-root files: $edition_root"
    [ -n "$raw_file" ] || fail "raw root contains no regular files: $edition_root"
done

if [ "$deploy_mode" = candidate ]; then
    /usr/bin/env -i \
        HOME="$expected_home" USER="$expected_user" LOGNAME="$expected_user" \
        PATH=/usr/bin:/bin PYTHONDONTWRITEBYTECODE=1 \
        ROBIN_REAL_FENCE_PINNED_CANDIDATE_FD="${candidate_root_fd#/proc/self/fd/}" \
        "$candidate_root_fd/deploy/tests/real-runtime-fence-release-gate.sh" \
        "$bundle_arg" "$expected_commit" "$expected_sums_sha256" \
        "$raw_root/demo" "$raw_root/full" ||
        fail "mandatory authentic runtime-fence release gate failed before activation mutation"
    ensure_candidate_root_authority ||
        fail "candidate changed across the mandatory authentic runtime-fence release gate"
fi

if [ "$deploy_mode" = resume ] && [ "$previous_commit" = "$expected_commit" ]; then
    resume_post_migration=1
fi
for unit in \
    robin-highscores.target \
    robin-highscores-api.service \
    robin-highscores-worker.service \
    robin-highscores-backup.service \
    robin-highscores-backup.timer
do
    destination=$user_unit_root/$unit
    if [ "$deploy_mode" = resume ]; then
        if [ -e "$destination" ] || [ -L "$destination" ]; then
            [ -f "$destination" ] && [ ! -L "$destination" ] &&
                [ "$(stat -c %u -- "$destination")" -eq "$(id -u)" ] &&
                [ "$(stat -c %h -- "$destination")" -eq 1 ] &&
                [ "$(stat -c %a -- "$destination")" = 440 ] ||
                fail "resume found unsafe managed unit metadata: $unit"
            target_unit_match=0
            old_unit_match=0
            if cmp -s -- "$destination" "$release/systemd/user/$unit"; then
                target_unit_match=1
            fi
            if [ -n "$previous_commit" ] &&
                cmp -s -- "$destination" "$release_root/$previous_commit/systemd/user/$unit"; then
                old_unit_match=1
            fi
            if [ "$target_unit_match" -eq 1 ] && [ "$old_unit_match" -eq 0 ]; then
                resume_post_migration=1
            elif [ "$target_unit_match" -eq 1 ] || [ "$old_unit_match" -eq 1 ]; then
                :
            else
                fail "resume found a unit that is neither exact old nor exact target: $unit"
            fi
        elif [ -n "$previous_commit" ]; then
            fail "resume is missing a managed unit from the selected old release: $unit"
        fi
        resume_active_state=$(systemctl --user show --property=ActiveState --value "$unit" 2>/dev/null || printf 'not-found')
        case "$resume_active_state" in active|inactive|failed|not-found) ;; *) fail "resume found a transitional managed unit: $unit" ;; esac
    elif [ -n "$previous_commit" ]; then
        [ -f "$destination" ] && [ ! -L "$destination" ] &&
            [ "$(stat -c %u -- "$destination")" -eq "$(id -u)" ] &&
            [ "$(stat -c %h -- "$destination")" -eq 1 ] &&
            [ "$(stat -c %a -- "$destination")" = 440 ] &&
            cmp -s -- "$destination" "$release_root/$previous_commit/systemd/user/$unit" ||
            fail "installed unit does not match the selected release: $unit"
    else
        [ ! -e "$destination" ] && [ ! -L "$destination" ] ||
            fail "managed unit exists without a current selector: $unit"
    fi
done
if [ "$deploy_mode" = resume ] && [ "$previous_commit" = "$expected_commit" ] &&
    [ ! -e "$prepared_journal" ] && [ ! -L "$prepared_journal" ] &&
    [ ! -e "$prepared_journal_temporary" ] && [ ! -L "$prepared_journal_temporary" ] &&
    [ ! -e "$authority_journal" ] && [ ! -L "$authority_journal" ] &&
    [ ! -e "$authority_journal_temporary" ] && [ ! -L "$authority_journal_temporary" ] &&
    [ ! -e "$prebackup_receipt" ] && [ ! -L "$prebackup_receipt" ] &&
    [ ! -e "$target_backup_receipt" ] && [ ! -L "$target_backup_receipt" ] &&
    [ ! -e "$unit_stage_root" ] && [ ! -L "$unit_stage_root" ] &&
    [ ! -e "$unit_backup_root" ] && [ ! -L "$unit_backup_root" ]; then
    runtime_authority_probe present || fail "active target runtime authority is no longer exact"
    verify_live_schema "$candidate_root_fd/bin/robin-highscores-admin" \
        "$candidate_root_fd" "$expected_vps_manifest_sha256" ||
        fail "active target no longer attests the live database schema"
    active_publication_sha=$(release_publication_lock_sha256 \
        "$candidate_root_fd" "$expected_vps_manifest_sha256") ||
        fail "active target publication identity is no longer exact"
    transaction_backup_receipt "$candidate_root_fd/bin/robin-highscores-admin" \
        "$candidate_root_fd" "$expected_commit" "$expected_vps_manifest_sha256" \
        "$active_publication_sha" >/dev/null ||
        fail "active target no longer has an authenticated exact BackupV4"
    consume_vps_sources || fail "active target uploader-source cleanup did not converge"
    for unit in robin-highscores.target robin-highscores-api.service robin-highscores-worker.service \
        robin-highscores-backup.service robin-highscores-backup.timer; do
        [ -f "$user_unit_root/$unit" ] && [ ! -L "$user_unit_root/$unit" ] &&
            [ "$(stat -c %u -- "$user_unit_root/$unit")" -eq "$(id -u)" ] &&
            [ "$(stat -c %h -- "$user_unit_root/$unit")" -eq 1 ] &&
            [ "$(stat -c %a -- "$user_unit_root/$unit")" = 440 ] &&
            cmp -s -- "$user_unit_root/$unit" "$candidate_root_fd/systemd/user/$unit" ||
            fail "active target unit is not exact: $unit"
    done
    for unit in robin-highscores.target robin-highscores-api.service robin-highscores-worker.service \
        robin-highscores-backup.timer; do
        [ "$(systemctl --user show --property=ActiveState --value "$unit")" = active ] ||
            fail "active target unit is not active: $unit"
    done
    [ "$(systemctl --user show --property=ActiveState --value robin-highscores-backup.service)" = inactive ] ||
        fail "active target backup service is not inactive"
    [ "$(systemctl --user is-enabled robin-highscores.target)" = enabled ] &&
        [ "$(systemctl --user is-enabled robin-highscores-backup.timer)" = enabled ] ||
        fail "active target enablement is not durable"
    probe http://127.0.0.1:8787/healthz 5 && probe http://127.0.0.1:8787/readyz 5 ||
        fail "active target failed final health/readiness validation"
    activation_complete=1
    echo "release $expected_commit is already active, verified, and ready"
    exit 0
fi
if [ -n "$previous_commit" ]; then
    backup_active_state=$(systemctl --user show --property=ActiveState --value robin-highscores-backup.service) ||
        fail "could not inspect the verified backup service"
    case "$backup_active_state" in
        inactive|failed) ;;
        *) fail "verified backup is running or transitional; wait for it to finish before deploying" ;;
    esac
fi

reconcile_prepared_journal_temporary ||
    fail "could not reconcile the exact interrupted deploy journal publication"
if [ -f "$prepared_journal" ] && [ ! -L "$prepared_journal" ]; then
    interrupted_source_commit=$(sed -n 's/^source_commit=//p' "$prepared_journal")
    validate_prepared_journal "$interrupted_source_commit" ||
        fail "durable deploy journal is not exact"
    if [ "$journal_phase" = preparing ]; then
        abort_interrupted_preparation "$interrupted_source_commit" ||
            fail "could not safely reconcile the interrupted preactivation preparation"
    fi
fi
reuse_recovery_snapshot=0
if [ -f "$prepared_journal" ] && [ ! -L "$prepared_journal" ]; then
    receipt_source_commit=$(sed -n 's/^source_commit=//p' "$prepared_journal")
    case "$receipt_source_commit" in
        none) ;;
        *[!0-9a-f]*|'') fail "durable deploy journal has invalid source identity" ;;
        *) [ "${#receipt_source_commit}" -eq 40 ] || fail "durable deploy journal has invalid source identity" ;;
    esac
    validate_prepared_journal "$receipt_source_commit" ||
        fail "durable deploy prepared journal and recovery snapshot are not exact"
    if [ "$journal_phase" = activation_complete ]; then
        [ "$deploy_mode" = resume ] || fail "activation-complete recovery requires --resume-installed"
        prepared_journal_owned=1
        activation_complete=1
        preserve_recovery=1
        finish_terminal_cleanup "$receipt_source_commit" ||
            fail "could not finish idempotent cleanup for the active target release"
        preserve_recovery=0
        echo "release $expected_commit is active and ready; interrupted terminal cleanup completed"
        exit 0
    fi
    [ -d "$unit_stage_root" ] && [ ! -L "$unit_stage_root" ] &&
        [ -d "$unit_backup_root" ] && [ ! -L "$unit_backup_root" ] ||
        fail "durable deploy prepared journal is missing its exact recovery trees"
    validate_owned_tree "$unit_stage_root" 0 && [ "$(stat -c %a -- "$unit_stage_root")" = 700 ] ||
        fail "durable deploy target staging is unsafe"
    for unit in robin-highscores.target robin-highscores-api.service robin-highscores-worker.service \
        robin-highscores-backup.service robin-highscores-backup.timer; do
        if [ -e "$unit_stage_root/units/$unit" ] || [ -L "$unit_stage_root/units/$unit" ]; then
            [ -f "$unit_stage_root/units/$unit" ] && [ ! -L "$unit_stage_root/units/$unit" ] &&
                [ "$(stat -c %u -- "$unit_stage_root/units/$unit")" -eq "$(id -u)" ] &&
                [ "$(stat -c %h -- "$unit_stage_root/units/$unit")" -eq 1 ] &&
                [ "$(stat -c %a -- "$unit_stage_root/units/$unit")" = 440 ] &&
                cmp -s -- "$unit_stage_root/units/$unit" "$candidate_tree/systemd/user/$unit" ||
                fail "durable deploy target unit staging changed: $unit"
        else
            [ -f "$user_unit_root/$unit" ] && [ ! -L "$user_unit_root/$unit" ] &&
                [ "$(stat -c %u -- "$user_unit_root/$unit")" -eq "$(id -u)" ] &&
                [ "$(stat -c %h -- "$user_unit_root/$unit")" -eq 1 ] &&
                [ "$(stat -c %a -- "$user_unit_root/$unit")" = 440 ] &&
                cmp -s -- "$user_unit_root/$unit" "$candidate_tree/systemd/user/$unit" ||
                fail "consumed durable target stage is not installed exactly: $unit"
            install -m 0440 -- "$candidate_tree/systemd/user/$unit" "$unit_stage_root/units/$unit" ||
                fail "could not reconstruct authenticated target unit staging: $unit"
            sync -f -- "$unit_stage_root/units/$unit" || fail "reconstructed target unit was not durable: $unit"
        fi
        if [ "$receipt_source_commit" = none ]; then
            [ -f "$unit_backup_root/absent/$unit" ] && [ ! -L "$unit_backup_root/absent/$unit" ] ||
                fail "durable first-deploy absence marker changed: $unit"
        else
            [ -f "$unit_backup_root/units/$unit" ] && [ ! -L "$unit_backup_root/units/$unit" ] &&
                [ "$(stat -c %u -- "$unit_backup_root/units/$unit")" -eq "$(id -u)" ] &&
                [ "$(stat -c %h -- "$unit_backup_root/units/$unit")" -eq 1 ] &&
                [ "$(stat -c %a -- "$unit_backup_root/units/$unit")" = 440 ] &&
                cmp -s -- "$unit_backup_root/units/$unit" "$release_root/$receipt_source_commit/systemd/user/$unit" ||
                fail "durable deploy source recovery unit changed: $unit"
        fi
    done
    unit_stage_owned=1
    unit_backup_owned=1
    prepared_journal_owned=1
    reuse_recovery_snapshot=1
    resume_with_recovery=1
fi
if [ "$reuse_recovery_snapshot" -eq 0 ]; then
    if [ -n "$previous_commit" ]; then preparation_source_commit=$previous_commit; else preparation_source_commit=none; fi
    write_preparation_intent "$preparation_source_commit" ||
        fail "could not make the preparation intent durable before recovery mutation"
    if [ ! -e "$user_unit_root" ] && [ ! -L "$user_unit_root" ]; then
        [ -z "$previous_commit" ] || fail "upgrade may not repair a missing user-unit root"
        install -d -m 0750 -- "$user_unit_root" || fail "could not create the journal-authorized user-unit root"
        [ -d "$user_unit_root" ] && [ ! -L "$user_unit_root" ] &&
            [ "$(realpath -e -- "$user_unit_root")" = "$user_unit_root" ] &&
            [ "$(stat -c %u -- "$user_unit_root")" -eq "$(id -u)" ] &&
            [ "$(stat -c %a -- "$user_unit_root")" = 750 ] ||
            fail "journal-authorized user-unit root is not exact"
        sync -f -- "$expected_home/.config/systemd" || fail "could not make the user-unit root durable"
    fi
    [ ! -e "$unit_stage_root" ] && [ ! -L "$unit_stage_root" ] || fail "unit staging path already exists"
    [ ! -e "$unit_backup_root" ] && [ ! -L "$unit_backup_root" ] || fail "unit backup path already exists"
    if mkdir -m 0700 -- "$unit_stage_root"; then unit_stage_owned=1; else fail "could not create unit staging"; fi
    mkdir -m 0700 -- "$unit_stage_root/units"
    if mkdir -m 0700 -- "$unit_backup_root"; then unit_backup_owned=1; else fail "could not create unit backup"; fi
    mkdir -m 0700 -- "$unit_backup_root/units" "$unit_backup_root/absent" \
        "$unit_backup_root/active" "$unit_backup_root/enabled"
    for unit in robin-highscores.target robin-highscores-api.service robin-highscores-worker.service \
        robin-highscores-backup.service robin-highscores-backup.timer; do
        destination=$user_unit_root/$unit
        install -m 0440 -- "$candidate_tree/systemd/user/$unit" "$unit_stage_root/units/$unit"
        sync -f -- "$unit_stage_root/units/$unit"
        if [ -n "$previous_commit" ]; then
            cp --archive -- "$destination" "$unit_backup_root/units/$unit"
            sync -f -- "$unit_backup_root/units/$unit"
            unit_active_state=$(systemctl --user show --property=ActiveState --value "$unit") || fail "could not capture active state for $unit"
            case "$unit_active_state" in
                active)
                    : >"$unit_backup_root/active/$unit"
                    sync -f -- "$unit_backup_root/active/$unit"
                    ;;
                inactive) ;;
                *) fail "unit is in a transitional state: $unit ($unit_active_state)" ;;
            esac
        else
            : >"$unit_backup_root/absent/$unit"
            sync -f -- "$unit_backup_root/absent/$unit"
            if unit_active_state=$(systemctl --user show --property=ActiveState --value "$unit" 2>/dev/null); then
                case "$unit_active_state" in inactive) ;; *) fail "unit is loaded, failed, or active without a selector: $unit" ;; esac
            fi
        fi
    done
fi
[ ! -f "$unit_backup_root/active/robin-highscores-backup.service" ] ||
    fail "verified backup is already running; wait for it to finish before deploying"
[ ! -f "$unit_backup_root/active/robin-highscores.target" ] ||
    [ -f "$unit_backup_root/active/robin-highscores-api.service" ] ||
    fail "active target has an inactive API"
[ ! -f "$unit_backup_root/active/robin-highscores-worker.service" ] ||
    [ -f "$unit_backup_root/active/robin-highscores-api.service" ] ||
    fail "active worker has an inactive API"
if [ "$reuse_recovery_snapshot" -eq 0 ]; then
for unit in robin-highscores.target robin-highscores-backup.timer; do
    if enabled_state=$(systemctl --user is-enabled "$unit" 2>/dev/null); then
        case "$enabled_state" in
            enabled|enabled-runtime)
                [ -n "$previous_commit" ] || fail "unit is enabled without a current selector: $unit"
                printf '%s\n' "$enabled_state" >"$unit_backup_root/enabled/$unit"
                sync -f -- "$unit_backup_root/enabled/$unit"
                ;;
            *) fail "unexpected enabled state for $unit: $enabled_state" ;;
        esac
    else
        case "$enabled_state" in
            '')
                [ -z "$previous_commit" ] &&
                    [ -f "$unit_backup_root/absent/$unit" ] &&
                    [ ! -L "$unit_backup_root/absent/$unit" ] ||
                    fail "could not prove an empty failed enabled-state query belongs to an absent unit: $unit"
                ;;
            disabled|static|indirect|masked|not-found) ;;
            *) fail "could not capture enabled state for $unit: $enabled_state" ;;
        esac
    fi
done
fi
if [ -L "$temporary_link" ] && [ "$(stat -c %u -- "$temporary_link")" -eq "$(id -u)" ] &&
    [ "$(readlink -- "$temporary_link")" = "releases/$expected_commit" ]; then
    [ "$reuse_recovery_snapshot" -eq 1 ] || fail "unexpected preexisting deploy selector staging"
    temporary_link_owned=1
elif [ ! -e "$temporary_link" ] && [ ! -L "$temporary_link" ]; then
    if ln -s -- "releases/$expected_commit" "$temporary_link"; then temporary_link_owned=1; else fail "could not create temporary selector"; fi
else
    fail "unsafe deploy selector staging"
fi
if [ "$reuse_recovery_snapshot" -eq 1 ] && [ "$receipt_source_commit" != none ]; then
    recovery_previous_target=releases/$receipt_source_commit
else
    recovery_previous_target=$previous_target
fi
if [ -n "$recovery_previous_target" ]; then
    if [ -L "$restore_link" ] && [ "$(stat -c %u -- "$restore_link")" -eq "$(id -u)" ] &&
        [ "$(readlink -- "$restore_link")" = "$recovery_previous_target" ]; then
        [ "$reuse_recovery_snapshot" -eq 1 ] || fail "unexpected preexisting deploy restore selector"
        restore_link_owned=1
    elif [ ! -e "$restore_link" ] && [ ! -L "$restore_link" ]; then
        if ln -s -- "$recovery_previous_target" "$restore_link"; then restore_link_owned=1; else fail "could not create restore selector"; fi
    else
        fail "unsafe deploy restore selector"
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
    if [ -n "$previous_commit" ]; then journal_source_commit=$previous_commit; else journal_source_commit=none; fi
    write_prepared_journal "$journal_source_commit" ||
        fail "could not make the exact prepared activation journal durable"
    receipt_source_commit=$journal_source_commit
fi
if [ "$deploy_mode" = resume ]; then
    validate_prepared_journal "$receipt_source_commit" ||
        fail "could not authenticate the deploy journal before uploader-source cleanup"
    preserve_recovery=1
    consume_vps_sources ||
        fail "descriptor-pinned uploader source cleanup could not complete during resume"
    preserve_recovery=0
fi

if [ "$deploy_mode" = candidate ]; then
    ensure_candidate_root_authority || fail "candidate path changed before release promotion"
    promoted_manifest=$("$manifest_tool_fd" promote-inherited-vps-release-v2 \
        "$expected_vps_manifest_sha256" \
        --candidate-root-fd "${candidate_root_fd#/proc/self/fd/}" \
        --activation-lock-fd "$activation_lock_number") ||
        fail "descriptor-pinned no-replace release promotion failed"
    [ "$promoted_manifest" = "$expected_vps_manifest_sha256" ] ||
        fail "descriptor-pinned promoter returned the wrong V2 manifest digest"
    [ -d "$release" ] && [ ! -L "$release" ] &&
        [ "$(stat -c %d:%i -- "$release")" = "$bundle_device:$bundle_inode" ] ||
        fail "installed release identity differs from the retained release candidate"
    ensure_candidate_root_authority || fail "candidate descriptor did not survive release promotion"
    release_installed=1
    preserve_recovery=1
    validate_prepared_journal "$journal_source_commit" ||
        fail "could not authenticate the prepared journal before candidate consumption"
    consume_vps_sources ||
        fail "descriptor-pinned uploader source cleanup could not complete after promotion"
    preserve_recovery=0
fi

reconcile_authority_journal_temporary ||
    fail "interrupted authority-journal publication is not an exact legal successor"
if [ -e "$authority_journal" ] || [ -L "$authority_journal" ]; then
    validate_authority_journal || fail "durable deployment authority journal is not exact"
    [ "$authority_source" = "$receipt_source_commit" ] ||
        fail "deployment authority journal names the wrong source release"
else
    publish_authority_journal "$receipt_source_commit" unobserved none none none none prepared ||
        fail "could not durably publish the initial deployment authority journal"
fi

if [ "$authority_runtime" != present ]; then
    publish_authority_journal "$receipt_source_commit" initializing \
        "$authority_source_receipt" "$authority_schema" "$authority_target_receipt" \
        "$authority_backup_status_before" "$authority_phase" ||
        fail "could not durably authorize runtime authority initialization"
    initialize_or_adopt_runtime_authority "$receipt_source_commit" ||
        fail "runtime authority could not be initialized or adopted exactly"
    publish_authority_journal "$receipt_source_commit" present \
        "$authority_source_receipt" "$authority_schema" "$authority_target_receipt" \
        "$authority_backup_status_before" runtime_ready ||
        fail "could not durably bind the initialized runtime authority"
else
    runtime_authority_probe present || fail "runtime authority changed after journal publication"
fi

if [ "$receipt_source_commit" = none ]; then
    for initialized_directory in "$backup_root" "$state_root/status"; do
        if [ ! -e "$initialized_directory" ] && [ ! -L "$initialized_directory" ]; then
            mkdir -m 0700 -- "$initialized_directory" ||
                fail "could not create journal-authorized backup state: $initialized_directory"
            sync -f -- "$initialized_directory" ||
                fail "could not make journal-authorized backup state durable: $initialized_directory"
        fi
        [ -d "$initialized_directory" ] && [ ! -L "$initialized_directory" ] &&
            [ "$(realpath -e -- "$initialized_directory")" = "$initialized_directory" ] &&
            [ "$(stat -c %u -- "$initialized_directory")" -eq "$(id -u)" ] &&
            [ "$(stat -c %a -- "$initialized_directory")" = 700 ] ||
            fail "journal-authorized backup state is not exact: $initialized_directory"
    done
    sync -f -- "$state_root" || fail "could not make backup state parents durable"
fi

reuse_prebackup_receipt=0
resume_source_backup=0
if [ "$receipt_source_commit" != none ]; then
    source_release=$release_root/$receipt_source_commit
    source_admin=$source_release/bin/robin-highscores-admin
    source_vps_sha=$(sha256sum "$source_release/vps-release-manifest-v2.json" | cut -d' ' -f1) ||
        fail "could not hash the installed source V2 manifest"
    valid_digest "$source_vps_sha" || fail "installed source V2 digest is not canonical"
    release_publication_lock_sha256 "$source_release" "$source_vps_sha" >/dev/null ||
        fail "installed source V2 identity projection failed"
fi
case "$authority_phase" in
    target_verified)
        # target_verified already binds the exact typed verifier stdout digest.
        # The periodic timer may legitimately advance current status afterward,
        # so resume authenticates the immutable journal-bound receipt by identity.
        validate_receipt_identity "$target_backup_receipt" "$authority_target_receipt" ||
            fail "durable target BackupV4 receipt changed after journal verification"
        target_publication_sha=$(release_publication_lock_sha256 \
            "$candidate_root_fd" "$expected_vps_manifest_sha256") ||
            fail "could not project journaled target BackupV4 publication authority"
        transaction_backup_receipt "$candidate_root_fd/bin/robin-highscores-admin" \
            "$candidate_root_fd" "$expected_commit" "$expected_vps_manifest_sha256" \
            "$target_publication_sha" >/dev/null ||
            fail "current backup status is not exact authenticated target authority"
        if [ "$receipt_source_commit" != none ]; then
            validate_receipt_identity "$prebackup_receipt" "$authority_source_receipt" ||
                fail "journal-bound source receipt changed after target status publication"
        fi
        reuse_prebackup_receipt=1
        resume_post_migration=1
        ;;
    target_backup_started)
        if [ "$receipt_source_commit" != none ]; then
            validate_receipt_identity "$prebackup_receipt" "$authority_source_receipt" ||
                fail "journal-bound source receipt changed before target backup recovery"
        fi
        resume_post_migration=1
        if try_adopt_transaction_backup_receipt "$target_backup_receipt" \
            "$target_backup_receipt_temporary" "$candidate_root_fd/bin/robin-highscores-admin" \
            "$candidate_root_fd" "$expected_commit" "$expected_vps_manifest_sha256" \
            "$authority_backup_status_before"; then
            target_receipt_sha=$receipt_sha
            publish_authority_journal "$receipt_source_commit" present \
                "$authority_source_receipt" verified "$target_receipt_sha" \
                "$authority_backup_status_before" target_verified ||
                fail "could not adopt the target receipt after interrupted BackupV4 publication"
            reuse_prebackup_receipt=1
        else
            adoption_status=$?
            [ "$adoption_status" -eq 2 ] ||
                fail "changed target backup status is not exact authenticated transaction authority"
        fi
        ;;
    source_backup_started)
        [ "$receipt_source_commit" != none ] || fail "clean deployment cannot have a source backup phase"
        resume_source_backup=1
        if try_adopt_transaction_backup_receipt "$prebackup_receipt" \
            "$prebackup_receipt_temporary" "$source_admin" "$source_release" \
            "$receipt_source_commit" "$source_vps_sha" "$authority_backup_status_before"; then
            source_receipt_sha=$receipt_sha
            publish_authority_journal "$receipt_source_commit" present "$source_receipt_sha" \
                none none "$authority_backup_status_before" source_verified ||
                fail "could not adopt the source receipt after interrupted BackupV4 publication"
            reuse_prebackup_receipt=1
            prebackup_receipt_valid=1
        else
            adoption_status=$?
            [ "$adoption_status" -eq 2 ] ||
                fail "changed source backup status is not exact authenticated transaction authority"
        fi
        ;;
    source_verified|schema_verified)
        [ "$receipt_source_commit" != none ] || {
            [ "$authority_phase" = schema_verified ] || fail "clean deployment has an invalid source phase"
        }
        if [ "$receipt_source_commit" != none ]; then
            validate_transaction_backup_receipt "$prebackup_receipt" "$source_admin" "$source_release" \
                "$receipt_source_commit" "$source_vps_sha" "$authority_source_receipt" \
                "$authority_backup_status_before" ||
                fail "durable source BackupV4 receipt does not match fresh typed verification"
            reuse_prebackup_receipt=1
            prebackup_receipt_valid=1
        fi
        [ "$authority_phase" = schema_verified ] && resume_post_migration=1
        ;;
    runtime_ready) ;;
    *) fail "deployment authority journal is at an unusable phase" ;;
esac
if [ "$authority_source_receipt" = none ] &&
    { [ -e "$prebackup_receipt" ] || [ -L "$prebackup_receipt" ]; }; then
    fail "source receipt exists without a journaled source backup boundary"
fi
if [ "$authority_target_receipt" = none ] && [ "$authority_phase" != target_backup_started ] &&
    { [ -e "$target_backup_receipt" ] || [ -L "$target_backup_receipt" ]; }; then
    fail "target receipt exists without a journaled target backup boundary"
fi

if [ -n "$previous_commit" ] && [ "$previous_commit" != "$expected_commit" ] &&
    [ "$resume_post_migration" -eq 0 ]; then
    if [ "$reuse_prebackup_receipt" -eq 0 ]; then
        if [ "$resume_source_backup" -eq 0 ]; then
            verify_live_schema "$source_admin" "$source_release" "$source_vps_sha" ||
                fail "installed source release does not exactly attest the live database schema"
            if [ -f "$unit_backup_root/active/robin-highscores-api.service" ]; then
                probe http://127.0.0.1:8787/healthz 5 ||
                    fail "active current release is not healthy before pre-migration quiescence"
                probe http://127.0.0.1:8787/readyz 5 ||
                    fail "active current release is not ready before pre-migration quiescence"
            fi
        fi
        [ "$(stat -Lc %d:%i -- "$activation_lock_fd")" = "$activation_lock_identity" ] ||
            fail "inherited activation lock descriptor changed before pre-migration quiescence"
        [ -f "$activation_lock" ] && [ ! -L "$activation_lock" ] &&
            [ "$(stat -c %d:%i -- "$activation_lock")" = "$activation_lock_identity" ] ||
            fail "canonical activation lock path changed before pre-migration quiescence"
        activation_started=1
        prebackup_timer_disabled=1
        systemctl --user disable --now robin-highscores-backup.timer >/dev/null ||
            fail "could not durably disable the old backup timer before the source backup"
        systemctl --user disable --now robin-highscores.target >/dev/null ||
            fail "could not durably disable the old target before the source backup"
        if [ -d "$user_unit_root/timers.target.wants" ] && [ ! -L "$user_unit_root/timers.target.wants" ]; then
            sync -f -- "$user_unit_root/timers.target.wants"
        fi
        backup_wait=0
        while [ "$backup_wait" -lt 300 ]; do
            backup_state=$(systemctl --user show --property=ActiveState --value robin-highscores-backup.service) ||
                fail "could not inspect the raced backup service"
            case "$backup_state" in
                inactive) break ;;
                active|activating|deactivating) backup_wait=$((backup_wait + 1)); sleep 1 ;;
                *) fail "raced backup entered an unsafe state: $backup_state" ;;
            esac
        done
        [ "$(systemctl --user show --property=ActiveState --value robin-highscores-backup.service)" = inactive ] ||
            fail "timed out waiting for a raced verified backup without interrupting it"
        for unit in robin-highscores.target robin-highscores-worker.service robin-highscores-api.service; do
            systemctl --user stop "$unit" || fail "could not quiesce $unit before the offline source backup"
        done
        for unit in robin-highscores.target robin-highscores-api.service robin-highscores-worker.service \
            robin-highscores-backup.service robin-highscores-backup.timer; do
            [ "$(systemctl --user show --property=ActiveState --value "$unit")" = inactive ] ||
                fail "source writer remained active before the offline backup: $unit"
        done
        for unit in robin-highscores.target robin-highscores-backup.timer; do
            if enabled_state=$(systemctl --user is-enabled "$unit" 2>/dev/null); then
                [ "$enabled_state" = disabled ] || fail "source writer remains enabled before BackupV4: $unit"
            else
                [ "$enabled_state" = disabled ] || fail "could not prove source writer disabled before BackupV4: $unit"
            fi
        done
        for wants_directory in "$user_unit_root/default.target.wants" "$user_unit_root/timers.target.wants"; do
            if [ -e "$wants_directory" ] || [ -L "$wants_directory" ]; then
                [ -d "$wants_directory" ] && [ ! -L "$wants_directory" ] &&
                    sync -f -- "$wants_directory" || fail "could not durably quiesce source enablement"
            fi
        done
        [ ! -e "$prebackup_receipt" ] && [ ! -L "$prebackup_receipt" ] &&
            [ ! -e "$prebackup_receipt_temporary" ] && [ ! -L "$prebackup_receipt_temporary" ] ||
            fail "unauthenticated source receipt residue blocks a fresh BackupV4"
        estimate_backup_space "$source_admin" "$source_release" ||
            fail "typed source BackupV4 capacity admission failed"
        if [ "$authority_phase" = runtime_ready ]; then
            source_status_before=$(backup_status_sha256) ||
                fail "could not bind the pre-source BackupV4 status identity"
            publish_authority_journal "$receipt_source_commit" present none none none \
                "$source_status_before" source_backup_started ||
                fail "could not durably authorize the exact source BackupV4"
        else
            [ "$authority_phase" = source_backup_started ] ||
                fail "source BackupV4 resumed from the wrong authority phase"
            source_status_before=$authority_backup_status_before
        fi
        systemctl --user start robin-highscores-backup.service ||
            fail "fresh offline pre-migration backup under the current release failed"
        [ "$(systemctl --user show --property=ActiveState --value robin-highscores-backup.service)" = inactive ] ||
            fail "offline source backup service did not return to inactive"
        write_transaction_backup_receipt "$prebackup_receipt" \
            "$prebackup_receipt_temporary" "$source_admin" "$source_release" \
            "$receipt_source_commit" "$source_vps_sha" "$source_status_before" ||
            fail "fresh source BackupV4 receipt could not be authenticated and made durable"
        source_receipt_sha=$receipt_sha
        publish_authority_journal "$receipt_source_commit" present "$source_receipt_sha" \
            none none "$source_status_before" source_verified ||
            fail "could not bind the source BackupV4 receipt into the durable transaction"
        prebackup_receipt_valid=1
    fi
fi
backup_quiesced=1

[ "$(stat -Lc %d:%i -- "$activation_lock_fd")" = "$activation_lock_identity" ] ||
    fail "inherited activation lock descriptor changed before activation"
[ -f "$activation_lock" ] && [ ! -L "$activation_lock" ] &&
    [ "$(stat -c %d:%i -- "$activation_lock")" = "$activation_lock_identity" ] ||
    fail "canonical activation lock path changed before activation"
activation_started=1
if [ -n "$previous_commit" ]; then
    for unit in \
        robin-highscores.target \
        robin-highscores-backup.timer \
        robin-highscores-worker.service \
        robin-highscores-api.service
    do
        systemctl --user stop "$unit" || fail "could not stop $unit"
    done
    for unit in \
        robin-highscores.target \
        robin-highscores-api.service \
        robin-highscores-worker.service \
        robin-highscores-backup.service \
        robin-highscores-backup.timer
    do
        [ "$(systemctl --user show --property=ActiveState --value "$unit")" = inactive ] ||
            fail "unit did not become inactive before activation: $unit"
    done
    systemctl --user disable robin-highscores.target >/dev/null || fail "could not disable the old target"
    systemctl --user disable robin-highscores-backup.timer >/dev/null || fail "could not disable the old backup timer"
    for wants_directory in "$user_unit_root/default.target.wants" "$user_unit_root/timers.target.wants"; do
        if [ -e "$wants_directory" ] || [ -L "$wants_directory" ]; then
            [ -d "$wants_directory" ] && [ ! -L "$wants_directory" ] || fail "unsafe enablement directory: $wants_directory"
            sync -f -- "$wants_directory"
        fi
    done
fi

if [ "$resume_post_migration" -eq 0 ]; then
    migration_started=1
    "$release/bin/robin-highscores-admin" \
        --config "$release/config/highscores-server.toml" migrate
    migration_completed=1
else
    migration_started=1
    migration_completed=1
fi
verify_live_schema "$candidate_root_fd/bin/robin-highscores-admin" \
    "$candidate_root_fd" "$expected_vps_manifest_sha256" ||
    fail "target release does not exactly attest the migrated live database schema"
case "$authority_phase" in
    runtime_ready|source_verified)
        publish_authority_journal "$receipt_source_commit" present \
            "$authority_source_receipt" verified none \
            "$authority_backup_status_before" schema_verified ||
            fail "could not bind the target live-schema proof into the durable transaction"
        ;;
    schema_verified|target_backup_started|target_verified) ;;
    *) fail "target live-schema proof followed an invalid authority phase" ;;
esac

for unit in \
    robin-highscores.target \
    robin-highscores-api.service \
    robin-highscores-worker.service \
    robin-highscores-backup.service \
    robin-highscores-backup.timer
do
    destination=$user_unit_root/$unit
    staged_unit=$unit_stage_root/units/$unit
    if [ -e "$destination" ] || [ -L "$destination" ]; then
        [ -f "$destination" ] && [ ! -L "$destination" ] &&
            [ "$(stat -c %u -- "$destination")" -eq "$(id -u)" ] &&
            [ "$(stat -c %h -- "$destination")" -eq 1 ] &&
            [ "$(stat -c %a -- "$destination")" = 440 ] &&
            { { [ -f "$unit_backup_root/units/$unit" ] && cmp -s -- "$destination" "$unit_backup_root/units/$unit"; } ||
                cmp -s -- "$destination" "$release/systemd/user/$unit"; } ||
            fail "managed unit changed before replacement: $unit"
    else
        [ -f "$unit_backup_root/absent/$unit" ] || fail "managed unit disappeared before replacement: $unit"
    fi
    if ! mv -T -- "$staged_unit" "$destination"; then
        [ ! -e "$staged_unit" ] && [ ! -L "$staged_unit" ] &&
            [ -f "$destination" ] && [ ! -L "$destination" ] &&
            [ "$(stat -c %u -- "$destination")" -eq "$(id -u)" ] &&
            [ "$(stat -c %h -- "$destination")" -eq 1 ] &&
            [ "$(stat -c %a -- "$destination")" = 440 ] &&
            cmp -s -- "$destination" "$release/systemd/user/$unit" ||
            fail "could not atomically replace managed unit: $unit"
    fi
    sync -f -- "$destination"
done
sync -f -- "$user_unit_root"
systemctl --user daemon-reload

if [ "$authority_target_receipt" = none ]; then
    if [ "$authority_phase" = schema_verified ]; then
        estimate_backup_space "$candidate_root_fd/bin/robin-highscores-admin" "$release" ||
            fail "typed target BackupV4 capacity admission failed"
        target_status_before=$(backup_status_sha256) ||
            fail "could not bind the pre-target BackupV4 status identity"
        publish_authority_journal "$receipt_source_commit" present \
            "$authority_source_receipt" verified none "$target_status_before" \
            target_backup_started ||
            fail "could not durably authorize the exact target BackupV4"
    else
        [ "$authority_phase" = target_backup_started ] ||
            fail "target BackupV4 resumed from the wrong authority phase"
        target_status_before=$authority_backup_status_before
        # A changed, authenticated status is adopted before capacity admission. If the
        # status did not change, only then estimate capacity for a new backup.
        if try_adopt_transaction_backup_receipt "$target_backup_receipt" \
            "$target_backup_receipt_temporary" "$candidate_root_fd/bin/robin-highscores-admin" \
            "$candidate_root_fd" "$expected_commit" "$expected_vps_manifest_sha256" \
            "$target_status_before"; then
            target_receipt_sha=$receipt_sha
            publish_authority_journal "$receipt_source_commit" present \
                "$authority_source_receipt" verified "$target_receipt_sha" \
                "$target_status_before" target_verified ||
                fail "could not bind the adopted target BackupV4 receipt"
        else
            adoption_status=$?
            [ "$adoption_status" -eq 2 ] ||
                fail "changed target backup status is not exact authenticated transaction authority"
            [ ! -e "$target_backup_receipt" ] && [ ! -L "$target_backup_receipt" ] &&
                [ ! -e "$target_backup_receipt_temporary" ] && [ ! -L "$target_backup_receipt_temporary" ] ||
                fail "unauthenticated target receipt residue blocks a fresh BackupV4"
            estimate_backup_space "$candidate_root_fd/bin/robin-highscores-admin" "$release" ||
                fail "typed target BackupV4 capacity admission failed"
        fi
    fi
fi
if [ "$authority_target_receipt" = none ]; then
    systemctl --user start robin-highscores-backup.service ||
        fail "offline target BackupV4 service failed"
    [ "$(systemctl --user show --property=ActiveState --value robin-highscores-backup.service)" = inactive ] ||
        fail "offline target backup service did not return to inactive"
    write_transaction_backup_receipt "$target_backup_receipt" \
        "$target_backup_receipt_temporary" "$candidate_root_fd/bin/robin-highscores-admin" \
        "$candidate_root_fd" "$expected_commit" "$expected_vps_manifest_sha256" \
        "$target_status_before" ||
        fail "fresh target BackupV4 receipt could not be authenticated and made durable"
    target_receipt_sha=$receipt_sha
    publish_authority_journal "$receipt_source_commit" present \
        "$authority_source_receipt" verified "$target_receipt_sha" \
        "$target_status_before" target_verified ||
        fail "could not bind the target BackupV4 receipt into the durable transaction"
fi
runtime_authority_probe present ||
    fail "runtime authority changed after the target BackupV4 transaction"

if [ -n "$previous_target" ]; then
    [ -L "$current_link" ] && [ "$(stat -c %u -- "$current_link")" -eq "$(id -u)" ] &&
        { [ "$(readlink -- "$current_link")" = "$previous_target" ] ||
            [ "$(readlink -- "$current_link")" = "releases/$expected_commit" ]; } ||
        fail "current selector changed before target selection"
else
    [ ! -e "$current_link" ] && [ ! -L "$current_link" ] ||
        fail "current selector appeared before first target selection"
fi
if mv -T -- "$temporary_link" "$current_link"; then
    current_switched=1
    temporary_link_owned=0
else
    if [ -L "$current_link" ] && [ "$(readlink -- "$current_link")" = "releases/$expected_commit" ]; then
        current_switched=1
        [ -e "$temporary_link" ] || [ -L "$temporary_link" ] || temporary_link_owned=0
    fi
    fail "could not atomically select the new release"
fi
if ! sync -f -- "$opt_root"; then
    current_durability_uncertain=1
    fail "current selector changed but parent durability sync failed"
fi

systemctl --user start robin-highscores-api.service
probe http://127.0.0.1:8787/healthz 30 || fail "API did not pass /healthz after the offline target backup"
probe http://127.0.0.1:8787/readyz 30 || fail "API did not pass /readyz against the offline target backup"
systemctl --user start robin-highscores-worker.service
systemctl --user start robin-highscores.target
systemctl --user enable robin-highscores.target >/dev/null
systemctl --user enable --now robin-highscores-backup.timer >/dev/null
for wants_directory in "$user_unit_root/default.target.wants" "$user_unit_root/timers.target.wants"; do
    [ -d "$wants_directory" ] && [ ! -L "$wants_directory" ] || fail "expected enablement directory is unsafe or missing"
    sync -f -- "$wants_directory"
done
for unit in robin-highscores.target robin-highscores-backup.timer; do
    [ "$(systemctl --user is-enabled "$unit")" = enabled ] || fail "final unit is not persistently enabled: $unit"
done
for unit in robin-highscores.target robin-highscores-api.service robin-highscores-worker.service \
    robin-highscores-backup.timer; do
    [ "$(systemctl --user show --property=ActiveState --value "$unit")" = active ] || fail "final unit is not active: $unit"
done
[ "$(systemctl --user show --property=ActiveState --value robin-highscores-backup.service)" = inactive ] ||
    fail "verified backup service did not return to inactive"
probe http://127.0.0.1:8787/healthz 5 || fail "final liveness probe failed"
probe http://127.0.0.1:8787/readyz 5 || fail "final readiness probe failed"

journal_recorded_source=$(sed -n 's/^source_commit=//p' "$prepared_journal")
case "$journal_recorded_source" in
    none) ;;
    *[!0-9a-f]*|'') fail "prepared journal has invalid source identity" ;;
    *) [ "${#journal_recorded_source}" -eq 40 ] || fail "prepared journal has invalid source identity" ;;
esac
write_terminal_journal "$journal_recorded_source" ||
    fail "could not durably record the activation-complete cleanup phase"
activation_complete=1
preserve_recovery=1
finish_terminal_cleanup "$journal_recorded_source" ||
    fail "release is active but idempotent terminal cleanup did not complete"
preserve_recovery=0
echo "release $expected_commit is active and ready"
if [ -n "$previous_commit" ]; then
    echo "previous release retained for explicit rollback: $previous_commit"
fi
