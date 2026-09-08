import assert from 'node:assert/strict';
import test from 'node:test';
import { filtersToApiQuery, routeFromUrl, urlWithFilters, urlWithPlayerCursor } from './state.js';

const digest = 'a'.repeat(64);

test('leaderboard URL parsing has stable public defaults', () => {
    assert.deepEqual(routeFromUrl('https://pages.example/leaderboards/?api=https://api.example/api/v1'), {
        kind: 'leaderboard',
        filters: {
            subject: 'individual_level',
            metric: 'original_score',
            missionId: null,
            presetId: null,
            difficultyId: null,
            rulesetId: null,
            contentIdentitySha256: null,
            rulesConfigSha256: null,
            competitionManifestSha256: null,
            maxConcurrentPlayers: null,
            cursor: null,
        },
    });
});

test('run routes are public and private submission status is rejected', () => {
    assert.deepEqual(routeFromUrl('https://pages.example/leaderboards/?run=run_42'), { kind: 'run', id: 'run_42' });
    assert.throws(
        () => routeFromUrl('https://pages.example/leaderboards/?submission=sub_7'),
        /authenticated owner client/u,
    );
    assert.throws(() => routeFromUrl('https://pages.example/leaderboards/?run=%20bad'), /invalid/u);
});

test('full-campaign session routes require an aggregate id and gap-free ordinal', () => {
    assert.deepEqual(
        routeFromUrl('https://pages.example/leaderboards/?run=aggregate_1&session=0'),
        { kind: 'campaign_session', aggregateRunId: 'aggregate_1', ordinal: 0 },
    );
    assert.throws(
        () => routeFromUrl('https://pages.example/leaderboards/?session=1'),
        /requires its aggregate/u,
    );
    assert.throws(
        () => routeFromUrl('https://pages.example/leaderboards/?run=aggregate_1&session=-1'),
        /non-negative/u,
    );
});

test('player routes round-trip an opaque history cursor without losing loopback API configuration', () => {
    assert.deepEqual(
        routeFromUrl(`https://pages.example/leaderboards/?player=${digest}&cursor=page%2B2`),
        { kind: 'player', publicKey: digest, cursor: 'page+2' },
    );
    const next = new URL(urlWithPlayerCursor(
        `http://localhost:5173/leaderboards/?api=https%3A%2F%2Fscores.example%2Fapi%2Fv1&run=old`,
        digest,
        'page+2',
    ));
    assert.equal(next.searchParams.get('api'), 'https://scores.example/api/v1');
    assert.equal(next.searchParams.get('run'), null);
    assert.equal(next.searchParams.get('player'), digest);
    assert.equal(next.searchParams.get('cursor'), 'page+2');
    assert.throws(
        () => routeFromUrl(`https://pages.example/leaderboards/?player=${digest}&cursor=bad%0Acursor`),
        /cursor is invalid/u,
    );
});

test('filter serialization preserves API configuration and canonical query vocabulary', () => {
    const filters = {
        subject: 'campaign' as const,
        metric: 'fastest_success' as const,
        missionId: 'nottingham',
        presetId: 'fresh_features',
        difficultyId: 'legendary',
        rulesetId: digest,
        contentIdentitySha256: digest,
        rulesConfigSha256: digest,
        competitionManifestSha256: 'b'.repeat(64),
        maxConcurrentPlayers: 2,
        cursor: 'cursor+opaque',
    };
    const url = new URL(urlWithFilters('https://pages.example/leaderboards/?api=https%3A%2F%2Fapi.example%2Fapi%2Fv1&run=old', filters));
    assert.equal(url.searchParams.get('api'), 'https://api.example/api/v1');
    assert.equal(url.searchParams.get('run'), null);
    assert.equal(url.searchParams.get('competition'), 'b'.repeat(64));

    assert.deepEqual(filtersToApiQuery(filters), {
        schema_version: 1,
        mission_id: 'nottingham',
        subject_kind: 'mission',
        mission_scope: 'campaign',
        metric: 'fastest_success',
        content_identity_sha256: digest,
        rules_config_sha256: digest,
        ruleset_manifest_sha256: digest,
        competition_manifest_sha256: 'b'.repeat(64),
        max_concurrent_players: 2,
        limit: 25,
        cursor: 'cursor+opaque',
    });
});

test('full campaign query forbids mission fields and remembered subject is used only as a URL default', () => {
    const route = routeFromUrl('https://pages.example/leaderboards/', 'full_campaign');
    assert.equal(route.kind === 'leaderboard' ? route.filters.subject : null, 'full_campaign');
    if (route.kind !== 'leaderboard') throw new Error('expected leaderboard route');
    const query = filtersToApiQuery({
        ...route.filters,
        metric: 'original_score',
        rulesetId: digest,
        contentIdentitySha256: digest,
        rulesConfigSha256: digest,
    });
    assert.equal(query.subject_kind, 'full_campaign');
    assert.equal(query.mission_id, null);
    assert.equal(query.mission_scope, null);
    assert.equal(routeFromUrl('https://pages.example/leaderboards/?subject=campaign', 'full_campaign').kind, 'leaderboard');
});

test('Worker-hosted leaderboard paths survive canonical query navigation and direct reload links', () => {
    const input = 'https://robinhood.phiresky.xyz/leaderboards/?run=run_42';
    assert.deepEqual(routeFromUrl(input), { kind: 'run', id: 'run_42' });

    const route = routeFromUrl('https://robinhood.phiresky.xyz/leaderboards/');
    if (route.kind !== 'leaderboard') throw new Error('expected leaderboard route');
    const output = new URL(urlWithFilters(
        'https://robinhood.phiresky.xyz/leaderboards/',
        route.filters,
    ));
    assert.equal(output.pathname, '/leaderboards/');
    assert.equal(output.searchParams.get('subject'), 'individual_level');
});
