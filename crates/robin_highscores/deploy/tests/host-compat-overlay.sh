#!/usr/bin/bash
set -euo pipefail
PATH=/usr/bin:/bin
export PATH

repo_root=$(git rev-parse --show-toplevel)
overlay_root=$repo_root/crates/robin_highscores/deploy/host-compat-overlay
tool=$overlay_root/robin-highscores-host-compat-overlay
fixture=$(mktemp -d)
trap 'chmod -R u+rwX "$fixture" 2>/dev/null || true; rm -rf -- "$fixture"' EXIT

test_home=$fixture/home
unit_root=$test_home/.config/systemd/user
runtime_root=$fixture/run/user/$(id -u)/systemd/user
mkdir -p -m 0700 -- "$unit_root"
chmod 0700 -- "$test_home" "$test_home/.config" "$test_home/.config/systemd" "$unit_root"
mkdir -p -- "$runtime_root"
chmod 0700 -- "$fixture/run/user/$(id -u)"
chmod 0755 -- "$fixture/run/user/$(id -u)/systemd" "$runtime_root"

commit=d3012fbb64954274aa4778df0882ba354e90df9c
for unit in robin-highscores-api.service robin-highscores-worker.service robin-highscores-backup.service; do
    git show "1064ab710:crates/robin_highscores/deploy/$unit" |
        sed "s/@SOURCE_COMMIT@/$commit/g" >"$unit_root/$unit"
    chmod 0440 -- "$unit_root/$unit"
    mkdir -m 0700 -- "$runtime_root/$unit.d"
    install -m 0400 -- \
        "$overlay_root/50-robin-highscores-openat2-compat.conf" \
        "$runtime_root/$unit.d/50-e080-vps-openat2.conf"
done

