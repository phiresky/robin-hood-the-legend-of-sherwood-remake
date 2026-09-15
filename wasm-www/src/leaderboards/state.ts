import { MAX_REPLAY_SEATS } from './decode.js';
import { type BoardMetric } from './types.js';

export const PAGE_SIZE = 25;

/** Board selection as read from the URL; unset fields take published defaults. */
export type BoardFilters = {
    readonly boardId: string | null;
    readonly missionId: string | null;
    readonly metric: BoardMetric | null;
    readonly maxConcurrentPlayers: number | null;
    readonly cursor: string | null;
};

/** A board selection resolved against published metadata. */
export type SelectedBoardFilters = BoardFilters & {
    readonly boardId: string;
    readonly missionId: string;
    readonly metric: BoardMetric;
};

export type PageRoute =
    | { readonly kind: 'submission'; readonly id: string }
    | { readonly kind: 'leaderboard'; readonly filters: BoardFilters }
    | { readonly kind: 'run'; readonly id: string }
    | { readonly kind: 'player'; readonly publicKey: string; readonly cursor: string | null };

const SHA256_RE = /^[0-9a-f]{64}$/u;
const ROUTE_KEYS = ['run', 'submission', 'player', 'board', 'mission', 'metric', 'players', 'cursor'] as const;

export function routeFromUrl(urlString: string): PageRoute {
    const url = new URL(urlString);
    const run = optionalProtocolText(url.searchParams.get('run'), 'run', 128);
    const submission = optionalProtocolText(url.searchParams.get('submission'), 'submission', 128);
    const player = optionalSha256(url.searchParams.get('player'), 'player');
    const cursor = optionalProtocolText(url.searchParams.get('cursor'), 'cursor', 4096);
    if ([run, player, submission].filter(value => value !== null).length > 1) {
        throw new Error('A page URL cannot select more than one run, player or submission.');
    }
    if (run !== null) return { kind: 'run', id: run };
    if (submission !== null) return { kind: 'submission', id: submission };
    if (player !== null) return { kind: 'player', publicKey: player, cursor };
    return {
        kind: 'leaderboard',
        filters: {
            boardId: optionalProtocolText(url.searchParams.get('board'), 'board', 128),
            missionId: optionalProtocolText(url.searchParams.get('mission'), 'mission', 256),
            metric: enumParam(url, 'metric', ['original_score', 'fastest_success'] as const),
            maxConcurrentPlayers: optionalPlayerCount(url.searchParams.get('players'), 'players'),
            cursor,
        },
    };
}

export function urlWithPlayerCursor(currentUrl: string, publicKey: string, cursor: string | null): string {
    const url = new URL(currentUrl);
    for (const key of ROUTE_KEYS) url.searchParams.delete(key);
    url.searchParams.set('player', publicKey);
    setOptional(url, 'cursor', cursor);
    return url.toString();
}

export function urlWithFilters(currentUrl: string, filters: BoardFilters): string {
    const url = new URL(currentUrl);
    for (const key of ROUTE_KEYS) url.searchParams.delete(key);
    setOptional(url, 'board', filters.boardId);
    setOptional(url, 'mission', filters.missionId);
    setOptional(url, 'metric', filters.metric);
    setOptional(url, 'players', filters.maxConcurrentPlayers === null ? null : String(filters.maxConcurrentPlayers));
    setOptional(url, 'cursor', filters.cursor);
    return url.toString();
}

/** Flat LeaderboardQueryV2 accepted by `GET /api/v1/leaderboards`. */
export function filtersToApiQuery(filters: SelectedBoardFilters): Readonly<Record<string, string | number | null>> {
    return {
        schema_version: 2,
        board_id: filters.boardId,
        mission_id: filters.missionId,
        metric: filters.metric,
        max_concurrent_players: filters.maxConcurrentPlayers,
        limit: PAGE_SIZE,
        cursor: filters.cursor,
    };
}

function setOptional(url: URL, key: string, value: string | null): void {
    if (value !== null) url.searchParams.set(key, value);
}

function enumParam<const T extends readonly string[]>(url: URL, key: string, choices: T): T[number] | null {
    const value = url.searchParams.get(key);
    if (value === null || value.length === 0) return null;
    if (!choices.includes(value)) throw new Error(`${key} must be one of: ${choices.join(', ')}`);
    return value;
}

function optionalSha256(value: string | null, label: string): string | null {
    if (value === null || value.length === 0) return null;
    if (!SHA256_RE.test(value) || /^0{64}$/u.test(value)) {
        throw new Error(`${label} must be a non-zero SHA-256 digest.`);
    }
    return value;
}

function optionalPlayerCount(value: string | null, label: string): number | null {
    if (value === null || value.length === 0) return null;
    if (!/^[1-9][0-9]*$/u.test(value)) throw new Error(`${label} must be a positive integer.`);
    const parsed = Number(value);
    if (!Number.isSafeInteger(parsed) || parsed > MAX_REPLAY_SEATS) throw new Error(`${label} is outside the replay seat range.`);
    return parsed;
}

function optionalProtocolText(value: string | null, label: string, maxLength: number): string | null {
    if (value === null || value.length === 0) return null;
    if (new TextEncoder().encode(value).byteLength > maxLength
        || value.trim() !== value
        || /\p{Cc}|\p{Bidi_Control}/u.test(value)) throw new Error(`${label} is invalid.`);
    return value;
}
