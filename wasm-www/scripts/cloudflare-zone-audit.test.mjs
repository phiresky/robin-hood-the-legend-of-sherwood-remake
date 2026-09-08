import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import test from 'node:test';
import {
    approveCloudflareZoneAudit,
    captureCloudflareZoneSnapshot,
    validateCloudflareZoneAudit,
} from './cloudflare-zone-audit.mjs';

const accountId = 'ab'.repeat(16);
const zoneId = 'cd'.repeat(16);
const token = 'test-token-that-is-long-enough';
const publicDnsId = '11'.repeat(16);
const routeId = '22'.repeat(16);
const rulesetId = '33'.repeat(16);
const pageRuleId = '44'.repeat(16);

function sorted(value) {
    if (Array.isArray(value)) return value.map(sorted);
    if (value !== null && typeof value === 'object') {
        return Object.fromEntries(Object.entries(value).sort(([left], [right]) => left.localeCompare(right))
            .map(([key, item]) => [key, sorted(item)]));
    }
    return value;
}

function bytes(value) {
    return Buffer.from(`${JSON.stringify(sorted(value))}\n`);
}

function response(result, { ok = true, status = 200, totalPages = undefined } = {}) {
    return {
        ok,
        status,
        async json() {
            return {
                errors: ok ? [] : [{ message: 'permission denied' }],
                ...(totalPages === undefined ? {} : { result_info: { total_pages: totalPages } }),
                result,
                success: ok,
            };
        },
    };
}

function api({ changedRoute = false, denyRules = false } = {}) {
    return async url => {
        const parsed = new URL(url);
        const path = parsed.pathname;
        if (path === '/client/v4/user/tokens/verify') {
            return response({ status: 'active' });
        }
        if (path === `/client/v4/zones/${zoneId}`) {
            return response({ id: zoneId, name: 'phiresky.xyz', status: 'active' });
        }
        if (path.endsWith('/dns_records')) {
            const name = parsed.searchParams.get('name');
            return response(name === 'robinhood.phiresky.xyz' ? [{
                content: '65.109.224.29',
                id: publicDnsId,
                name,
                proxied: true,
                ttl: 1,
                type: 'A',
            }] : [], { totalPages: 1 });
        }
        if (path.endsWith('/workers/routes')) {
            return response([{
                id: routeId,
                pattern: 'robinhood.phiresky.xyz/*',
                script: changedRoute ? 'substituted-worker' : 'robinhood',
            }]);
        }
        if (path.endsWith('/pagerules')) {
            return response([{
                actions: [{ id: 'cache_level', value: 'bypass' }],
                id: pageRuleId,
                priority: 1,
                status: 'active',
                targets: [{ constraint: { operator: 'matches', value: '*example*' }, target: 'url' }],
            }], { totalPages: 1 });
        }
        if (path.endsWith('/rulesets')) {
            if (denyRules) return response(null, { ok: false, status: 403 });
            return response([{ id: rulesetId, phase: 'http_request_cache_settings' }], { totalPages: 1 });
        }
        if (path.endsWith(`/rulesets/${rulesetId}`)) {
            return response({
                id: rulesetId,
                kind: 'zone',
                name: 'cache',
                phase: 'http_request_cache_settings',
                rules: [{ action: 'set_cache_settings', enabled: true, expression: 'false' }],
                version: '1',
            });
        }
        throw new Error(`unexpected Cloudflare URL ${url}`);
    };
}

test('captures DNS, routes, active Page Rules, and complete cache Rulesets', async () => {
    const snapshot = await captureCloudflareZoneSnapshot({
        accountId,
        apiToken: token,
        fetchImpl: api(),
        zoneId,
    });
    assert.equal(snapshot.dns_records[0].content, '65.109.224.29');
    assert.equal(snapshot.worker_routes[0].script, 'robinhood');
    assert.equal(snapshot.active_page_rules[0].id, pageRuleId);
    assert.equal(snapshot.active_cache_rulesets[0].id, rulesetId);
});

test('approved audit is digest-bound and rejects any live drift before deployment', async t => {
    const temporary = await mkdtemp(resolve(tmpdir(), 'cloudflare-zone-audit-'));
    t.after(() => rm(temporary, { force: true, recursive: true }));
    const snapshot = await captureCloudflareZoneSnapshot({ accountId, apiToken: token, fetchImpl: api(), zoneId });
    const audit = {
        approval: {
            api_cache_safe: true,
            expected_api_origin_addresses: ['65.109.224.29'],
            reviewed_at: '2026-08-31T12:00:00.000Z',
            reviewed_by: 'operator',
            retire_public_routes: ['robinhood.phiresky.xyz/api/*'],
        },
        schema_version: 1,
        snapshot,
    };
    const auditBytes = bytes(audit);
    const auditPath = resolve(temporary, 'audit.json');
    await writeFile(auditPath, auditBytes);
    const expectedAuditSha256 = createHash('sha256').update(auditBytes).digest('hex');
    const verified = await validateCloudflareZoneAudit({
        accountId,
        apiToken: token,
        auditPath,
        expectedAuditSha256,
        fetchImpl: api(),
        zoneId,
    });
    assert.equal(verified.auditSha256, expectedAuditSha256);
    await assert.rejects(validateCloudflareZoneAudit({
        accountId,
        apiToken: token,
        auditPath,
        expectedAuditSha256,
        fetchImpl: api({ changedRoute: true }),
        zoneId,
    }), /differ from the approved zone audit/u);
});

test('audit fails closed when cache Ruleset read permission is absent', async () => {
    await assert.rejects(captureCloudflareZoneSnapshot({
        accountId,
        apiToken: token,
        fetchImpl: api({ denyRules: true }),
        zoneId,
    }), /Ruleset audit failed \(403\): permission denied/u);
});

test('audit reports the precise account-owned token blocker before Page Rules', async () => {
    const fetchImpl = async url => {
        const path = new URL(url).pathname;
        if (path === '/client/v4/user/tokens/verify') return response(null, { ok: false, status: 403 });
        if (path === `/client/v4/accounts/${accountId}/tokens/verify`) return response({ status: 'active' });
        throw new Error(`unexpected URL ${url}`);
    };
    await assert.rejects(captureCloudflareZoneSnapshot({
        accountId,
        apiToken: token,
        fetchImpl,
        zoneId,
    }), /account-owned.*Page Rules.*code 1011.*My Profile > API Tokens/u);
});

test('approval turns an untouched capture into canonical digest-bound operator input', async t => {
    const temporary = await mkdtemp(resolve(tmpdir(), 'cloudflare-zone-approval-'));
    t.after(() => rm(temporary, { force: true, recursive: true }));
    const snapshot = await captureCloudflareZoneSnapshot({ accountId, apiToken: token, fetchImpl: api(), zoneId });
    const draftPath = resolve(temporary, 'draft.json');
    const output = resolve(temporary, 'approved.json');
    await writeFile(draftPath, bytes({
        approval: {
            api_cache_safe: false,
            expected_api_origin_addresses: [],
            reviewed_at: '',
            reviewed_by: '',
            retire_public_routes: [],
        },
        schema_version: 1,
        snapshot,
    }));
    const approved = await approveCloudflareZoneAudit({
        draftPath,
        expectedApiOriginAddresses: ['65.109.224.29'],
        output,
        retirePublicRoutes: ['robinhood.phiresky.xyz/api/*'],
        reviewedAt: '2026-08-31T12:00:00.000Z',
        reviewedBy: 'operator',
    });
    assert.equal(approved.audit.approval.api_cache_safe, true);
    assert.match(approved.auditSha256, /^[0-9a-f]{64}$/u);
});