fake_systemctl=$fixture/systemctl
cat >"$fake_systemctl" <<'EOF'
#!/usr/bin/bash
set -euo pipefail
if [[ $1 == --user && $2 == daemon-reload && $# -eq 2 ]]; then
    exit 0
fi
[[ $1 == --user && $2 == show && $# -eq 5 ]] || exit 64
unit=$3
property=${4#--property=}
[[ $5 == --value ]]
unit_root=$ROBIN_HIGHSCORES_HOST_COMPAT_TEST_HOME/.config/systemd/user
runtime_root=$ROBIN_HIGHSCORES_HOST_COMPAT_TEST_RUNTIME_ROOT
drop_in=$unit_root/$unit.d/50-robin-highscores-openat2-compat.conf
transient=$runtime_root/$unit.d/50-e080-vps-openat2.conf
release_root=/home/robinhood/.local/opt/robin-highscores/releases/d3012fbb64954274aa4778df0882ba354e90df9c
state_root=/home/robinhood/.local/share/robin-highscores
case "$property" in
    LoadState) printf 'loaded\n' ;;
    FragmentPath) printf '%s/%s\n' "$unit_root" "$unit" ;;
    DropInPaths)
        paths=()
        [[ -f $drop_in ]] && paths+=("$drop_in")
        [[ -f $transient ]] && paths+=("$transient")
        [[ ${ROBIN_HIGHSCORES_HOST_COMPAT_TEST_EXTRA_DROPIN:-0} == 1 ]] &&
            paths+=(/tmp/unapproved-robin-highscores.conf)
        printf '%s\n' "${paths[*]}"
        ;;
    PrivateUsers) printf 'yes\n' ;;
    NoNewPrivileges)
        if [[ ${ROBIN_HIGHSCORES_HOST_COMPAT_TEST_FAIL_OVERLAP:-0} == 1 && -f $drop_in && -f $transient ]]; then
            printf 'no\n'
        elif [[ ${ROBIN_HIGHSCORES_HOST_COMPAT_TEST_FAIL_PERSISTENT_ONLY:-0} == 1 && -f $drop_in && ! -f $transient ]]; then
            printf 'no\n'
        else
            printf 'yes\n'
        fi
        ;;
    PrivateTmp|PrivateDevices|ProtectKernelTunables|ProtectKernelModules|ProtectKernelLogs|ProtectControlGroups|RestrictRealtime|LockPersonality)
        printf 'yes\n'
        ;;
    ProtectSystem) printf 'strict\n' ;;
    ProtectHome) printf 'read-only\n' ;;
    CapabilityBoundingSet|AmbientCapabilities) printf '\n' ;;
    RestrictSUIDSGID) [[ -f $drop_in || -f $transient ]] && printf 'no\n' || printf 'yes\n' ;;
    MemoryDenyWriteExecute)
        [[ $unit == robin-highscores-worker.service ]] && printf 'no\n' || printf 'yes\n'
        ;;
    RestrictNamespaces)
        [[ $unit == robin-highscores-api.service ]] && printf 'yes\n' || printf 'no\n'
        ;;
    RestrictAddressFamilies)
        case "$unit" in
            robin-highscores-api.service) printf 'AF_INET AF_INET6 AF_UNIX\n' ;;
            robin-highscores-worker.service) printf 'AF_NETLINK AF_UNIX\n' ;;
            robin-highscores-backup.service) printf 'AF_UNIX\n' ;;
        esac
        ;;
    SystemCallArchitectures)
        [[ $unit == robin-highscores-worker.service ]] && printf '\n' || printf 'native\n'
        ;;
    ProtectClock|ProtectHostname)
        [[ $unit == robin-highscores-backup.service ]] && printf 'yes\n' || printf 'no\n'
        ;;
    UMask) printf '0077\n' ;;
    MemorySwapMax) printf '0\n' ;;
    TasksMax)
        case "$unit" in
            robin-highscores-api.service) printf '1024\n' ;;
            robin-highscores-worker.service) printf '512\n' ;;
            robin-highscores-backup.service) printf '256\n' ;;
        esac
        ;;
    LimitNOFILE)
        [[ $unit == robin-highscores-api.service ]] && printf '16384\n' || printf '8192\n'
        ;;
    MemoryMax)
        case "$unit" in
            robin-highscores-api.service) printf '3221225472\n' ;;
            robin-highscores-worker.service) printf '2147483648\n' ;;
            robin-highscores-backup.service) printf '1073741824\n' ;;
        esac
        ;;
    ReadOnlyPaths)
        case "$unit" in
            robin-highscores-api.service)
                printf '%s\n' "$release_root $state_root/api-secrets $state_root/status $state_root/runtime-fence"
                ;;
            robin-highscores-worker.service)
                printf '%s\n' "/usr/bin/bwrap /usr/bin/prlimit $release_root $state_root/raw-content $state_root/runtime-fence"
                ;;
            robin-highscores-backup.service)
                printf '%s\n' "$release_root $unit_root/robin-highscores.target $unit_root/robin-highscores-api.service $unit_root/robin-highscores-worker.service $unit_root/robin-highscores-backup.service $unit_root/robin-highscores-backup.timer $state_root/api-secrets $state_root/runtime-fence"
                ;;
        esac
        ;;
    ReadWritePaths)
        value="$state_root/database $state_root/replays $state_root/campaign-states"
        [[ $unit == robin-highscores-backup.service ]] && value="$value $state_root/backups $state_root/status"
        printf '%s\n' "$value"
        ;;
    InaccessiblePaths)
        case "$unit" in
            robin-highscores-api.service) printf '%s\n' "$state_root/backups $state_root/raw-content" ;;
            robin-highscores-worker.service) printf '%s\n' "$state_root/api-secrets $state_root/backups $state_root/status" ;;
            robin-highscores-backup.service) printf '%s\n' "$state_root/raw-content" ;;
        esac
        ;;
    SystemCallFilter)
        [[ $unit == robin-highscores-worker.service ]] && printf '~\n' || printf '_llseek _newselect accept synthetic-expanded-system-service\n'
        ;;
    *) exit 64 ;;
esac
EOF
chmod 0500 -- "$fake_systemctl"

export ROBIN_HIGHSCORES_HOST_COMPAT_TEST_MODE=1
export ROBIN_HIGHSCORES_HOST_COMPAT_TEST_HOME=$test_home
export ROBIN_HIGHSCORES_HOST_COMPAT_TEST_SYSTEMCTL=$fake_systemctl
export ROBIN_HIGHSCORES_HOST_COMPAT_TEST_RUNTIME_ROOT=$runtime_root

