#!/bin/sh
set -eu

# The only privileged host step. It installs the stable nginx origin, obtains
# its certificate with the host's existing Certbot account, and enables the
# robinhood user's lingering systemd manager. Releases never invoke sudo.

usage() {
    echo "usage: $0 ABSOLUTE_ROOT_ONCE_KIT_DIR EXPECTED_ROOT_ONCE_SHA256SUMS_SHA256" >&2
    exit 64
}

fail() {
    echo "root-once setup failed: $*" >&2
    exit 1
}

[ "$#" -eq 2 ] || usage
kit_arg=$1
expected_sums_sha256=$2
case "$kit_arg" in /*) ;; *) fail "kit path must be absolute" ;; esac
case "$expected_sums_sha256" in *[!0-9a-f]*|'') fail "SHA256SUMS digest must be lowercase hexadecimal" ;; esac
[ "${#expected_sums_sha256}" -eq 64 ] || fail "SHA256SUMS digest must contain exactly 64 hexadecimal digits"
[ "$(id -u)" -eq 0 ] || fail "must run as root"
[ -d "$kit_arg" ] && [ ! -L "$kit_arg" ] || fail "kit is missing or symlinked"
kit=$(realpath -e -- "$kit_arg")

id robinhood >/dev/null 2>&1 || fail "required robinhood SSH account does not exist"
[ "$(getent passwd robinhood | cut -d: -f6)" = /home/robinhood ] ||
    fail "robinhood account has an unexpected home directory"
for command_name in nginx certbot curl loginctl systemctl sha256sum openssl; do
    command -v "$command_name" >/dev/null 2>&1 || fail "required command is unavailable: $command_name"
done

umask 077
work_directory=$(mktemp -d /etc/nginx/.robinhood-root-once.XXXXXX)
staged_manifest=$work_directory/ROOT_ONCE_SHA256SUMS
early_cleanup() {
    status=$?
    trap - EXIT HUP INT TERM
    rm -f -- "$staged_manifest"
    rmdir -- "$work_directory"
    exit "$status"
}
trap early_cleanup EXIT HUP INT TERM

for source_path in \
    ROOT_ONCE_SHA256SUMS \
    root-once.sh \
    nginx-robinhood-api.challenge.conf \
    nginx-robinhood-cloudflare-only.conf \
    nginx-robinhood-api.locations.conf \
    nginx-robinhood-api.vhost.conf
do
    [ -f "$kit/$source_path" ] && [ ! -L "$kit/$source_path" ] ||
        fail "required kit file is missing: $source_path"
done
install -o root -g root -m 0600 -- "$kit/ROOT_ONCE_SHA256SUMS" "$staged_manifest"
[ "$(sha256sum "$staged_manifest" | cut -d' ' -f1)" = "$expected_sums_sha256" ] ||
    fail "ROOT_ONCE_SHA256SUMS differs from the reviewed digest"
[ "$(wc -l < "$staged_manifest" | tr -d ' ')" -eq 5 ] ||
    fail "root-once manifest must contain exactly five entries"
for exact_entry in \
    nginx-robinhood-api.challenge.conf \
    nginx-robinhood-api.locations.conf \
    nginx-robinhood-api.vhost.conf \
    nginx-robinhood-cloudflare-only.conf \
    root-once.sh
do
    [ "$(awk -v wanted="$exact_entry" 'substr($0, 67) == wanted { count += 1 } END { print count + 0 }' "$staged_manifest")" -eq 1 ] ||
        fail "root-once manifest does not contain exactly one entry for $exact_entry"
done

expected_file_digest() {
    relative=$1
    digest=$(awk -v wanted="$relative" '
        length($0) >= 67 && substr($0, 65, 2) == "  " && substr($0, 67) == wanted {
            print substr($0, 1, 64)
        }
    ' "$staged_manifest")
    case "$digest" in *[!0-9a-f]*|'') fail "missing canonical digest for $relative" ;; esac
    [ "${#digest}" -eq 64 ] || fail "invalid canonical digest for $relative"
    printf '%s\n' "$digest"
}

challenge_relative=nginx-robinhood-api.challenge.conf
cloudflare_relative=nginx-robinhood-cloudflare-only.conf
locations_relative=nginx-robinhood-api.locations.conf
vhost_relative=nginx-robinhood-api.vhost.conf
challenge_digest=$(expected_file_digest "$challenge_relative")
cloudflare_digest=$(expected_file_digest "$cloudflare_relative")
locations_digest=$(expected_file_digest "$locations_relative")
vhost_digest=$(expected_file_digest "$vhost_relative")

domain=robinhood.phiresky.xyz
challenge_root=/var/lib/letsencrypt
challenge_directory=$challenge_root/.well-known/acme-challenge
snippet_directory=/etc/nginx/snippets
available_directory=/etc/nginx/sites-available
enabled_directory=/etc/nginx/sites-enabled
locations_destination=$snippet_directory/robinhood-api.locations.conf
cloudflare_destination=$snippet_directory/robinhood-cloudflare-only.conf
vhost_destination=$available_directory/$domain
enabled_destination=$enabled_directory/$domain

for directory in "$snippet_directory" "$available_directory" "$enabled_directory"; do
    [ -d "$directory" ] && [ ! -L "$directory" ] || fail "required nginx directory is missing or symlinked: $directory"
done
for destination in "$cloudflare_destination" "$locations_destination" "$vhost_destination"; do
    if [ -e "$destination" ] || [ -L "$destination" ]; then
        [ -f "$destination" ] && [ ! -L "$destination" ] ||
            fail "managed nginx destination is not a regular file: $destination"
    fi
done
if [ -e "$enabled_destination" ] || [ -L "$enabled_destination" ]; then
    [ -L "$enabled_destination" ] || fail "enabled nginx vhost path is not a symlink"
    [ "$(readlink -- "$enabled_destination")" = "../sites-available/$domain" ] ||
        fail "enabled nginx vhost symlink has an unexpected target"
fi

challenge_staged=$work_directory/challenge.conf
cloudflare_staged=$work_directory/cloudflare.conf
locations_staged=$work_directory/locations.conf
vhost_staged=$work_directory/vhost.conf
install -o root -g root -m 0644 -- "$kit/$challenge_relative" "$challenge_staged"
install -o root -g root -m 0644 -- "$kit/$cloudflare_relative" "$cloudflare_staged"
install -o root -g root -m 0644 -- "$kit/$locations_relative" "$locations_staged"
install -o root -g root -m 0644 -- "$kit/$vhost_relative" "$vhost_staged"
[ "$(sha256sum "$challenge_staged" | cut -d' ' -f1)" = "$challenge_digest" ] || fail "staged challenge vhost changed"
[ "$(sha256sum "$cloudflare_staged" | cut -d' ' -f1)" = "$cloudflare_digest" ] || fail "staged Cloudflare allowlist changed"
[ "$(sha256sum "$locations_staged" | cut -d' ' -f1)" = "$locations_digest" ] || fail "staged locations include changed"
[ "$(sha256sum "$vhost_staged" | cut -d' ' -f1)" = "$vhost_digest" ] || fail "staged final vhost changed"
trap - EXIT HUP INT TERM

old_vhost=$work_directory/old-vhost
old_cloudflare=$work_directory/old-cloudflare
old_locations=$work_directory/old-locations
had_vhost=0
had_cloudflare=0
had_locations=0
created_enabled=0
if [ -f "$vhost_destination" ]; then cp --preserve=all -- "$vhost_destination" "$old_vhost"; had_vhost=1; fi
if [ -f "$cloudflare_destination" ]; then cp --preserve=all -- "$cloudflare_destination" "$old_cloudflare"; had_cloudflare=1; fi
if [ -f "$locations_destination" ]; then cp --preserve=all -- "$locations_destination" "$old_locations"; had_locations=1; fi

vhost_new=$available_directory/.$domain.new-$$
cloudflare_new=$snippet_directory/.robinhood-cloudflare-only.conf.new-$$
locations_new=$snippet_directory/.robinhood-api.locations.conf.new-$$
for new_path in "$vhost_new" "$cloudflare_new" "$locations_new"; do
    [ ! -e "$new_path" ] && [ ! -L "$new_path" ] || fail "nginx temporary destination already exists: $new_path"
done
atomic_install() {
    source_file=$1
    temporary_file=$2
    destination_file=$3
    install -o root -g root -m 0644 -- "$source_file" "$temporary_file"
    mv -T -- "$temporary_file" "$destination_file"
}

restore_nginx() {
    if [ "$had_vhost" -eq 1 ]; then mv -T -- "$old_vhost" "$vhost_destination"; else rm -f -- "$vhost_destination"; fi
    if [ "$had_cloudflare" -eq 1 ]; then mv -T -- "$old_cloudflare" "$cloudflare_destination"; else rm -f -- "$cloudflare_destination"; fi
    if [ "$had_locations" -eq 1 ]; then mv -T -- "$old_locations" "$locations_destination"; else rm -f -- "$locations_destination"; fi
    if [ "$created_enabled" -eq 1 ]; then rm -f -- "$enabled_destination"; fi
    nginx -t >/dev/null 2>&1 && systemctl reload nginx >/dev/null 2>&1 || true
}

success=0
challenge_file=
probe_output=$work_directory/probe-output
probe_headers=$work_directory/probe-headers
cleanup() {
    status=$?
    trap - EXIT HUP INT TERM
    if [ -n "$challenge_file" ]; then rm -f -- "$challenge_file"; fi
    if [ "$status" -ne 0 ] && [ "$success" -eq 0 ]; then restore_nginx; fi
    rm -f -- "$staged_manifest" "$challenge_staged" "$cloudflare_staged" "$locations_staged" "$vhost_staged" \
        "$vhost_new" "$cloudflare_new" "$locations_new" \
        "$old_vhost" "$old_cloudflare" "$old_locations" "$probe_output" "$probe_headers"
    rmdir -- "$work_directory"
    exit "$status"
}
trap cleanup EXIT HUP INT TERM

install -d -o root -g root -m 0755 -- "$challenge_root" "$challenge_root/.well-known" "$challenge_directory"
if [ -s "/etc/letsencrypt/live/$domain/fullchain.pem" ] && [ -s "/etc/letsencrypt/live/$domain/privkey.pem" ]; then
    atomic_install "$cloudflare_staged" "$cloudflare_new" "$cloudflare_destination"
    atomic_install "$locations_staged" "$locations_new" "$locations_destination"
    atomic_install "$vhost_staged" "$vhost_new" "$vhost_destination"
else
    atomic_install "$challenge_staged" "$vhost_new" "$vhost_destination"
fi
if [ ! -L "$enabled_destination" ]; then
    ln -s -- "../sites-available/$domain" "$enabled_destination"
    created_enabled=1
fi
nginx -t
systemctl reload nginx

# Prove that the permanent Cloudflare/DNS route reaches this exact challenge
# vhost before asking the CA to issue anything.
challenge_token=robinhood-preflight-$(openssl rand -hex 16)
challenge_file=$challenge_directory/$challenge_token
printf '%s\n' "$challenge_token" > "$challenge_file"
chmod 0644 -- "$challenge_file"
curl --fail --silent --show-error --max-time 15 \
    "http://$domain/.well-known/acme-challenge/$challenge_token" > "$probe_output"
[ "$(sed -n '1p' "$probe_output")" = "$challenge_token" ] ||
    fail "public ACME challenge probe returned different content"
[ "$(wc -l < "$probe_output" | tr -d ' ')" -eq 1 ] ||
    fail "public ACME challenge probe returned extra content"
rm -f -- "$challenge_file"
challenge_file=

[ -n "$(find /etc/letsencrypt/accounts -type f -name regr.json -print -quit 2>/dev/null)" ] ||
    fail "no existing Certbot account found; register one deliberately before rerunning"
certbot certonly --non-interactive --agree-tos --keep-until-expiring \
    --webroot --webroot-path "$challenge_root" \
    --cert-name "$domain" \
    --domain "$domain" \
    --deploy-hook "systemctl reload nginx"
[ -s "/etc/letsencrypt/live/$domain/fullchain.pem" ] || fail "Certbot did not install the certificate chain"
[ -s "/etc/letsencrypt/live/$domain/privkey.pem" ] || fail "Certbot did not install the certificate key"

atomic_install "$cloudflare_staged" "$cloudflare_new" "$cloudflare_destination"
atomic_install "$locations_staged" "$locations_new" "$locations_destination"
atomic_install "$vhost_staged" "$vhost_new" "$vhost_destination"
nginx -t
systemctl reload nginx
# The API release is deliberately installed after this root-only bootstrap.
# Probe the TLS vhost directly so an absent upstream cannot make the stable
# nginx/certificate installation fail. Cloudflare route authority was already
# proven separately by the public HTTP-01 challenge above.
curl --silent --show-error --max-time 15 --dump-header "$probe_headers" \
    --output /dev/null --noproxy '*' --resolve "$domain:443:127.0.0.1" \
    "https://$domain/api" || fail "direct TLS API-origin probe failed"
grep -Eiq '^x-robinhood-origin:[[:space:]]*api-v1[[:space:]]*$' "$probe_headers" ||
    fail "direct /api probe did not reach the installed nginx origin"
loginctl enable-linger robinhood

success=1
echo "root-once setup complete: certificate, nginx API origin, and robinhood linger are active"
echo "future releases require only the unprivileged deploy-release.sh command"
