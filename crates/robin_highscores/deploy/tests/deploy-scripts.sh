#!/bin/sh
set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
validator=$script_directory/validate-release-bundle.sh

for script in \
    "$script_directory/deploy-release.sh" \
    "$script_directory/rollback-release.sh" \
    "$script_directory/root-once.sh" \
    "$validator"
do
    sh -n "$script"
done
(cd "$script_directory" && sha256sum --strict --check ROOT_ONCE_SHA256SUMS >/dev/null)

temporary_root=$(mktemp -d)
cleanup() {
    rm -rf -- "$temporary_root"
}
trap cleanup EXIT HUP INT TERM

commit=0123456789abcdef0123456789abcdef01234567
bundle=$temporary_root/$commit
release_root=/home/robinhood/.local/opt/robin-highscores/releases/$commit
state_root=/home/robinhood/.local/share/robin-highscores
mkdir -p \
    "$bundle/bin" \
    "$bundle/config" \
    "$bundle/deploy" \
    "$bundle/deploy/tests" \
    "$bundle/systemd/user"

printf '%s\n' "$commit" > "$bundle/SOURCE_COMMIT"
printf 'fixture mode inventory\n' > "$bundle/MODE_INVENTORY"
printf '{}\n' > "$bundle/vps-release-manifest-v2.json"

for binary in \
    robin-highscores-admin \
    robin-highscores-server \
    robin-highscores-worker \
    robin-replay-verifier
do
    printf '#!/bin/sh\nexit 0\n' > "$bundle/bin/$binary"
done
printf '#!/bin/sh\n[ "$1" = validate-vps-release-v2 ]\n[ -d "$2" ]\n' \
    > "$bundle/bin/robin-highscores-manifestctl"

printf '%s\n' \
    'bind = "127.0.0.1:8787"' \
    "database_path = \"$state_root/database/highscores.sqlite3\"" \
    "replay_directory = \"$state_root/replays\"" \
    "campaign_state_directory = \"$state_root/campaign-states\"" \
    "cursor_secret_path = \"$state_root/api-secrets/cursor-hmac.key\"" \
    "competition_run_grant_secret_path = \"$state_root/api-secrets/competition-run-grant.key\"" \
    "run_preflight_grant_secret_path = \"$state_root/api-secrets/run-preflight-grant.key\"" \
    "moderation_bearer_token_path = \"$state_root/api-secrets/moderation-bearer.token\"" \
    'allowed_origins = []' \
    'trusted_proxy_cidrs = ["127.0.0.1/32", "::1/128"]' \
    "backup_manifest_path = \"$state_root/status/backup-status.json\"" \
    "backup_authority_hmac_secret_path = \"$state_root/api-secrets/backup-authority-hmac.key\"" \
    "runtime_fence_directory = \"$state_root/runtime-fence\"" \
    "release_manifest_path = \"$release_root/vps-release-manifest-v2.json\"" \
    'maximum_backup_age_hours = 32' \
    > "$bundle/config/highscores-server.toml"
printf '%s\n' \
    "server_config = \"$release_root/config/highscores-server.toml\"" \
    "campaign_state_directory = \"$state_root/campaign-states\"" \
    "demo_raw_content_manifest = \"$release_root/private/demo.json\"" \
    "full_raw_content_manifest = \"$release_root/private/full.json\"" \
    '[verifier_launcher]' \
    'bwrap_program = "/usr/bin/bwrap"' \
    'prlimit_program = "/usr/bin/prlimit"' \
    "verifier_program = \"$release_root/bin/robin-replay-verifier\"" \
    > "$bundle/config/highscores-worker.toml"
printf 'RUST_LOG=info\n' > "$bundle/config/api.env"
printf 'RUST_LOG=info\n' > "$bundle/config/worker.env"

for document in README.md VPS_RELEASE_INSTALL.md BACKUP_RESTORE.md \
    nginx-robinhood-api.challenge.conf nginx-robinhood-cloudflare-only.conf \
    nginx-robinhood-api.locations.conf nginx-robinhood-api.vhost.conf
do
    printf 'fixture %s\n' "$document" > "$bundle/deploy/$document"
