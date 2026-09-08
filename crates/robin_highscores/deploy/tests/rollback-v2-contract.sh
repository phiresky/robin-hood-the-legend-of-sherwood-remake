#!/bin/sh
set -eu

script=${1:-crates/robin_highscores/deploy/rollback-release.sh}

fail() {
    echo "rollback V2 contract test failed: $*" >&2
    exit 1
}

/bin/sh -n "$script" || fail "script is not POSIX-sh syntax"

require() {
    grep -Fq -- "$1" "$script" || fail "missing contract token: $1"
}

reject() {
    if grep -Eq -- "$1" "$script"; then
        fail "forbidden rollback authority remains: $1"
    fi
}

for token in \
    'verify-live-database-schema-v2' \
    'project-vps-publication-lock-v2' \
    'verify-transaction-backup' \
    'estimate-backup-space' \
    '--require-available' \
    'backup-authority-hmac.key' \
    'db-admission.lock' \
    'db-quiescence.lock' \
    'canonical activation lock was not already held' \
    'source_backup_started' \
    'source_backup_invalidated' \
    'converge_source_units_stopped' \
    'target_backup_started' \
    'systemctl --user disable --now robin-highscores.target' \
    'systemctl --user disable --now robin-highscores-backup.timer' \
    'durable source receipt no longer matches typed verification' \
    'durable target BackupV4 receipt no longer matches a fresh typed verification' \
    'source BackupV4 receipt was durably invalidated before source restart' \
    'rollback source BackupV4 receipt is not bound before target selection' \
    'rollback target BackupV4 receipt changed before target selection' \
    'source remains selected and stopped for resume'
do
    require "$token"
done

reject 'consume-vps-sources-v2|expected_plan_sha256|PLAN_FD'
reject 'backup_status_sha256=|backup_du=|du[[:space:]]+--bytes'
reject 'publication_lock_sha256=.*sed|sed.*publication_lock_sha256'
reject 'restore-source-map.*backup-authority-hmac.key'
reject '--release-manifest-fd[[:space:]]+3'
reject 'source_backup_verified[[:space:]]+target_backup_verified'
reject 'if ! classify_backup_status_since'

line_for() {
    grep -nF -- "$1" "$script" | tail -n 1 | cut -d: -f1
}

schema_gate=$(line_for 'retained rollback target changed at the live-schema receipt boundary')
journal_reconcile=$(line_for 'reconcile_prepared_journal_temporary ||')
source_receipt=$(line_for 'rollback source BackupV4 receipt is not bound before target selection')
selector=$(line_for 'selector durability is uncertain')
target_receipt=$(line_for 'rollback target BackupV4 receipt changed before target selection')
api_start=$(line_for 'systemctl --user start robin-highscores-api.service')
writers_inactive=$(line_for 'source writer remained active before the offline rollback backup')
source_capacity=$(line_for 'typed stopped rollback backup capacity admission failed')
source_backup=$(line_for 'fresh pre-rollback backup failed')
source_started=$(line_for 'could not durably record the source BackupV4 start boundary')
target_units=$(line_for 'systemctl --user daemon-reload')
target_capacity=$(line_for 'initial typed rollback target-backup capacity admission failed')
target_started=$(line_for 'could not durably record the target BackupV4 start boundary')
target_backup=$(line_for 'offline rollback target backup failed')
invalidate=$(line_for 'advance_prepared_journal "$previous_commit" source_backup_verified source_backup_invalidated')
restore=$(line_for 'elif ! restore_previous_state; then')
source_converged=$(line_for 'converge_source_units_stopped ||')
invalidated_retired=$(line_for 'could not make source BackupV4 receipt retirement durable')
source_receipt_write=$(line_for 'fresh rollback prebackup receipt could not be authenticated and made durable')
source_verified=$(line_for 'advance_prepared_journal "$previous_commit" source_backup_started source_backup_verified')
target_receipt_write=$(line_for 'fresh rollback target BackupV4 receipt could not be verified and made durable')
target_verified=$(line_for 'advance_prepared_journal "$previous_commit" target_backup_started target_backup_verified')
source_status_condition=$(grep -nF 'if [ "$backup_status_change" = unchanged ]; then' "$script" | head -n 1 | cut -d: -f1)
target_status_condition=$(grep -nF 'if [ "$backup_status_change" = unchanged ]; then' "$script" | tail -n 1 | cut -d: -f1)
source_service_start=$(grep -nF 'systemctl --user start robin-highscores-backup.service' "$script" | head -n 1 | cut -d: -f1)
target_service_start=$(grep -nF 'systemctl --user start robin-highscores-backup.service' "$script" | tail -n 1 | cut -d: -f1)