staging=$unit_root/.robin-highscores-api.service.d.host-compat-install
destination=$unit_root/robin-highscores-api.service.d
for adversary in wrong_name subdirectory symlink extra_entry; do
    mkdir -m 0700 -- "$staging"
    case "$adversary" in
        wrong_name)
            printf 'unexpected' >"$staging/not-the-drop-in.conf"
            chmod 0400 -- "$staging/not-the-drop-in.conf"
            ;;
        subdirectory)
            mkdir -m 0700 -- "$staging/50-robin-highscores-openat2-compat.conf"
            ;;
        symlink)
            ln -s -- /etc/passwd "$staging/50-robin-highscores-openat2-compat.conf"
            ;;
        extra_entry)
            printf 'one' >"$staging/one"
            printf 'two' >"$staging/two"
            chmod 0400 -- "$staging/one" "$staging/two"
            ;;
    esac
    if "$tool" install-and-retire-transient; then
        echo "adversarial staging case was accepted: $adversary" >&2
        exit 1
    fi
    [[ ! -e $destination && ! -L $destination ]]
    case "$adversary" in
        wrong_name) unlink -- "$staging/not-the-drop-in.conf" ;;
        subdirectory) rmdir -- "$staging/50-robin-highscores-openat2-compat.conf" ;;
        symlink) unlink -- "$staging/50-robin-highscores-openat2-compat.conf" ;;
        extra_entry)
            unlink -- "$staging/one"
            unlink -- "$staging/two"
            ;;
    esac
    rmdir -- "$staging"
done

export ROBIN_HIGHSCORES_HOST_COMPAT_TEST_CRASH_AFTER_STAGE_MKDIR=1
if "$tool" install-and-retire-transient; then
    echo "injected post-mkdir staging crash unexpectedly succeeded" >&2
    exit 1
fi
unset ROBIN_HIGHSCORES_HOST_COMPAT_TEST_CRASH_AFTER_STAGE_MKDIR
empty_stage=$unit_root/.robin-highscores-api.service.d.host-compat-install
[[ -d $empty_stage && $(find "$empty_stage" -mindepth 1 -maxdepth 1 -printf . | wc -c) -eq 0 ]]
# Simulate death inside install(1), after it created the exact staging name but
# before the 30-byte copy became complete.
printf 'partial' >"$empty_stage/50-robin-highscores-openat2-compat.conf"
chmod 0400 -- "$empty_stage/50-robin-highscores-openat2-compat.conf"

export ROBIN_HIGHSCORES_HOST_COMPAT_TEST_FAIL_OVERLAP=1
if "$tool" install-and-retire-transient; then
    echo "failed overlap proof was accepted" >&2
    exit 1
fi
unset ROBIN_HIGHSCORES_HOST_COMPAT_TEST_FAIL_OVERLAP
for unit in robin-highscores-api.service robin-highscores-worker.service robin-highscores-backup.service; do
    [[ ! -e $unit_root/$unit.d && ! -L $unit_root/$unit.d ]]
    [[ -f $runtime_root/$unit.d/50-e080-vps-openat2.conf ]]
done

export ROBIN_HIGHSCORES_HOST_COMPAT_TEST_FAIL_PERSISTENT_ONLY=1
if "$tool" install-and-retire-transient; then
    echo "failed persistent-only proof was accepted" >&2
    exit 1
fi
unset ROBIN_HIGHSCORES_HOST_COMPAT_TEST_FAIL_PERSISTENT_ONLY
for unit in robin-highscores-api.service robin-highscores-worker.service robin-highscores-backup.service; do
    [[ -f $unit_root/$unit.d/50-robin-highscores-openat2-compat.conf ]]
    [[ -f $runtime_root/$unit.d/50-e080-vps-openat2.conf ]]
done

