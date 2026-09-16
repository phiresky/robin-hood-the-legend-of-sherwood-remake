import assert from 'node:assert/strict';
import test from 'node:test';
import { browsableMetadata, missionGroup, missionLabel } from './missions.js';
import { formatActiveTime } from './format.js';
import { normalizeFilters, withAggregateViews } from './board-filters.js';
import { boardDocument } from './model-fixtures.js';
import { parseBoardMetadata } from './public-response.js';

test('mission browsing uses numbered titles and matching demo names', () => {
    assert.equal(missionLabel('H01_Lin_VL', 'Official FULL H01_Lin_VL'), "01 · Finding Godwin");
    assert.equal(missionLabel('Dem_Lei_MP', 'Official DEMO Dem_Lei_MP', true), '03 · Scarlet Night (Demo)');
    assert.equal(missionLabel('Dem_Lin_MP', '', true), `${missionLabel('H01_Lin_VL', '')} (Demo)`);
    assert.equal(missionGroup('S02_Lei_MP'), 'Campaign');
    assert.equal(missionGroup('Tac17_FoC_EC'), 'Tactical missions');
});

test('full-game default starts with the first story mission and excludes unwinnable locations', () => {
    const metadata = browsableMetadata(withAggregateViews(parseBoardMetadata({
        schema_version: 2, tick_duration: { numerator_micros: 40_000, denominator: 1 },
        boards: [boardDocument(), boardDocument({
            board_id: 'full-original-normal', edition: 'full', viewer_content_requirement: 'user_local_retail',
            missions: ['Emb01_FoA_EC', 'H01_Lin_VL', 'S02_Lei_MP', 'Sherwood', 'SherwoodOutro'].map(id => ({ mission_id: id, display_name: `Official FULL ${id}` })),
        })],
    })));
    const empty = { boardId: null, missionId: null, metric: null, cursor: null, maxConcurrentPlayers: null };
    const selected = normalizeFilters(empty, metadata);
    assert.equal(selected.boardId, 'full-any');
    assert.equal(selected.missionId, 'H01_Lin_VL');
    assert.deepEqual(selected.board.missions.map(mission => mission.missionId), ['H01_Lin_VL', 'S02_Lei_MP', 'Emb01_FoA_EC']);
    for (const missionId of ['Sherwood', 'SherwoodOutro']) {
        assert.throws(() => normalizeFilters({ ...empty, missionId }, metadata), /mission is not part/u);
    }
});

test('25 Hz run times preserve every frame, including minute and hour boundaries', () => {
    const tick = { numeratorMicros: 40_000, denominator: 1 };
    for (const [frames, expected] of [[0, '0:00.00'], [1, '0:00.04'], [2, '0:00.08'], [24, '0:00.96'], [25, '0:01.00'], [1499, '0:59.96'], [1500, '1:00.00'], [90001, '1:00:00.04']] as const) {
        assert.equal(formatActiveTime(frames, tick), expected);
    }
});