[ "$schema_gate" -lt "$journal_reconcile" ] || fail "journal mutation can precede live-schema admission"
[ "$source_receipt" -lt "$selector" ] || fail "selector can move before the source BackupV4 receipt"
[ "$target_units" -lt "$target_capacity" ] || fail "target capacity admission can use source unit definitions"
[ "$target_capacity" -lt "$target_started" ] || fail "target backup start can precede typed capacity admission"
[ "$target_started" -lt "$target_backup" ] || fail "target BackupV4 can start before its durable start phase"
[ "$target_receipt" -lt "$selector" ] || fail "selector can move before the target BackupV4 receipt is journaled"
[ "$target_receipt" -lt "$api_start" ] || fail "API can start before target BackupV4 verification"
[ "$writers_inactive" -lt "$source_capacity" ] || fail "source capacity admission can race active writers"
[ "$source_capacity" -lt "$source_backup" ] || fail "source backup can start before stopped capacity admission"
[ "$source_started" -lt "$source_backup" ] || fail "source BackupV4 can start before its durable start phase"
[ "$source_receipt_write" -lt "$source_verified" ] || fail "source journal can verify before exact receipt publication"
[ "$target_receipt_write" -lt "$target_verified" ] || fail "target journal can verify before exact receipt publication"
[ "$invalidate" -lt "$restore" ] || fail "cleanup can restart source before durable receipt invalidation"
[ "$source_converged" -lt "$invalidated_retired" ] || fail "invalidated resume can retire receipt before exact source-unit convergence"
[ "$source_status_condition" -lt "$source_service_start" ] || fail "source backup start is not guarded by exact unchanged classification"
[ "$target_status_condition" -lt "$target_service_start" ] || fail "target backup start is not guarded by exact unchanged classification"

# Crash-boundary phase model. A SIGKILL after BackupV4 status publication but
# before verifier stdout, temporary receipt publication, final receipt rename,
# or journal advance must resume with the verifier selected by the durable
# started phase. Target failures retain the source selector and stopped units.
resume_after_status_publish() {
    case "$1:$2:$3:$4" in
        source_backup_started:source:changed:status|source_backup_started:source:changed:before_stdout|source_backup_started:source:changed:before_temp|source_backup_started:source:changed:before_mv|source_backup_started:source:changed:before_journal)
            printf '%s\n' 'source-verifier adopt source stopped'
            ;;
        target_backup_started:source:changed:status|target_backup_started:source:changed:before_stdout|target_backup_started:source:changed:before_temp|target_backup_started:source:changed:before_mv|target_backup_started:source:changed:before_journal)
            printf '%s\n' 'target-verifier adopt source stopped'
            ;;
        source_backup_invalidated:source:*:*)
            printf '%s\n' 'fresh-source-backup source stopped'
            ;;
        *) return 1 ;;
    esac
}

for boundary in status before_stdout before_temp before_mv before_journal; do
    [ "$(resume_after_status_publish source_backup_started source changed "$boundary")" = \
        'source-verifier adopt source stopped' ] || fail "source $boundary crash selects a stale verifier"
    [ "$(resume_after_status_publish target_backup_started source changed "$boundary")" = \
        'target-verifier adopt source stopped' ] || fail "target $boundary crash moves selector or selects source verifier"
done
[ "$(resume_after_status_publish source_backup_invalidated source any reboot)" = \
    'fresh-source-backup source stopped' ] || fail "reboot can reuse a durably invalidated source receipt"

# Reproduce the frozen outer case where the manifest tool itself occupies FD3.
# The nested Bash allocator must choose another FD for the release manifest.
fixture=$(mktemp -d)
trap 'rm -rf -- "$fixture"' EXIT HUP INT TERM

# The outer activation lock and the first retained release may occupy the
# historical 8/9 pair.  Exercise the production allocator verbatim: it must
# retain two distinct directory authorities without overwriting inherited FDs,
# skip arbitrary occupied slots, and fail closed when the bounded pool is full.
allocator_lib=$fixture/fd-allocator.sh
sed -n '/^open_pinned_release_root() {$/,/^}$/p' "$script" >"$allocator_lib"
. "$allocator_lib"
mkdir -- "$fixture/target" "$fixture/source"
(
    exec 8< /dev/null
    exec 9< /dev/null
    open_pinned_release_root "$fixture/target" target_fd || exit 1
    open_pinned_release_root "$fixture/source" source_fd || exit 1
    [ "$target_fd" != 8 ] && [ "$target_fd" != 9 ] &&
        [ "$source_fd" != 8 ] && [ "$source_fd" != 9 ] &&
        [ "$target_fd" != "$source_fd" ] &&
        [ "$(stat -Lc %d:%i -- "/proc/self/fd/$target_fd")" = \
            "$(stat -c %d:%i -- "$fixture/target")" ] &&
        [ "$(stat -Lc %d:%i -- "/proc/self/fd/$source_fd")" = \
            "$(stat -c %d:%i -- "$fixture/source")" ]
) || fail "retained rollback target/source descriptors collide with inherited authority"
(
    exec 3< /dev/null; exec 4< /dev/null; exec 5< /dev/null; exec 6< /dev/null
    exec 7< /dev/null; exec 8< /dev/null; exec 9< /dev/null
    ! open_pinned_release_root "$fixture/source" exhausted_fd
) || fail "rollback descriptor exhaustion did not fail closed"