done
cp -- "$script_directory/ROOT_ONCE_SHA256SUMS" \
    "$bundle/deploy/ROOT_ONCE_SHA256SUMS"
cp -- "$validator" "$bundle/deploy/validate-release-bundle.sh"
cp -- "$script_directory/deploy-release.sh" "$bundle/deploy/deploy-release.sh"
cp -- "$script_directory/rollback-release.sh" "$bundle/deploy/rollback-release.sh"
cp -- "$script_directory/root-once.sh" "$bundle/deploy/root-once.sh"
cp -- "$script_directory/tests/real-runtime-fence-release-gate.sh" \
    "$bundle/deploy/tests/real-runtime-fence-release-gate.sh"
cp -- "$script_directory/tests/real-runtime-fence-e2e.py" \
    "$bundle/deploy/tests/real-runtime-fence-e2e.py"
cp -- "$script_directory/tests/real-runtime-fence-e2e-selftest.py" \
    "$bundle/deploy/tests/real-runtime-fence-e2e-selftest.py"
(cd "$bundle/deploy" && sha256sum \
    deploy-release.sh rollback-release.sh validate-release-bundle.sh) \
    > "$bundle/deploy/DEPLOY_BOOTSTRAP_SHA256SUMS"

printf '[Service]\nExecStart=%s/bin/robin-highscores-server --config %s/config/highscores-server.toml\nReadOnlyPaths=%s/status\nReadOnlyPaths=%s/runtime-fence\nReadOnlyPaths=%s/api-secrets\nInaccessiblePaths=%s/backups\n' \
    "$release_root" "$release_root" "$state_root" "$state_root" \
    "$state_root" "$state_root" \
    > "$bundle/systemd/user/robin-highscores-api.service"
printf '[Service]\nExecStart=%s/bin/robin-highscores-worker --config %s/config/highscores-worker.toml\nReadOnlyPaths=%s/runtime-fence\nInaccessiblePaths=%s/api-secrets\nInaccessiblePaths=%s/backups\nInaccessiblePaths=%s/status\n' \
    "$release_root" "$release_root" "$state_root" "$state_root" \
    "$state_root" "$state_root" \
    > "$bundle/systemd/user/robin-highscores-worker.service"
sed "s/@SOURCE_COMMIT@/$commit/g" "$script_directory/robin-highscores-backup.service" \
    > "$bundle/systemd/user/robin-highscores-backup.service"
printf '[Unit]\nDescription=fixture target\n' > "$bundle/systemd/user/robin-highscores.target"
printf '[Timer]\nOnCalendar=*-*-* 02:15:00\nRandomizedDelaySec=45m\n' > "$bundle/systemd/user/robin-highscores-backup.timer"

find "$bundle" -type d -exec chmod 0750 -- {} +
find "$bundle" -type f -exec chmod 0640 -- {} +
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
    chmod 0750 -- "$bundle/$executable"
done
chmod 0550 -- "$bundle/bin/robin-highscores-manifestctl"

refresh_sums() {
    rm -f -- "$bundle/SHA256SUMS"
    (cd "$bundle" && find . -type f ! -path ./SHA256SUMS -printf '%P\n' | LC_ALL=C sort |
        while IFS= read -r relative; do sha256sum "$relative"; done) > "$bundle/SHA256SUMS"
    chmod 0640 -- "$bundle/SHA256SUMS"
    sha256sum "$bundle/SHA256SUMS" | cut -d' ' -f1
}

expect_failure() {
    if "$@" >/dev/null 2>&1; then
        echo "expected command to fail: $*" >&2
        exit 1
    fi
}

manifest_tool_authority_function=$(sed -n \
    '/^manifest_tool_authority_is_accepted() {$/,/^}$/p' "$validator")
[ -n "$manifest_tool_authority_function" ] || {
    echo "validator omits manifest-tool authority predicate" >&2
    exit 1
}
eval "$manifest_tool_authority_function"

fixture_digest=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
manifest_tool_authority_is_accepted \
    "$fixture_digest" "$fixture_digest" 0123456789abcdef0123456789abcdef01234567 \
    "$fixture_digest" "$fixture_digest"
