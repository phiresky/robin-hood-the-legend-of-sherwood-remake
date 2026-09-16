import { createHash } from 'node:crypto';

export const sha = (character: string): string => character.repeat(64);
export const publicKey = '1'.repeat(64);
export const publicFingerprint = '24752c730a3eea290df5600aacb45161';
export const secondPublicKey = '2'.repeat(64);
export const secondPublicFingerprint = '94ca02d832b9e0d11b766cf04148bccf';

export const canonicalJson = (value: unknown): string => {
    if (value === null || typeof value !== 'object') return JSON.stringify(value);
    if (Array.isArray(value)) return `[${value.map(canonicalJson).join(',')}]`;
    const object = value as Record<string, unknown>;
    return `{${Object.keys(object).sort().map(key => `${JSON.stringify(key)}:${canonicalJson(object[key])}`).join(',')}}`;
};
export const canonicalDigest = (value: unknown): string =>
    createHash('sha256').update(canonicalJson(value)).digest('hex');

export const uploader = {
    seat: 0,
    username: 'Robin',
    public_key: publicKey,
    public_key_fingerprint: publicFingerprint,
};

export function boardDocument(overrides: Record<string, unknown> = {}): Record<string, unknown> {
    return {
        board_id: 'demo-standard-normal',
        display_name: 'Demo / Standard / Normal',
        edition: 'demo',
        preset_id: 'standard',
        preset_name: 'Standard',
        difficulty_id: 'normal',
        difficulty_name: 'Normal',
        simulation_policy: { kind: 'fixed', policy: { version: 1, preset: 'standard', difficulty: 'medium' } },
        allow_state_load: false,
        metrics: ['original_score', 'fastest_success'],
        viewer_content_requirement: 'bundled_demo',
        missions: [
            { mission_id: 'Dem_Lei_MP', display_name: 'Leicester' },
            { mission_id: 'Dem_Lin_MP', display_name: 'Lincoln' },
        ],
        ...overrides,
    };
}

export function metadataDocument(): Record<string, unknown> {
    return {
        schema_version: 2,
        tick_duration: { numerator_micros: 50_000, denominator: 1 },
        boards: [
            boardDocument(),
            boardDocument({
                board_id: 'full-any-config',
                display_name: 'Full / Any settings',
                edition: 'full',
                preset_id: 'any',
                preset_name: 'Any settings',
                difficulty_id: 'custom',
                difficulty_name: 'Custom',
                simulation_policy: { kind: 'any_config' },
                allow_state_load: true,
                metrics: ['original_score'],
                viewer_content_requirement: 'user_local_retail',
                missions: [{ mission_id: 'M01', display_name: 'Sherwood' }],
            }),
        ],
    };
}

export function runFilter(overrides: Record<string, unknown> = {}): Record<string, unknown> {
    return {
        schema_version: 2,
        board_id: 'demo-standard-normal',
        mission_id: 'Dem_Lei_MP',
        metric: 'original_score',
        max_concurrent_players: null,
        ...overrides,
    };
}

export function leaderboardEntry(position: number, rank: number, points: number, sequence: number): Record<string, unknown> {
    return {
        position,
        rank,
        run_id: `run-${position}`,
        metric_value: { metric: 'original_score', points },
        max_concurrent_players: 1,
        participant_instance_count: 1,
        uploader: position === 1 ? uploader : null,
        replay_sha256: sha(String(position % 10)),
        accepted_sequence: sequence,
        verified_at_unix_ms: 1_700_000_000_000 + sequence,
    };
}

export function leaderboardPage(entries: readonly Record<string, unknown>[] = [
    leaderboardEntry(1, 1, 500, 1),
    leaderboardEntry(2, 1, 500, 2),
    leaderboardEntry(3, 3, 400, 3),
]): Record<string, unknown> {
    return {
        schema_version: 2,
        filter: runFilter(),
        accepted_sequence_watermark: 10,
        previous_cursor: null,
        entries,
        next_cursor: null,
    };
}

export function cursorFor(page: Record<string, unknown>, entry: Record<string, unknown>, token: string): Record<string, unknown> {
    return {
        schema_version: 2,
        query_sha256: canonicalDigest(page.filter),
        accepted_sequence_watermark: page.accepted_sequence_watermark,
        last: {
            position: entry.position,
            rank: entry.rank,
            metric_value: entry.metric_value,
            accepted_sequence: entry.accepted_sequence,
            verified_at_unix_ms: entry.verified_at_unix_ms,
            run_id: entry.run_id,
        },
        opaque_token: token,
    };
}

export function runDetail(): Record<string, unknown> {
    return {
        schema_version: 2,
        run_id: 'run-1',
        board_id: 'demo-standard-normal',
        mission_id: 'Dem_Lei_MP',
        edition: 'demo',
        metrics: { original_score_delta: 500, active_simulation_ticks: 1200, ransom_collected: 4 },
        max_concurrent_players: 1,
        participant_instance_count: 1,
        uploader,
        verified_at_unix_ms: 1_700_000_000_001,
        replay: {
            artifact: { sha256: sha('a'), byte_length: 123, media_type: 'application/x-robin-rhrec' },
            replay_schema_version: 44,
        },
        recorded_engine_version: '0123456789ab',
        sim_config: { difficulty: 'medium', friendly_fire: false, nested: { value: 3 } },
        starting_campaign_score: 0,
        final_campaign_score: 500,
        achievements: [
            { display_name: 'Quiet hands', verified: { achievement_id: 'quiet-hands', evaluation: 'earned' } },
            { display_name: 'Unseen', verified: { achievement_id: 'unseen', evaluation: 'unverifiable' } },
        ],
        viewer: { availability: { status: 'available' }, content_requirement: 'bundled_demo', runtime_build: '0123456789ab' },
    };
}

export function playerHistoryPage(): Record<string, unknown> {
    return {
        schema_version: 2,
        player: {
            schema_version: 1,
            username: '<img src=x onerror=alert(1)>',
            public_key: publicKey,
            public_key_fingerprint: publicFingerprint,
        },
        accepted_sequence_watermark: 9,
        runs: [{
            player_public_key: publicKey,
            run: {
                schema_version: 2,
                run_id: 'run-9',
                board_id: 'demo-standard-normal',
                mission_id: 'Dem_Lei_MP',
                max_concurrent_players: 1,
                participant_instance_count: 1,
                uploader,
                metrics: { original_score_delta: 120, active_simulation_ticks: 240, ransom_collected: 4 },
            },
            verified_at_unix_ms: 1_700_000_000_000,
        }],
        personal_bests: [{
            filter: runFilter({ player_public_key: publicKey }),
            run_id: 'run-9',
            metric_value: { metric: 'original_score', points: 120 },
        }],
        next_cursor: 'authenticated-cursor',
    };
}
