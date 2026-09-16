import assert from 'node:assert/strict';
import test from 'node:test';
import { missionPlayUrl } from './browsing.js';

test('play links select the mission and content edition without carrying leaderboard or replay parameters', () => {
    for (const edition of ['full', 'demo'] as const) {
        const url = new URL(missionPlayUrl('https://game.example/leaderboards/?run=old&board=full-any&cursor=old#rankings', edition, 'S02_Lei_MP'));
        assert.equal(url.pathname, '/');
        assert.equal(url.hash, '');
        assert.deepEqual([...url.searchParams], [['mission', 'S02_Lei_MP'], ['edition', edition]]);
    }
});