manifest_tool_authority_is_accepted \
    6b5417d94ff17349188bc189ab996948ff58fee26dc16c3cdae432e347688ad9 \
    c7a158a6d534509728051e5740a4f71f18e0dc8f7f285cf8038643c8f0162157 \
    d3012fbb64954274aa4778df0882ba354e90df9c \
    673037c81a26f4254c66c0a286a0a879239fd7d01033fb05f97b32c2341f354c \
    1dbeead7aba2f62eb95acb2f5b2ea19031a5b62c1d81252079399bd33c331d72
for rejected_authority in \
    'ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff c7a158a6d534509728051e5740a4f71f18e0dc8f7f285cf8038643c8f0162157 d3012fbb64954274aa4778df0882ba354e90df9c 673037c81a26f4254c66c0a286a0a879239fd7d01033fb05f97b32c2341f354c 1dbeead7aba2f62eb95acb2f5b2ea19031a5b62c1d81252079399bd33c331d72' \
    '6b5417d94ff17349188bc189ab996948ff58fee26dc16c3cdae432e347688ad9 ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff d3012fbb64954274aa4778df0882ba354e90df9c 673037c81a26f4254c66c0a286a0a879239fd7d01033fb05f97b32c2341f354c 1dbeead7aba2f62eb95acb2f5b2ea19031a5b62c1d81252079399bd33c331d72' \
    '6b5417d94ff17349188bc189ab996948ff58fee26dc16c3cdae432e347688ad9 c7a158a6d534509728051e5740a4f71f18e0dc8f7f285cf8038643c8f0162157 ffffffffffffffffffffffffffffffffffffffff 673037c81a26f4254c66c0a286a0a879239fd7d01033fb05f97b32c2341f354c 1dbeead7aba2f62eb95acb2f5b2ea19031a5b62c1d81252079399bd33c331d72' \
    '6b5417d94ff17349188bc189ab996948ff58fee26dc16c3cdae432e347688ad9 c7a158a6d534509728051e5740a4f71f18e0dc8f7f285cf8038643c8f0162157 d3012fbb64954274aa4778df0882ba354e90df9c ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff 1dbeead7aba2f62eb95acb2f5b2ea19031a5b62c1d81252079399bd33c331d72' \
    '6b5417d94ff17349188bc189ab996948ff58fee26dc16c3cdae432e347688ad9 c7a158a6d534509728051e5740a4f71f18e0dc8f7f285cf8038643c8f0162157 d3012fbb64954274aa4778df0882ba354e90df9c 673037c81a26f4254c66c0a286a0a879239fd7d01033fb05f97b32c2341f354c ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff'
do
    # Intentional field splitting exercises each exact five-field authority.
    # shellcheck disable=SC2086
    expect_failure manifest_tool_authority_is_accepted $rejected_authority
done

exec 9< "$bundle/bin/robin-highscores-manifestctl"
run_validator() {
    "$validator" "$bundle" "$commit" "$sums_sha256" /proc/self/fd/9
}

sums_sha256=$(refresh_sums)
run_validator >/dev/null

for script in deploy-release.sh rollback-release.sh; do
    valid_script=$bundle/deploy/$script
    cp -- "$valid_script" "$temporary_root/$script.valid"
    for adversarial_assignment in \
        'opt_root=$expected_home/.local/opt/robin-highscores' \
        'opt_root=$HOME/.local/opt/robin-highscores' \
        '    opt_root=$expected_home/.local/opt/robin-highscores' \
        'missing' \
        'opt_root=/opt/robin-highscores'
    do
        if [ "$adversarial_assignment" = missing ]; then
            sed '/^opt_root=/d' "$temporary_root/$script.valid" > "$valid_script"
        else
            sed "s|^opt_root=.*|$adversarial_assignment|" \
                "$temporary_root/$script.valid" > "$valid_script"
        fi
        chmod 0750 -- "$valid_script"
        sums_sha256=$(refresh_sums)
        expect_failure run_validator
    done
    cp -- "$temporary_root/$script.valid" "$valid_script"
    chmod 0750 -- "$valid_script"
    sums_sha256=$(refresh_sums)
    run_validator >/dev/null