# Exercise the production classifier itself, extracted verbatim. Classification
# errors are fail-closed; only exact equality admits one new backup. Any valid
# changed identity selects the lane-specific typed verifier and never starts a
# second backup, including when that verifier rejects foreign/malformed bytes.
classifier_lib=$fixture/status-classifier.sh
for classifier_function in current_backup_status_sha256 valid_status_boundary classify_backup_status_since; do
    sed -n "/^$classifier_function() {$/,/^}$/p" "$script" >>"$classifier_lib"
done
. "$classifier_lib"
status_fixture=$fixture/backup-status.json
backup_status_path=$status_fixture
printf '%s\n' baseline >"$status_fixture"
chmod 0400 -- "$status_fixture"
status_boundary=$(sha256sum "$status_fixture" | cut -d' ' -f1)

resume_status_decision() {
    decision_lane=$1
    decision_boundary=$2
    decision_verifier=$3
    if ! classify_backup_status_since "$decision_boundary"; then
        printf '%s\n' "$decision_lane:fail-closed-zero-service-mutation"
        return
    fi
    case "$backup_status_change:$decision_verifier" in
        unchanged:*) printf '%s\n' "$decision_lane:one-new-backup" ;;
        changed:authentic) printf '%s\n' "$decision_lane:adopt-zero-new-backup" ;;
        changed:*) printf '%s\n' "$decision_lane:fail-closed-zero-service-mutation" ;;
        *) return 1 ;;
    esac
}

for lane in source target; do
    [ "$(resume_status_decision "$lane" "$status_boundary" unused)" = "$lane:one-new-backup" ] ||
        fail "$lane unchanged status does not admit exactly one backup"

    chmod 0600 -- "$status_fixture"
    printf '%s\n' authentic-changed >"$status_fixture"
    chmod 0400 -- "$status_fixture"
    [ "$(resume_status_decision "$lane" "$status_boundary" authentic)" = "$lane:adopt-zero-new-backup" ] ||
        fail "$lane authentic changed status is not adopted without a second backup"

    for rejected_status in foreign malformed; do
        chmod 0600 -- "$status_fixture"
        printf '%s\n' "$rejected_status-status" >"$status_fixture"
        chmod 0400 -- "$status_fixture"
        [ "$(resume_status_decision "$lane" "$status_boundary" rejected)" = \
            "$lane:fail-closed-zero-service-mutation" ] ||
            fail "$lane $rejected_status changed status does not fail closed"
    done

    chmod 0600 -- "$status_fixture"
    [ "$(resume_status_decision "$lane" "$status_boundary" authentic)" = \
        "$lane:fail-closed-zero-service-mutation" ] || fail "$lane unsafe status mode admits mutation"

    chmod 0000 -- "$status_fixture"
    [ "$(resume_status_decision "$lane" "$status_boundary" authentic)" = \
        "$lane:fail-closed-zero-service-mutation" ] || fail "$lane unreadable status admits mutation"

    rm -f -- "$status_fixture"
    printf '%s\n' symlink-target >"$fixture/status-target"
    ln -s -- "$fixture/status-target" "$status_fixture"
    [ "$(resume_status_decision "$lane" "$status_boundary" authentic)" = \
        "$lane:fail-closed-zero-service-mutation" ] || fail "$lane status symlink admits mutation"

    rm -f -- "$status_fixture"
    mkdir -- "$status_fixture"
    [ "$(resume_status_decision "$lane" "$status_boundary" authentic)" = \
        "$lane:fail-closed-zero-service-mutation" ] || fail "$lane status directory admits mutation"
    rmdir -- "$status_fixture"

    [ "$(resume_status_decision "$lane" "$status_boundary" rejected)" = \
        "$lane:fail-closed-zero-service-mutation" ] || fail "$lane vanished status admits mutation"

    printf '%s\n' baseline >"$status_fixture"
    chmod 0400 -- "$status_fixture"
done

mkdir -p -- "$fixture/release"
printf 'typed-manifest\n' >"$fixture/release/vps-release-manifest-v2.json"
apply_mock=$fixture/manifestctl
printf '%s\n' \
    '#!/bin/sh' \
    'set -eu' \
    '[ "$1" = project-vps-publication-lock-v2 ]' \
    '[ "$2" = --release-manifest-fd ]' \
    'manifest_fd=$3' \
    '[ "$manifest_fd" != 3 ]' \
    '[ "$4" = --expected-vps-release-manifest-sha256 ]' \
    '[ "$(cat -- "/proc/self/fd/$manifest_fd")" = typed-manifest ]' \
    'printf "%064d\n" 1' >"$apply_mock"
chmod 0700 -- "$apply_mock"
exec 3< "$apply_mock"
projection=$(/usr/bin/bash -c '
    set -eu
    exec {release_manifest_fd}<"$1/vps-release-manifest-v2.json"
    exec "$2" project-vps-publication-lock-v2 \
        --release-manifest-fd "$release_manifest_fd" \
        --expected-vps-release-manifest-sha256 "$3"
' rollback-v2-fd3-test "$fixture/release" /proc/self/fd/3 "$(printf '%064d' 2)")
[ "$projection" = "$(printf '%064d' 1)" ] || fail "dynamic projection failed with manifestctl on FD3"

echo "rollback V2 static contract passed"