for crash_point in 1 2 3; do
    if [[ $crash_point -ne 1 ]]; then
        # Simulate /run recreation at reboot before reconstructing the exact
        # transient-only starting point for the next crash position.
        for unit in robin-highscores-api.service robin-highscores-worker.service robin-highscores-backup.service; do
            quarantine=$runtime_root/.$unit.d.e080-retire
            unlink -- "$quarantine/50-e080-vps-openat2.conf"
            rmdir -- "$quarantine"
        done
    fi
    for unit in robin-highscores-api.service robin-highscores-worker.service robin-highscores-backup.service; do
        if [[ ! -d $runtime_root/$unit.d ]]; then
            mkdir -m 0700 -- "$runtime_root/$unit.d"
            install -m 0400 -- \
                "$overlay_root/50-robin-highscores-openat2-compat.conf" \
                "$runtime_root/$unit.d/50-e080-vps-openat2.conf"
        fi
    done
    export ROBIN_HIGHSCORES_HOST_COMPAT_TEST_CRASH_AFTER_QUARANTINE=$crash_point
    if "$tool" install-and-retire-transient; then
        echo "injected runtime-quarantine crash $crash_point unexpectedly succeeded" >&2
        exit 1
    fi
    unset ROBIN_HIGHSCORES_HOST_COMPAT_TEST_CRASH_AFTER_QUARANTINE
    remaining_quarantines=0
    remaining_transients=0
    for unit in robin-highscores-api.service robin-highscores-worker.service robin-highscores-backup.service; do
        [[ -d $runtime_root/.$unit.d.e080-retire ]] &&
            remaining_quarantines=$((remaining_quarantines + 1))
        [[ -d $runtime_root/$unit.d ]] && remaining_transients=$((remaining_transients + 1))
    done
    [[ $remaining_quarantines -eq $crash_point ]]
    [[ $remaining_transients -eq $((3 - crash_point)) ]]
    "$tool" install-and-retire-transient
done
"$tool" install-and-retire-transient
"$tool" verify

# A reboot removes all authenticated /run quarantines together. The durable
# verifier must accept the exact persistent-only state after that evaporation.
for unit in robin-highscores-api.service robin-highscores-worker.service robin-highscores-backup.service; do
    quarantine=$runtime_root/.$unit.d.e080-retire
    unlink -- "$quarantine/50-e080-vps-openat2.conf"
    rmdir -- "$quarantine"
done
"$tool" verify

staged_duplicate=$unit_root/.robin-highscores-api.service.d.host-compat-install
mkdir -m 0700 -- "$staged_duplicate"
install -m 0400 -- "$overlay_root/50-robin-highscores-openat2-compat.conf" \
    "$staged_duplicate/50-robin-highscores-openat2-compat.conf"
export ROBIN_HIGHSCORES_HOST_COMPAT_TEST_CRASH_AFTER_STAGE_UNLINK=1
if "$tool" install-and-retire-transient; then
    echo "injected post-unlink staging crash unexpectedly succeeded" >&2
    exit 1
fi
unset ROBIN_HIGHSCORES_HOST_COMPAT_TEST_CRASH_AFTER_STAGE_UNLINK
[[ -d $staged_duplicate && $(find "$staged_duplicate" -mindepth 1 -maxdepth 1 -printf . | wc -c) -eq 0 ]]
"$tool" install-and-retire-transient
[[ ! -e $staged_duplicate && ! -L $staged_duplicate ]]

export ROBIN_HIGHSCORES_HOST_COMPAT_TEST_EXTRA_DROPIN=1
if "$tool" verify; then
    echo "unapproved effective DropInPaths entry was accepted" >&2
    exit 1
fi
unset ROBIN_HIGHSCORES_HOST_COMPAT_TEST_EXTRA_DROPIN

for unit in robin-highscores-api.service robin-highscores-worker.service robin-highscores-backup.service; do
    directory=$unit_root/$unit.d
    drop_in=$directory/50-robin-highscores-openat2-compat.conf
    [[ -d $directory && ! -L $directory && $(stat -c %a -- "$directory") == 700 ]]
    [[ -f $drop_in && ! -L $drop_in && $(stat -c %a -- "$drop_in") == 400 ]]
    [[ $(stat -c %h -- "$drop_in") -eq 1 ]]
    [[ $(sha256sum "$drop_in" | cut -d' ' -f1) == 832331e248a58932f3f0923285d7979d7316306cd0947d2502408e8e08bef579 ]]
    [[ ! -e $runtime_root/$unit.d && ! -L $runtime_root/$unit.d ]]
done

first_drop_in=$unit_root/robin-highscores-api.service.d/50-robin-highscores-openat2-compat.conf
chmod 0600 -- "$first_drop_in"
if "$tool" verify; then
    echo "tampered drop-in metadata was accepted" >&2
    exit 1
fi
chmod 0400 -- "$first_drop_in"

"$tool" remove
"$tool" remove
for unit in robin-highscores-api.service robin-highscores-worker.service robin-highscores-backup.service; do
    [[ ! -e $unit_root/$unit.d && ! -L $unit_root/$unit.d ]]
    [[ -d $unit_root/.$unit.d.host-compat-remove ]]
done

echo "host compatibility overlay tests passed"