done

printf 'nested checksum file\n' > "$bundle/config/SHA256SUMS"
chmod 0640 -- "$bundle/config/SHA256SUMS"
expect_failure run_validator
sums_sha256=$(refresh_sums)
run_validator >/dev/null
rm -f -- "$bundle/config/SHA256SUMS"
sums_sha256=$(refresh_sums)

chmod 0640 -- "$bundle/config/api.env"
printf 'RUST_LOG=debug\n' > "$bundle/config/api.env"
chmod 0640 -- "$bundle/config/api.env"
expect_failure run_validator
printf 'RUST_LOG=info\n' > "$bundle/config/api.env"
chmod 0640 -- "$bundle/config/api.env"
sums_sha256=$(refresh_sums)

chmod 0640 -- "$bundle/config/highscores-worker.toml"
cp -- "$bundle/config/highscores-worker.toml" "$temporary_root/worker-config.valid"
printf 'broker_socket = "/run/obsolete.sock"\n' >> "$bundle/config/highscores-worker.toml"
chmod 0640 -- "$bundle/config/highscores-worker.toml"
sums_sha256=$(refresh_sums)
expect_failure run_validator
cp -- "$temporary_root/worker-config.valid" "$bundle/config/highscores-worker.toml"
chmod 0640 -- "$bundle/config/highscores-worker.toml"

api_unit=$bundle/systemd/user/robin-highscores-api.service
cp -- "$api_unit" "$temporary_root/api.valid"
sed "\|^ReadOnlyPaths=$state_root/status$|d" "$temporary_root/api.valid" > "$api_unit"
chmod 0640 -- "$api_unit"
sums_sha256=$(refresh_sums)
expect_failure run_validator
cp -- "$temporary_root/api.valid" "$api_unit"
chmod 0640 -- "$api_unit"

worker_unit=$bundle/systemd/user/robin-highscores-worker.service
cp -- "$worker_unit" "$temporary_root/worker.valid"
sed "s|^InaccessiblePaths=$state_root/status$|ReadOnlyPaths=$state_root/status|" \
    "$temporary_root/worker.valid" > "$worker_unit"
chmod 0640 -- "$worker_unit"
sums_sha256=$(refresh_sums)
expect_failure run_validator
cp -- "$temporary_root/worker.valid" "$worker_unit"
chmod 0640 -- "$worker_unit"

backup_unit=$bundle/systemd/user/robin-highscores-backup.service
cp -- "$backup_unit" "$temporary_root/backup.valid"
sed 's|--restore-source-map /home/robinhood/.config/systemd/user/robin-highscores.target=/home/robinhood/.config/systemd/user/robin-highscores.target|--restore-source-map /home/robinhood/.config/systemd/user=/home/robinhood/.config/systemd/user|' \
    "$temporary_root/backup.valid" > "$backup_unit"
chmod 0640 -- "$backup_unit"
sums_sha256=$(refresh_sums)
expect_failure run_validator
sed "s|--restore-source-map /home/robinhood/.config/systemd/user/robin-highscores.target=/home/robinhood/.config/systemd/user/robin-highscores.target|--restore-source-map $release_root=$release_root|" \
    "$temporary_root/backup.valid" > "$backup_unit"
chmod 0640 -- "$backup_unit"
sums_sha256=$(refresh_sums)
expect_failure run_validator
cp -- "$temporary_root/backup.valid" "$backup_unit"
chmod 0640 -- "$backup_unit"

server_config=$bundle/config/highscores-server.toml
cp -- "$server_config" "$temporary_root/server.valid"
sed "s|$release_root/vps-release-manifest-v2.json|/home/robinhood/.local/opt/robin-highscores/releases/ffffffffffffffffffffffffffffffffffffffff/vps-release-manifest-v2.json|" \
    "$temporary_root/server.valid" > "$server_config"
chmod 0640 -- "$server_config"
sums_sha256=$(refresh_sums)
expect_failure run_validator
cp -- "$temporary_root/server.valid" "$server_config"
chmod 0640 -- "$server_config"
sums_sha256=$(refresh_sums)
run_validator >/dev/null

echo "deploy shell tests passed"
