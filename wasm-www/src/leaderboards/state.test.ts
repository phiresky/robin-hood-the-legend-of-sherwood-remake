import assert from 'node:assert/strict';
import test from 'node:test';
import { filtersToApiQuery, routeFromUrl, urlWithFilters, urlWithPlayerCursor } from './state.js';

const digest = 'a'.repeat(64);

test('leaderboard URL parsing leaves unset board choices to published defaults', () => {
    assert.deepEqual(routeFromUrl('https://pages.example/leaderboards/?api=https://api.example/api/v1'), {
        kind: 'leaderboard',
        filters: { boardId: null, missionId: null, metric: null, maxConcurrentPlayers: null, cursor: null },
    });
    assert.deepEqual(routeFromUrl('https://pages.example/leaderboards/?board=demo-standard-normal&mission=Dem_Lei_MP&metric=fastest_success&players=2'), {
        kind: 'leaderboard',
        filters: { boardId: 'demo-standard-normal', missionId: 'Dem_Lei_MP', metric: 'fastest_success', maxConcurrentPlayers: 2, cursor: null },
    });
    assert.throws(() => routeFromUrl('https://pages.example/leaderboards/?metric=ransom'), /metric must be one of/u);
    assert.throws(() => routeFromUrl('https://pages.example/leaderboards/?players=5'), /seat range/u);
    assert.throws(() => routeFromUrl('https://pages.example/leaderboards/?board=%20bad'), /board is invalid/u);
});

test('run routes and minimal public submission routes preserve their identifiers', () => {
    assert.deepEqual(routeFromUrl('https://pages.example/leaderboards/?run=run_42'), { kind: 'run', id: 'run_42' });
    assert.deepEqual(
        routeFromUrl('https://pages.example/leaderboards/?submission=sub_7'),
        { kind: 'submission', id: 'sub_7' },
    );
    assert.throws(() => routeFromUrl('https://pages.example/leaderboards/?run=%20bad'), /invalid/u);
    assert.throws(() => routeFromUrl(`https://pages.example/leaderboards/?run=r&player=${digest}`), /more than one/u);
});

test('player routes round-trip an opaque history cursor without losing loopback API configuration', () => {
    assert.deepEqual(
        routeFromUrl(`https://pages.example/leaderboards/?player=${digest}&cursor=page%2B2`),
        { kind: 'player', publicKey: digest, cursor: 'page+2' },
    );
    const next = new URL(urlWithPlayerCursor(
        `http://localhost:5173/leaderboards/?api=https%3A%2F%2Fscores.example%2Fapi%2Fv1&run=old&board=b`,
        digest,
        'page+2',
    ));
    assert.equal(next.searchParams.get('api'), 'https://scores.example/api/v1');
    assert.equal(next.searchParams.get('run'), null);
    assert.equal(next.searchParams.get('board'), null);
    assert.equal(next.searchParams.get('player'), digest);
    assert.equal(next.searchParams.get('cursor'), 'page+2');
    assert.throws(
        () => routeFromUrl(`https://pages.example/leaderboards/?player=${digest}&cursor=bad%0Acursor`),
        /cursor is invalid/u,
    );
});

test('filter serialization preserves API configuration and the flat LeaderboardQueryV2 vocabulary', () => {
    const filters = {
        boardId: 'demo-standard-normal',
        missionId: 'Dem_Lei_MP',
        metric: 'fastest_success' as const,
        maxConcurrentPlayers: 2,
        cursor: 'cursor+opaque',
    };
    const url = new URL(urlWithFilters('https://pages.example/leaderboards/?api=https%3A%2F%2Fapi.example%2Fapi%2Fv1&run=old', filters));
    assert.equal(url.searchParams.get('api'), 'https://api.example/api/v1');
    assert.equal(url.searchParams.get('run'), null);
    assert.equal(url.searchParams.get('board'), 'demo-standard-normal');
    assert.deepEqual(routeFromUrl(url.toString()), { kind: 'leaderboard', filters });

    assert.deepEqual(filtersToApiQuery(filters), {
        schema_version: 2,
        board_id: 'demo-standard-normal',
        mission_id: 'Dem_Lei_MP',
        metric: 'fastest_success',
        max_concurrent_players: 2,
        limit: 25,
        cursor: 'cursor+opaque',
    });
});

test('Worker-hosted leaderboard paths survive canonical query navigation and direct reload links', () => {
    assert.deepEqual(routeFromUrl('https://robinhood.phiresky.xyz/leaderboards/?run=run_42'), { kind: 'run', id: 'run_42' });
    const output = new URL(urlWithFilters('https://robinhood.phiresky.xyz/leaderboards/', {
        boardId: 'b', missionId: null, metric: null, maxConcurrentPlayers: null, cursor: null,
    }));
    assert.equal(output.pathname, '/leaderboards/');
    assert.equal(output.search, '?board=b');
});
