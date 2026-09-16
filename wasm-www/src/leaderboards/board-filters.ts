import type { BoardFilters, SelectedBoardFilters } from './state.js';
import type { Board, BoardMetadata, ContentEdition } from './types.js';

export type NormalizedBoardFilters = SelectedBoardFilters & { readonly board: Board };

export type FacetOption = { readonly id: string; readonly label: string };

export function requireBoard(metadata: BoardMetadata, boardId: string): Board {
    const board = metadata.boards.find(item => item.boardId === boardId);
    if (board === undefined) throw new Error('The selected board is not published by this server.');
    return board;
}

/**
 * Resolve URL filters against published boards: board × mission × metric, with
 * the first published choice as default. A named but unpublished choice fails
 * clearly instead of silently showing another board.
 */
export function normalizeFilters(
    input: BoardFilters,
    metadata: BoardMetadata,
    rememberedBoardId: string | null = null,
): NormalizedBoardFilters {
    const fallback = metadata.boards.find(item => item.boardId === rememberedBoardId) ?? metadata.boards[0];
    if (fallback === undefined) throw new Error('This server publishes no ranked boards.');
    const board = input.boardId === null ? fallback : requireBoard(metadata, input.boardId);
    const missionId = input.missionId ?? board.missions[0]?.missionId;
    if (missionId === undefined || !board.missions.some(mission => mission.missionId === missionId)) {
        throw new Error('The selected mission is not part of this board.');
    }
    const metric = input.metric ?? board.metrics[0];
    if (metric === undefined || !board.metrics.includes(metric)) {
        throw new Error('This board does not rank the selected metric.');
    }
    return { ...input, board, boardId: board.boardId, missionId, metric };
}

/** Choices for the edition, preset and difficulty selectors around the current board. */
export function boardFacets(metadata: BoardMetadata, board: Board): {
    readonly editions: readonly FacetOption[];
    readonly presets: readonly FacetOption[];
    readonly difficulties: readonly FacetOption[];
    /** Boards sharing all three labels; more than one needs an explicit board choice. */
    readonly variants: readonly Board[];
} {
    const sameEdition = metadata.boards.filter(item => item.edition === board.edition);
    const samePreset = sameEdition.filter(item => item.presetId === board.presetId);
    return {
        editions: unique(metadata.boards.map(item => ({ id: item.edition, label: item.edition === 'demo' ? 'Demo' : 'Full game' }))),
        presets: unique(sameEdition.map(item => ({ id: item.presetId, label: item.presetName }))),
        difficulties: unique(samePreset.map(item => ({ id: item.difficultyId, label: item.difficultyName }))),
        variants: samePreset.filter(item => item.difficultyId === board.difficultyId),
    };
}

/** The board selected by changing one facet, keeping the other facets where possible. */
export function boardForFacet(
    metadata: BoardMetadata,
    current: Board,
    change: { readonly edition?: ContentEdition; readonly presetId?: string; readonly difficultyId?: string },
): Board {
    const narrow = (boards: readonly Board[], matches: (board: Board) => boolean): readonly Board[] => {
        const matching = boards.filter(matches);
        return matching.length === 0 ? boards : matching;
    };
    const edition = change.edition ?? current.edition;
    const byEdition = metadata.boards.filter(board => board.edition === edition);
    if (byEdition.length === 0) throw new Error('No board is published for the selected edition.');
    const presetId = change.presetId ?? current.presetId;
    const byPreset = change.presetId === undefined
        ? narrow(byEdition, board => board.presetId === presetId)
        : byEdition.filter(board => board.presetId === presetId);
    const difficultyId = change.difficultyId ?? current.difficultyId;
    const byDifficulty = change.difficultyId === undefined
        ? narrow(byPreset, board => board.difficultyId === difficultyId)
        : byPreset.filter(board => board.difficultyId === difficultyId);
    const selected = byDifficulty.find(board => board.boardId === current.boardId) ?? byDifficulty[0];
    if (selected === undefined) throw new Error('No board is published for the selected preset and difficulty.');
    return selected;
}

/** Filters after switching boards: keep the mission and metric when the new board has them. */
export function filtersForBoard(board: Board, current: BoardFilters): BoardFilters {
    return {
        boardId: board.boardId,
        missionId: current.missionId !== null && board.missions.some(mission => mission.missionId === current.missionId)
            ? current.missionId
            : null,
        metric: current.metric !== null && board.metrics.includes(current.metric) ? current.metric : null,
        maxConcurrentPlayers: current.maxConcurrentPlayers,
        cursor: null,
    };
}

function unique(values: readonly FacetOption[]): readonly FacetOption[] {
    const seen = new Set<string>();
    return values.filter(value => !seen.has(value.id) && seen.add(value.id));
}

/** Full-game browsing combines configured submission boards without publishing a new one. */
export function withAggregateViews(metadata: BoardMetadata): BoardMetadata {
    const full = metadata.boards.filter(board => board.edition === 'full');
    const first = full[0];
    if (first === undefined) return metadata;
    const aggregate: Board = {
        ...first,
        boardId: 'full-any', displayName: 'Full / Any ruleset',
        presetId: 'any', presetName: 'Any ruleset',
        difficultyId: 'any', difficultyName: 'Any difficulty',
        simulationPolicy: { kind: 'any_config' },
        allowStateLoad: full.some(board => board.allowStateLoad),
        metrics: ['original_score', 'fastest_success'].filter(metric => full.some(board => board.metrics.includes(metric as Board['metrics'][number]))) as Board['metrics'],
        missions: [...new Map(full.flatMap(board => board.missions).map(mission => [mission.missionId, mission])).values()],
    };
    return { ...metadata, boards: [...metadata.boards.filter(board => board.boardId !== 'full-any'), aggregate]
        .sort((a, b) => a.boardId < b.boardId ? -1 : a.boardId > b.boardId ? 1 : 0) };
}
