import assert from 'node:assert/strict';
import test from 'node:test';
import { boardFacets, boardForFacet, filtersForBoard, normalizeFilters, withAggregateViews } from './board-filters.js';
import { boardDocument, metadataDocument } from './model-fixtures.js';
import { parseBoardMetadata } from './public-response.js';
import type { BoardFilters } from './state.js';
import { validateBoardView } from './view-model.js';
import type { BoardPage } from './types.js';

const empty: BoardFilters = { boardId: null, missionId: null, metric: null, maxConcurrentPlayers: null, cursor: null };

function metadata(extraBoards: readonly Record<string, unknown>[] = []) {
    const document = metadataDocument();
    const boards = [...(document.boards as Record<string, unknown>[]), ...extraBoards]
        .sort((left, right) => String(left.board_id) < String(right.board_id) ? -1 : 1);
    return parseBoardMetadata({ ...document, boards });
}

test('defaults select a full-game board, or the remembered board', () => {
    const selected = normalizeFilters(empty, metadata());
    assert.equal(selected.boardId, 'full-any-config');
    assert.equal(selected.missionId, 'M01');
    assert.equal(selected.metric, 'original_score');
    assert.equal(normalizeFilters(empty, metadata(), 'full-any-config').boardId, 'full-any-config');
    assert.equal(normalizeFilters(empty, metadata(), 'retired-board').boardId, 'full-any-config');
    assert.equal(normalizeFilters({ ...empty, boardId: 'demo-standard-normal' }, metadata(), 'full-any-config').boardId, 'demo-standard-normal');
});

test('unpublished boards, missions and metrics fail clearly', () => {
    assert.throws(() => normalizeFilters({ ...empty, boardId: 'missing' }, metadata()), /board is not published/u);
    assert.throws(() => normalizeFilters({ ...empty, boardId: 'demo-standard-normal', missionId: 'M01' }, metadata()), /mission is not part of this board/u);
    assert.throws(() => normalizeFilters({ ...empty, boardId: 'full-any-config', metric: 'fastest_success' }, metadata()), /does not rank the selected metric/u);
    assert.throws(() => normalizeFilters(empty, parseBoardMetadata({ ...metadataDocument(), boards: [] })), /no ranked boards/u);
});

test('edition, preset and difficulty facets select boards and keep compatible choices', () => {
    const hard = boardDocument({ board_id: 'demo-standard-hard', display_name: 'Demo / Standard / Hard', difficulty_id: 'hard', difficulty_name: 'Hard',
        simulation_policy: { kind: 'fixed', policy: { version: 1, preset: 'standard', difficulty: 'hard' } } });
    const original = boardDocument({ board_id: 'demo-original-easy', display_name: 'Demo / Original / Easy', preset_id: 'original', preset_name: 'Original',
        difficulty_id: 'easy', difficulty_name: 'Easy', simulation_policy: { kind: 'fixed', policy: { version: 1, preset: 'original_parity', difficulty: 'easy' } } });
    const all = metadata([hard, original]);
    const normal = normalizeFilters({ ...empty, boardId: 'demo-standard-normal' }, all).board;
    const facets = boardFacets(all, normal);
    assert.deepEqual(facets.editions.map(item => item.id), ['demo', 'full']);
    assert.deepEqual(facets.presets.map(item => item.id), ['original', 'standard']);
    // Facets keep the canonical board_id order of the metadata.
    assert.deepEqual(facets.difficulties.map(item => item.id), ['hard', 'normal']);
    assert.deepEqual(facets.variants.map(item => item.boardId), ['demo-standard-normal']);

    assert.equal(boardForFacet(all, normal, { difficultyId: 'hard' }).boardId, 'demo-standard-hard');
    assert.equal(boardForFacet(all, normal, { presetId: 'original' }).boardId, 'demo-original-easy');
    const full = boardForFacet(all, normal, { edition: 'full' });
    assert.equal(full.boardId, 'full-any-config');
    assert.throws(() => boardForFacet(all, normal, { presetId: 'missing' }), /No board is published/u);

    const current = { ...empty, boardId: normal.boardId, missionId: 'Dem_Lin_MP', metric: 'fastest_success' as const, maxConcurrentPlayers: 2, cursor: 'c' };
    assert.deepEqual(filtersForBoard(all.boards.find(board => board.boardId === 'demo-standard-hard')!, current),
        { boardId: 'demo-standard-hard', missionId: 'Dem_Lin_MP', metric: 'fastest_success', maxConcurrentPlayers: 2, cursor: null });
    assert.deepEqual(filtersForBoard(full, current),
        { boardId: 'full-any-config', missionId: null, metric: null, maxConcurrentPlayers: 2, cursor: null });
});

test('a leaderboard page must answer the exact requested board query', () => {
    const filters = normalizeFilters({ ...empty, maxConcurrentPlayers: 1 }, metadata());
    const page: BoardPage = {
        filter: { boardId: filters.boardId, missionId: filters.missionId, metric: filters.metric, maxConcurrentPlayers: 1, playerPublicKey: null },
        entries: [], acceptedSequenceWatermark: 0, previousCursor: null, nextCursorDocument: null, nextCursor: null,
    };
    validateBoardView(page, filters);
    for (const change of [{ boardId: 'other' }, { missionId: 'Dem_Lin_MP' }, { metric: 'fastest_success' as const }, { maxConcurrentPlayers: null }, { playerPublicKey: '1'.repeat(64) }]) {
        assert.throws(() => validateBoardView({ ...page, filter: { ...page.filter, ...change } }, filters), /different filters/u);
    }
});

test('Any ruleset is a browsing view built from all full-game submission boards', () => {
    const source = metadata([
        boardDocument({ board_id: 'full-standard-normal', edition: 'full',
            viewer_content_requirement: 'user_local_retail' }),
    ]);
    const views = withAggregateViews(source);
    assert.equal(source.boards.some(board => board.boardId === 'full-any'), false);
    const any = normalizeFilters({ ...empty, boardId: 'full-any' }, views).board;
    assert.deepEqual(any.metrics, ['original_score', 'fastest_success']);
    assert.deepEqual(new Set(any.missions.map(mission => mission.missionId)),
        new Set(source.boards.filter(board => board.edition === 'full').flatMap(board => board.missions.map(mission => mission.missionId))));
    const standard = views.boards.find(board => board.boardId === 'full-standard-normal')!;
    assert.equal(boardForFacet(views, standard, { presetId: 'any' }).boardId, 'full-any');
    assert.equal(withAggregateViews(views).boards.filter(board => board.boardId === 'full-any').length, 1);
});
