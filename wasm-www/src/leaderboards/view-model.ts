import type { SelectedBoardFilters } from './state.js';
import type { Board, BoardMetadata, BoardPage } from './types.js';
import { editionLabel } from './format.js';

/** The server must answer exactly the requested board query. */
export function validateBoardView(page: BoardPage, filters: SelectedBoardFilters): void {
    if (page.filter.boardId !== filters.boardId
        || page.filter.missionId !== filters.missionId
        || page.filter.metric !== filters.metric
        || page.filter.maxConcurrentPlayers !== filters.maxConcurrentPlayers
        || page.filter.playerPublicKey !== null) {
        throw new Error('The server returned a leaderboard for different filters.');
    }
}

export function boardPolicyLabel(board: Board): string {
    const settings = board.simulationPolicy.kind === 'any_config'
        ? 'any gameplay settings'
        : `${board.presetName} / ${board.difficultyName}`;
    return `${editionLabel(board.edition)} · ${settings} · state loading ${board.allowStateLoad ? 'allowed' : 'excluded'}`;
}

/** Labels for a run's board and mission; a board may no longer be published. */
export function runLabels(metadata: BoardMetadata, boardId: string, missionId: string): {
    readonly board: Board | null;
    readonly boardLabel: string;
    readonly missionLabel: string;
} {
    const board = metadata.boards.find(item => item.boardId === boardId) ?? null;
    return {
        board,
        boardLabel: board?.displayName ?? `${boardId} (no longer published)`,
        missionLabel: board?.missions.find(mission => mission.missionId === missionId)?.displayName ?? missionId,
    };
}
