import {
    type BoardMetric,
} from './types.js';

export const PAGE_SIZE = 25;
export type SubjectSelection = 'individual_level' | 'campaign' | 'full_campaign';

export type BoardFilters = {
    readonly subject: SubjectSelection;
    readonly metric: BoardMetric;
    readonly missionId: string | null;
    readonly presetId: string | null;
    readonly difficultyId: string | null;
    readonly rulesetId: string | null;
    readonly contentIdentitySha256: string | null;
    readonly rulesConfigSha256: string | null;
    readonly competitionManifestSha256: string | null;
    readonly maxConcurrentPlayers: number | null;
    readonly cursor: string | null;
};

export type PageRoute =
    | { readonly kind: 'leaderboard'; readonly filters: BoardFilters }
    | { readonly kind: 'run'; readonly id: string }
    | { readonly kind: 'campaign_session'; readonly aggregateRunId: string; readonly ordinal: number }
    | { readonly kind: 'player'; readonly publicKey: string; readonly cursor: string | null };

const SHA256_RE = /^[0-9a-f]{64}$/u;

export function routeFromUrl(
    urlString: string,
    rememberedSubject: SubjectSelection = 'individual_level',
): PageRoute {
    const url = new URL(urlString);
    const run = optionalOpaqueId(url.searchParams.get('run'), 'run');
    const session = optionalNonNegativeInteger(url.searchParams.get('session'), 'session');
    if (url.searchParams.has('submission')) {
        throw new Error('Private submission status is available only through an authenticated owner client.');
    }
    const player = optionalSha256(url.searchParams.get('player'), 'player');
    const cursor = optionalBoundedString(url.searchParams.get('cursor'), 'cursor', 4096);
    if ([run, player].filter(value => value !== null).length > 1) {
        throw new Error('A page URL cannot select more than one run or player.');
    }
    if (session !== null && run === null) {
        throw new Error('A campaign session ordinal requires its aggregate run identity.');
    }
    if (run !== null && session !== null) return { kind: 'campaign_session', aggregateRunId: run, ordinal: session };
    if (run !== null) return { kind: 'run', id: run };
    if (player !== null) return { kind: 'player', publicKey: player, cursor };
    return {
        kind: 'leaderboard',
        filters: {
            subject: enumParam(
                url,
                'subject',
                ['individual_level', 'campaign', 'full_campaign'] as const,
            ) ?? rememberedSubject,
            metric: enumParam(url, 'metric', ['original_score', 'fastest_success'] as const) ?? 'original_score',
            missionId: optionalProtocolText(url.searchParams.get('mission'), 'mission', 256),
            presetId: optionalOpaqueId(url.searchParams.get('preset'), 'preset'),
            difficultyId: optionalOpaqueId(url.searchParams.get('difficulty'), 'difficulty'),
            rulesetId: optionalSha256(url.searchParams.get('ruleset'), 'ruleset'),
            contentIdentitySha256: null,
            rulesConfigSha256: null,
            competitionManifestSha256: optionalSha256(url.searchParams.get('competition'), 'competition'),
            maxConcurrentPlayers: optionalPositiveInteger(url.searchParams.get('players'), 'players'),
            cursor,
        },
    };
}

export function urlWithPlayerCursor(
    currentUrl: string,
    publicKey: string,
    cursor: string | null,
): string {
    const url = new URL(currentUrl);
    for (const key of [
        'run', 'session', 'submission', 'player', 'subject', 'metric', 'mission', 'preset',
        'difficulty', 'ruleset', 'competition', 'players', 'cursor',
    ]) url.searchParams.delete(key);
    url.searchParams.set('player', publicKey);
    setOptional(url, 'cursor', cursor);
    return url.toString();
}

export function urlWithFilters(currentUrl: string, filters: BoardFilters): string {
    const url = new URL(currentUrl);
    for (const key of [
        'run', 'session', 'submission', 'player', 'subject', 'metric', 'mission', 'preset', 'difficulty',
        'ruleset', 'competition', 'players', 'cursor',
    ]) url.searchParams.delete(key);
    url.searchParams.set('subject', filters.subject);
    url.searchParams.set('metric', filters.metric);
    setOptional(url, 'mission', filters.missionId);
    setOptional(url, 'preset', filters.presetId);
    setOptional(url, 'difficulty', filters.difficultyId);
    setOptional(url, 'ruleset', filters.rulesetId);
    setOptional(url, 'competition', filters.competitionManifestSha256);
    setOptional(url, 'players', filters.maxConcurrentPlayers === null ? null : String(filters.maxConcurrentPlayers));
    setOptional(url, 'cursor', filters.cursor);
    return url.toString();
}

export function filtersToApiQuery(filters: BoardFilters): Readonly<Record<string, string | number | null>> {
    return {
        schema_version: 1,
        subject_kind: filters.subject === 'full_campaign' ? 'full_campaign' : 'mission',
        mission_id: filters.subject === 'full_campaign' ? null : filters.missionId,
        mission_scope: filters.subject === 'full_campaign' ? null : filters.subject,
        metric: filters.metric,
        content_identity_sha256: filters.contentIdentitySha256,
        rules_config_sha256: filters.rulesConfigSha256,
        ruleset_manifest_sha256: filters.rulesetId,
        competition_manifest_sha256: filters.competitionManifestSha256,
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
    if (value === null) return null;
    if (!choices.includes(value)) throw new Error(`${key} must be one of: ${choices.join(', ')}`);
    return value;
}

function optionalOpaqueId(value: string | null, label: string): string | null {
    return optionalProtocolText(value, label, 128);
}

function optionalSha256(value: string | null, label: string): string | null {
    if (value === null || value.length === 0) return null;
    if (!SHA256_RE.test(value) || /^0{64}$/u.test(value)) {
        throw new Error(`${label} must be a non-zero SHA-256 digest.`);
    }
    return value;
}

function optionalPositiveInteger(value: string | null, label: string): number | null {
    if (value === null || value.length === 0) return null;
    if (!/^[1-9][0-9]*$/u.test(value)) throw new Error(`${label} must be a positive integer.`);
    const parsed = Number(value);
    if (!Number.isSafeInteger(parsed) || parsed > 4) throw new Error(`${label} is outside the replay seat range.`);
    return parsed;
}

function optionalNonNegativeInteger(value: string | null, label: string): number | null {
    if (value === null || value.length === 0) return null;
    if (!/^(?:0|[1-9][0-9]*)$/u.test(value)) throw new Error(`${label} must be a non-negative integer.`);
    const parsed = Number(value);
    if (!Number.isSafeInteger(parsed) || parsed > 0xffff_ffff) throw new Error(`${label} is too large.`);
    return parsed;
}

function optionalBoundedString(value: string | null, label: string, maxLength: number): string | null {
    return optionalProtocolText(value, label, maxLength);
}

function optionalProtocolText(value: string | null, label: string, maxLength: number): string | null {
    if (value === null || value.length === 0) return null;
    if (new TextEncoder().encode(value).byteLength > maxLength
        || value.trim() !== value
        || /\p{Cc}|\p{Bidi_Control}/u.test(value)) throw new Error(`${label} is invalid.`);
    return value;
}
