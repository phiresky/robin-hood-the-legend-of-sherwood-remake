import { createHash } from 'node:crypto';
import { readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const HEX_32 = /^[0-9a-f]{32}$/u;
const DIGEST = /^[0-9a-f]{64}$/u;
const HOST = /^(?:[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?\.)+[a-z]{2,63}$/u;
const PUBLIC_HOST = 'robinhood.phiresky.xyz';
const SIGNER_HOST = 'identity.robinhood.phiresky.xyz';
const ZONE_NAME = 'phiresky.xyz';

function utf8Order(left, right) {
    return Buffer.compare(Buffer.from(left, 'utf8'), Buffer.from(right, 'utf8'));
}

function canonical(value) {
    if (Array.isArray(value)) return value.map(canonical);
    if (value !== null && typeof value === 'object') {
        return Object.fromEntries(Object.entries(value)
            .sort(([left], [right]) => utf8Order(left, right))
            .map(([key, item]) => [key, canonical(item)]));
    }
    return value;
}

function canonicalBytes(value) {
    return Buffer.from(`${JSON.stringify(canonical(value))}\n`);
}

function sha256(bytes) {
    return createHash('sha256').update(bytes).digest('hex');
}

function exactKeys(value, keys, label) {
    if (value === null || typeof value !== 'object' || Array.isArray(value)) {
        throw new Error(`${label} must be an object`);
    }
    const actual = Object.keys(value).sort(utf8Order);
    const expected = [...keys].sort(utf8Order);
    if (JSON.stringify(actual) !== JSON.stringify(expected)) {
        throw new Error(`${label} has unexpected keys: ${actual.join(', ')}`);
    }
}

async function envelope(response, operation) {
    let body;
    try {
        body = await response.json();
    } catch {
        throw new Error(`Cloudflare ${operation} returned non-JSON status ${response.status}`);
    }
    if (!response.ok || body?.success !== true) {
        const messages = Array.isArray(body?.errors)
            ? body.errors.map(item => item?.message).filter(Boolean).join('; ')
            : '';
        throw new Error(`Cloudflare ${operation} failed (${response.status})${messages === '' ? '' : `: ${messages}`}`);
    }
    return body;
}

async function request(fetchImpl, apiToken, path, operation) {
    const response = await fetchImpl(`https://api.cloudflare.com/client/v4${path}`, {
        headers: { authorization: `Bearer ${apiToken}` },
        method: 'GET',
        redirect: 'error',
        signal: AbortSignal.timeout(30_000),
    });
    return envelope(response, operation);
}

export async function requireUserOwnedCloudflareToken({ accountId, apiToken, fetchImpl = fetch }) {
    const options = {
        headers: { authorization: `Bearer ${apiToken}` },
        method: 'GET',
        redirect: 'error',
        signal: AbortSignal.timeout(30_000),
    };
    const userResponse = await fetchImpl('https://api.cloudflare.com/client/v4/user/tokens/verify', options);
    const userBody = await userResponse.json().catch(() => undefined);
    if (userResponse.ok && userBody?.success === true && userBody.result?.status === 'active') return;
    const accountResponse = await fetchImpl(
        `https://api.cloudflare.com/client/v4/accounts/${accountId}/tokens/verify`,
        options,
    );
    const accountBody = await accountResponse.json().catch(() => undefined);
    if (accountResponse.ok && accountBody?.success === true && accountBody.result?.status === 'active') {
        throw new Error('Cloudflare token is account-owned; the Page Rules API rejects account-owned tokens (code 1011). Create a user-owned token under My Profile > API Tokens');
    }
    throw new Error('Cloudflare user-owned API token is invalid or inactive');
}

async function paginated(fetchImpl, apiToken, path, operation) {
    const values = [];
    for (let page = 1; ; page += 1) {
        const separator = path.includes('?') ? '&' : '?';
        const envelopeValue = await request(
            fetchImpl,
            apiToken,
            `${path}${separator}per_page=50&page=${page}`,
            operation,
        );
        if (!Array.isArray(envelopeValue.result)) {
            throw new Error(`Cloudflare ${operation} result is not an array`);
        }
        values.push(...envelopeValue.result);
        const totalPages = envelopeValue.result_info?.total_pages ?? 1;
        if (!Number.isSafeInteger(totalPages) || totalPages < 1 || totalPages > 100) {
            throw new Error(`Cloudflare ${operation} has invalid pagination`);
        }
        if (page >= totalPages) return values;
    }
}

function normalizeRoute(route) {
    if (route === null || typeof route !== 'object'
        || !HEX_32.test(route.id)
        || typeof route.pattern !== 'string') {
        throw new Error('Cloudflare returned a malformed Worker route');
    }
    return canonical({ id: route.id, pattern: route.pattern, script: route.script ?? null });
}

function normalizeDns(record) {
    if (record === null || typeof record !== 'object'
        || !HEX_32.test(record.id)
        || typeof record.name !== 'string'
        || typeof record.type !== 'string'
        || typeof record.content !== 'string'
        || typeof record.proxied !== 'boolean'
        || !Number.isSafeInteger(record.ttl)) {
        throw new Error('Cloudflare returned a malformed DNS record');
    }
    return canonical({
        content: record.content,
        id: record.id,
        name: record.name,
        proxied: record.proxied,
        ttl: record.ttl,
        type: record.type,
    });
}

function normalizePageRule(rule) {
    if (rule === null || typeof rule !== 'object'
        || !HEX_32.test(rule.id)
        || !Number.isSafeInteger(rule.priority)
        || rule.status !== 'active'
        || !Array.isArray(rule.targets)
        || !Array.isArray(rule.actions)) {
        throw new Error('Cloudflare returned a malformed active Page Rule');
    }
    return canonical({
        actions: rule.actions,
        id: rule.id,
        priority: rule.priority,
        status: rule.status,
        targets: rule.targets,
    });
}

function normalizeRuleset(ruleset) {
    if (ruleset === null || typeof ruleset !== 'object'
        || !HEX_32.test(ruleset.id)
        || typeof ruleset.kind !== 'string'
        || typeof ruleset.name !== 'string'
        || ruleset.phase !== 'http_request_cache_settings'
        || !Array.isArray(ruleset.rules)) {
        throw new Error('Cloudflare returned a malformed cache Ruleset');
    }
    return canonical({
        id: ruleset.id,
        kind: ruleset.kind,
        name: ruleset.name,
        phase: ruleset.phase,
        rules: ruleset.rules,
        version: String(ruleset.version),
    });
}

export async function captureCloudflareZoneSnapshot({
    accountId,
    zoneId,
    apiToken,
    fetchImpl = fetch,
}) {
    if (!HEX_32.test(accountId) || !HEX_32.test(zoneId)) {
        throw new Error('Cloudflare account and zone IDs must be 32 lowercase hexadecimal characters');
    }
    if (typeof apiToken !== 'string' || apiToken.length < 20) {
        throw new Error('Cloudflare API token is missing or malformed');
    }
    await requireUserOwnedCloudflareToken({ accountId, apiToken, fetchImpl });
    const zone = await request(fetchImpl, apiToken, `/zones/${zoneId}`, 'zone lookup');
    if (zone.result?.id !== zoneId || zone.result?.name !== ZONE_NAME || zone.result?.status !== 'active') {
        throw new Error(`Cloudflare zone must be the active ${ZONE_NAME} zone`);
    }
    const dns = await paginated(
        fetchImpl,
        apiToken,
        `/zones/${zoneId}/dns_records?name=${PUBLIC_HOST}`,
        'public-host DNS audit',
    );
    const signerDns = await paginated(
        fetchImpl,
        apiToken,
        `/zones/${zoneId}/dns_records?name=${SIGNER_HOST}`,
        'signer-host DNS audit',
    );
    const routeEnvelope = await request(fetchImpl, apiToken, `/zones/${zoneId}/workers/routes`, 'Worker route audit');
    if (!Array.isArray(routeEnvelope.result)) throw new Error('Cloudflare Worker route audit is not an array');
    const pageRules = await paginated(
        fetchImpl,
        apiToken,
        `/zones/${zoneId}/pagerules?status=active`,
        'active Page Rule audit',
    );
    const rulesetList = await paginated(fetchImpl, apiToken, `/zones/${zoneId}/rulesets`, 'Ruleset audit');
    const cacheRulesets = [];
    for (const summary of rulesetList) {
        if (summary?.phase !== 'http_request_cache_settings') continue;
        if (!HEX_32.test(summary.id)) throw new Error('Cloudflare returned a cache Ruleset with a malformed ID');
        const detail = await request(
            fetchImpl,
            apiToken,
            `/zones/${zoneId}/rulesets/${summary.id}`,
            `cache Ruleset ${summary.id} audit`,
        );
        cacheRulesets.push(normalizeRuleset(detail.result));
    }
    const byId = (left, right) => utf8Order(left.id, right.id);
    return canonical({
        account_id: accountId,
        active_cache_rulesets: cacheRulesets.sort(byId),
        active_page_rules: pageRules.map(normalizePageRule).sort((left, right) => left.priority - right.priority || byId(left, right)),
        dns_records: [...dns, ...signerDns].map(normalizeDns).sort(byId),
        worker_routes: routeEnvelope.result.map(normalizeRoute).sort((left, right) => utf8Order(left.pattern, right.pattern)),
        zone_id: zoneId,
        zone_name: ZONE_NAME,
    });
}

function validateApproval(approval) {
    exactKeys(approval, [
        'api_cache_safe',
        'expected_api_origin_addresses',
        'reviewed_at',
        'reviewed_by',
        'retire_public_routes',
    ], 'zone audit approval');
    if (approval.api_cache_safe !== true) throw new Error('zone audit must explicitly approve the complete /api* prefix as cache-safe');
    if (!Array.isArray(approval.expected_api_origin_addresses)
        || approval.expected_api_origin_addresses.length === 0
        || approval.expected_api_origin_addresses.some(value => typeof value !== 'string' || value.length === 0)) {
        throw new Error('zone audit must bind at least one expected API origin address');
    }
    if (!Array.isArray(approval.retire_public_routes)
        || approval.retire_public_routes.some(value => typeof value !== 'string' || !value.startsWith(`${PUBLIC_HOST}/`))) {
        throw new Error('zone audit route retirements must be exact public-host route patterns');
    }
    if (new Set(approval.retire_public_routes).size !== approval.retire_public_routes.length) {
        throw new Error('zone audit has duplicate route retirements');
    }
    if (typeof approval.reviewed_by !== 'string' || approval.reviewed_by.trim() === '') {
        throw new Error('zone audit reviewer is missing');
    }
    if (typeof approval.reviewed_at !== 'string' || Number.isNaN(Date.parse(approval.reviewed_at))) {
        throw new Error('zone audit review time is invalid');
    }
}

export async function validateCloudflareZoneAudit({
    auditPath,
    expectedAuditSha256,
    accountId,
    zoneId,
    apiToken,
    fetchImpl = fetch,
}) {
    if (!DIGEST.test(expectedAuditSha256)) throw new Error('expected zone audit SHA-256 is invalid');
    const bytes = await readFile(auditPath);
    if (sha256(bytes) !== expectedAuditSha256) throw new Error('zone audit SHA-256 mismatch');
    let audit;
    try {
        audit = JSON.parse(bytes.toString('utf8'));
    } catch (error) {
        throw new Error(`zone audit is not JSON: ${error instanceof Error ? error.message : String(error)}`);
    }
    exactKeys(audit, ['approval', 'schema_version', 'snapshot'], 'zone audit');
    if (audit.schema_version !== 1) throw new Error('zone audit schema must be 1');
    if (!bytes.equals(canonicalBytes(audit))) throw new Error('zone audit must be canonical JSON');
    validateApproval(audit.approval);
    if (audit.snapshot?.account_id !== accountId || audit.snapshot?.zone_id !== zoneId) {
        throw new Error('zone audit account/zone differs from operator credentials');
    }
    const publicAddresses = audit.snapshot.dns_records
        ?.filter(record => record.name === PUBLIC_HOST && ['A', 'AAAA'].includes(record.type) && record.proxied)
        .map(record => record.content) ?? [];
    if (JSON.stringify([...publicAddresses].sort(utf8Order))
        !== JSON.stringify([...audit.approval.expected_api_origin_addresses].sort(utf8Order))) {
        throw new Error('public proxied DNS records differ from the approved API origin addresses');
    }
    const current = await captureCloudflareZoneSnapshot({ accountId, zoneId, apiToken, fetchImpl });
    if (JSON.stringify(current) !== JSON.stringify(audit.snapshot)) {
        throw new Error('live Cloudflare DNS/routes/cache Rulesets/Page Rules differ from the approved zone audit');
    }
    return { approval: audit.approval, auditSha256: expectedAuditSha256, snapshot: current };
}

export async function approveCloudflareZoneAudit({
    draftPath,
    output,
    reviewedBy,
    expectedApiOriginAddresses,
    retirePublicRoutes,
    reviewedAt = new Date().toISOString(),
}) {
    const draftBytes = await readFile(draftPath);
    let draft;
    try { draft = JSON.parse(draftBytes.toString('utf8')); } catch (error) {
        throw new Error(`zone audit draft is not JSON: ${error instanceof Error ? error.message : String(error)}`);
    }
    exactKeys(draft, ['approval', 'schema_version', 'snapshot'], 'zone audit draft');
    if (draft.schema_version !== 1 || !draftBytes.equals(canonicalBytes(draft))) {
        throw new Error('zone audit draft must be the untouched canonical capture');
    }
    const audit = canonical({
        approval: {
            api_cache_safe: true,
            expected_api_origin_addresses: expectedApiOriginAddresses,
            retire_public_routes: retirePublicRoutes,
            reviewed_at: reviewedAt,
            reviewed_by: reviewedBy,
        },
        schema_version: 1,
        snapshot: draft.snapshot,
    });
    validateApproval(audit.approval);
    const bytes = canonicalBytes(audit);
    await writeFile(output, bytes, { flag: 'wx', mode: 0o600 });
    return { audit, auditSha256: sha256(bytes) };
}

async function main() {
    const [command, ...args] = process.argv.slice(2);
    if (command === 'capture' && args.length === 1) {
        const [output] = args;
        const snapshot = await captureCloudflareZoneSnapshot({
            accountId: process.env.CLOUDFLARE_ACCOUNT_ID,
            zoneId: process.env.CLOUDFLARE_ZONE_ID,
            apiToken: process.env.CLOUDFLARE_API_TOKEN,
        });
        const draft = canonical({
            approval: {
                api_cache_safe: false,
                expected_api_origin_addresses: [],
                reviewed_at: '',
                reviewed_by: '',
                retire_public_routes: [],
            },
            schema_version: 1,
            snapshot,
        });
        await writeFile(output, canonicalBytes(draft), { flag: 'wx', mode: 0o600 });
        console.log(`captured private Cloudflare zone audit draft ${resolve(output)}`);
        return;
    }
    if (command === 'verify' && args.length === 2) {
        const [auditPath, expectedAuditSha256] = args;
        await validateCloudflareZoneAudit({
            accountId: process.env.CLOUDFLARE_ACCOUNT_ID,
            apiToken: process.env.CLOUDFLARE_API_TOKEN,
            auditPath,
            expectedAuditSha256,
            zoneId: process.env.CLOUDFLARE_ZONE_ID,
        });
        console.log(`verified Cloudflare zone audit ${expectedAuditSha256}`);
        return;
    }
    if (command === 'approve' && args.length === 5) {
        const [draftPath, output, reviewedBy, addresses, retirements] = args;
        const result = await approveCloudflareZoneAudit({
            draftPath,
            expectedApiOriginAddresses: addresses.split(',').filter(Boolean),
            output,
            retirePublicRoutes: retirements === '-' ? [] : retirements.split(',').filter(Boolean),
            reviewedBy,
        });
        console.log(`approved private Cloudflare zone audit ${result.auditSha256}`);
        return;
    }
    throw new Error('usage: cloudflare-zone-audit.mjs capture OUTPUT | approve DRAFT OUTPUT REVIEWER API_ADDRESS_CSV RETIRE_PATTERN_CSV_OR_DASH | verify AUDIT EXPECTED_SHA256');
}

const invokedPath = process.argv[1];
if (invokedPath !== undefined && import.meta.url === pathToFileURL(resolve(invokedPath)).href) {
    main().catch(error => {
        console.error(error instanceof Error ? error.message : String(error));
        process.exitCode = 1;
    });
}
