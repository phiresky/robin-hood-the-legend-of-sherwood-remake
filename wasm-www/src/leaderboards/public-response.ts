// Public leaderboard, player history and run responses (protocol V2).
import {
    type Achievement,
    type Board,
    type BoardMetadata,
    type BoardMetric,
    type BoardMetricValue,
    type BoardPage,
    type BoardSimulationPolicy,
    type LeaderboardCursor,
    type LeaderboardEntry,
    type LeaderboardOrderAnchor,
    type PlayerPersonalBest,
    type PlayerRunHistoryEntry,
    type PlayerRunHistoryPage,
    type ReplayArtifact,
    type RunDetail,
    type RunFilter,
    type RunSummary,
    type ViewerLaunch,
} from './types.js';
import {
    METRIC_ORDER,
    array,
    boolean,
    boundedString,
    enumeration,
    i32,
    nonNegativeInteger,
    nonzeroSha256,
    object,
    assertExactKeys,
    opaqueId,
    positiveInteger,
    positiveUnixMilliseconds,
    publicKey,
    replaySeatCount,
    strictObject,
    strictlyIncreasingByOrder,
    u16,
    versionedObjectV2,
} from './decode.js';
import { canonicalDocumentSha256Sync, compareUtf8, parseCanonicalMap } from './canonical.js';
import {
    parseMetricValue,
    parseRunMetrics,
    parseTickDuration,
    parseUploader,
    validateParticipantCounts,
} from './participants-metrics.js';
import { parsePlayerProfile } from './account-contract.js';

export const RANKED_REPLAY_MEDIA_TYPE = 'application/x-robin-rhrec+compact';

export function parseBoardMetadata(value: unknown): BoardMetadata {
    const obj = versionedObjectV2(value, 'metadata', ['tick_duration', 'boards']);
    const boards = array(obj.boards, 'metadata.boards').map((item, index) =>
        parseBoard(item, `metadata.boards[${index}]`));
    if (boards.length > 1024) throw new Error('metadata.boards exceeds the protocol limit');
    if (!boards.every((board, index) => index === 0
        || compareUtf8(boards[index - 1]?.boardId ?? '', board.boardId) < 0)) {
        throw new Error('metadata.boards must be strictly ordered by board_id');
    }
    return { tickDuration: parseTickDuration(obj.tick_duration, 'metadata.tick_duration'), boards };
}

export function parseBoard(value: unknown, path: string): Board {
    const obj = strictObject(value, path, [
        'board_id', 'display_name', 'edition', 'preset_id', 'preset_name', 'difficulty_id',
        'difficulty_name', 'simulation_policy', 'allow_state_load', 'metrics',
        'viewer_content_requirement', 'missions',
    ]);
    const metrics = array(obj.metrics, `${path}.metrics`).map((item, index) =>
        enumeration(item, METRIC_ORDER, `${path}.metrics[${index}]`));
    if (metrics.length === 0 || !strictlyIncreasingByOrder(metrics, METRIC_ORDER)) {
        throw new Error(`${path}.metrics must be non-empty and in canonical order without duplicates`);
    }
    const missions = array(obj.missions, `${path}.missions`).map((item, index) => {
        const mission = strictObject(item, `${path}.missions[${index}]`, ['mission_id', 'display_name']);
        return {
            missionId: boundedString(mission.mission_id, `${path}.missions[${index}].mission_id`, 256),
            displayName: boundedString(mission.display_name, `${path}.missions[${index}].display_name`, 100),
        };
    });
    if (missions.length === 0 || missions.length > 4096
        || new Set(missions.map(mission => mission.missionId)).size !== missions.length) {
        throw new Error(`${path}.missions must contain 1–4096 unique missions`);
    }
    return {
        boardId: opaqueId(obj.board_id, `${path}.board_id`),
        displayName: boundedString(obj.display_name, `${path}.display_name`, 100),
        edition: enumeration(obj.edition, ['demo', 'full'] as const, `${path}.edition`),
        presetId: boundedString(obj.preset_id, `${path}.preset_id`, 100),
        presetName: boundedString(obj.preset_name, `${path}.preset_name`, 100),
        difficultyId: boundedString(obj.difficulty_id, `${path}.difficulty_id`, 100),
        difficultyName: boundedString(obj.difficulty_name, `${path}.difficulty_name`, 100),
        simulationPolicy: parseSimulationPolicy(obj.simulation_policy, `${path}.simulation_policy`),
        allowStateLoad: boolean(obj.allow_state_load, `${path}.allow_state_load`),
        metrics,
        viewerContentRequirement: enumeration(
            obj.viewer_content_requirement,
            ['bundled_demo', 'user_local_retail'] as const,
            `${path}.viewer_content_requirement`,
        ),
        missions,
    };
}

export function parseSimulationPolicy(value: unknown, path: string): BoardSimulationPolicy {
    const obj = object(value, path);
    const kind = enumeration(obj.kind, ['fixed', 'any_config'] as const, `${path}.kind`);
    if (kind === 'any_config') {
        assertExactKeys(obj, path, ['kind']);
        return { kind };
    }
    const outer = strictObject(obj, path, ['kind', 'policy']);
    const policy = strictObject(outer.policy, `${path}.policy`, ['version', 'preset', 'difficulty']);
    if (policy.version !== 1) throw new Error(`${path}.policy.version must be 1`);
    const preset = enumeration(policy.preset, ['standard', 'original_parity', 'custom'] as const, `${path}.policy.preset`);
    const difficulty = enumeration(
        policy.difficulty,
        ['easy', 'medium', 'hard', 'legendary', 'custom'] as const,
        `${path}.policy.difficulty`,
    );
    if (preset === 'custom' || difficulty === 'legendary' || difficulty === 'custom') {
        throw new Error(`${path} must be a Standard or Original preset with a retail difficulty`);
    }
    return { kind, policy: { version: 1, preset, difficulty } };
}

export function parseRunFilter(value: unknown, path: string): RunFilter {
    const obj = versionedObjectV2(value, path, [
        'board_id', 'mission_id', 'metric', 'max_concurrent_players',
    ], ['player_public_key']);
    return {
        boardId: opaqueId(obj.board_id, `${path}.board_id`),
        missionId: boundedString(obj.mission_id, `${path}.mission_id`, 256),
        metric: enumeration(obj.metric, METRIC_ORDER, `${path}.metric`),
        maxConcurrentPlayers: obj.max_concurrent_players === null
            ? null
            : replaySeatCount(obj.max_concurrent_players, `${path}.max_concurrent_players`),
        playerPublicKey: obj.player_public_key === undefined
            ? null
            : publicKey(obj.player_public_key, `${path}.player_public_key`),
    };
}

export function parseBoardPage(value: unknown): BoardPage {
    const obj = versionedObjectV2(value, 'leaderboard', [
        'filter', 'accepted_sequence_watermark', 'previous_cursor', 'entries', 'next_cursor',
    ]);
    const filter = parseRunFilter(obj.filter, 'leaderboard.filter');
    const acceptedSequenceWatermark = nonNegativeInteger(
        obj.accepted_sequence_watermark,
        'leaderboard.accepted_sequence_watermark',
    );
    const previousCursor = obj.previous_cursor === null
        ? null
        : parseLeaderboardCursor(obj.previous_cursor, 'leaderboard.previous_cursor');
    const entries = array(obj.entries, 'leaderboard.entries').map((entry, index) =>
        parseLeaderboardEntry(entry, `leaderboard.entries[${index}]`));
    const nextCursorDocument = obj.next_cursor === null
        ? null
        : parseLeaderboardCursor(obj.next_cursor, 'leaderboard.next_cursor');
    if (entries.length > 100) throw new Error('leaderboard.entries exceeds protocol maximum');
    if (entries.length === 0) {
        if (previousCursor !== null || nextCursorDocument !== null) {
            throw new Error('an empty leaderboard page cannot carry cursors');
        }
        return { filter, entries, acceptedSequenceWatermark, previousCursor, nextCursorDocument, nextCursor: null };
    }
    if (acceptedSequenceWatermark === 0) {
        throw new Error('a non-empty leaderboard page requires a positive snapshot watermark');
    }
    if (new Set(entries.map(entry => entry.runId)).size !== entries.length
        || new Set(entries.map(entry => entry.acceptedSequence)).size !== entries.length) {
        throw new Error('leaderboard entries contain duplicate run or acceptance identities');
    }
    for (const entry of entries) {
        if (entry.metricValue.metric !== filter.metric) {
            throw new Error('leaderboard entries do not match the page metric');
        }
        if (filter.maxConcurrentPlayers !== null
            && entry.maxConcurrentPlayers !== filter.maxConcurrentPlayers) {
            throw new Error('leaderboard entry max_concurrent_players does not match its filter');
        }
        if (entry.acceptedSequence > acceptedSequenceWatermark) {
            throw new Error('leaderboard entry is newer than the page snapshot watermark');
        }
    }
    const querySha256 = canonicalDocumentSha256Sync(obj.filter);
    if (previousCursor !== null && (previousCursor.querySha256 !== querySha256
        || previousCursor.acceptedSequenceWatermark !== acceptedSequenceWatermark)) {
        throw new Error('previous leaderboard cursor does not match the page filter and snapshot');
    }
    let prior = previousCursor?.last ?? null;
    for (const entry of entries) {
        if (prior === null && (entry.position !== 1 || entry.rank !== 1)) {
            throw new Error('the first leaderboard page must begin at position and rank 1');
        }
        validateLeaderboardOrder(filter.metric, prior, entry);
        prior = leaderboardAnchor(entry);
    }
    if (nextCursorDocument !== null) {
        if (nextCursorDocument.acceptedSequenceWatermark !== acceptedSequenceWatermark
            || nextCursorDocument.querySha256 !== querySha256
            || prior === null || !equalLeaderboardAnchors(nextCursorDocument.last, prior)) {
            throw new Error('next leaderboard cursor does not bind the final page row');
        }
    }
    return {
        filter,
        entries,
        acceptedSequenceWatermark,
        previousCursor,
        nextCursorDocument,
        nextCursor: nextCursorDocument?.opaqueToken ?? null,
    };
}

export function parseLeaderboardEntry(value: unknown, path: string): LeaderboardEntry {
    const obj = strictObject(value, path, [
        'position', 'rank', 'run_id', 'metric_value', 'max_concurrent_players',
        'participant_instance_count', 'uploader', 'replay_sha256', 'accepted_sequence',
        'verified_at_unix_ms',
    ]);
    const position = positiveInteger(obj.position, `${path}.position`);
    const rank = positiveInteger(obj.rank, `${path}.rank`);
    if (rank > position) throw new Error(`${path}.rank cannot exceed its snapshot position`);
    const maxConcurrentPlayers = replaySeatCount(obj.max_concurrent_players, `${path}.max_concurrent_players`);
    const participantInstanceCount = u16(obj.participant_instance_count, `${path}.participant_instance_count`);
    validateParticipantCounts(maxConcurrentPlayers, participantInstanceCount, path);
    return {
        position,
        rank,
        runId: opaqueId(obj.run_id, `${path}.run_id`),
        metricValue: parseMetricValue(obj.metric_value, `${path}.metric_value`),
        maxConcurrentPlayers,
        participantInstanceCount,
        uploader: parseUploader(obj.uploader, `${path}.uploader`),
        replaySha256: nonzeroSha256(obj.replay_sha256, `${path}.replay_sha256`),
        acceptedSequence: positiveInteger(obj.accepted_sequence, `${path}.accepted_sequence`),
        verifiedAtUnixMs: positiveUnixMilliseconds(obj.verified_at_unix_ms, `${path}.verified_at_unix_ms`),
    };
}

export function parseLeaderboardCursor(value: unknown, path: string): LeaderboardCursor {
    const obj = versionedObjectV2(value, path, [
        'query_sha256', 'accepted_sequence_watermark', 'last', 'opaque_token',
    ]);
    const acceptedSequenceWatermark = positiveInteger(
        obj.accepted_sequence_watermark,
        `${path}.accepted_sequence_watermark`,
    );
    const last = parseLeaderboardOrderAnchor(obj.last, `${path}.last`);
    if (last.acceptedSequence > acceptedSequenceWatermark) {
        throw new Error(`${path}.last is newer than its snapshot watermark`);
    }
    return {
        querySha256: nonzeroSha256(obj.query_sha256, `${path}.query_sha256`),
        acceptedSequenceWatermark,
        last,
        opaqueToken: boundedString(obj.opaque_token, `${path}.opaque_token`, 4096),
    };
}

export function parseLeaderboardOrderAnchor(value: unknown, path: string): LeaderboardOrderAnchor {
    const obj = strictObject(value, path, [
        'position', 'rank', 'metric_value', 'accepted_sequence', 'verified_at_unix_ms', 'run_id',
    ]);
    const position = positiveInteger(obj.position, `${path}.position`);
    const rank = positiveInteger(obj.rank, `${path}.rank`);
    if (rank > position) throw new Error(`${path}.rank cannot exceed its snapshot position`);
    return {
        position,
        rank,
        metricValue: parseMetricValue(obj.metric_value, `${path}.metric_value`),
        acceptedSequence: positiveInteger(obj.accepted_sequence, `${path}.accepted_sequence`),
        verifiedAtUnixMs: positiveUnixMilliseconds(obj.verified_at_unix_ms, `${path}.verified_at_unix_ms`),
        runId: opaqueId(obj.run_id, `${path}.run_id`),
    };
}

export function leaderboardAnchor(entry: LeaderboardEntry): LeaderboardOrderAnchor {
    return {
        position: entry.position,
        rank: entry.rank,
        metricValue: entry.metricValue,
        acceptedSequence: entry.acceptedSequence,
        verifiedAtUnixMs: entry.verifiedAtUnixMs,
        runId: entry.runId,
    };
}

export function equalLeaderboardAnchors(left: LeaderboardOrderAnchor, right: LeaderboardOrderAnchor): boolean {
    return left.position === right.position
        && left.rank === right.rank
        && metricPrimaryValue(left.metricValue.metric, left.metricValue)
            === metricPrimaryValue(left.metricValue.metric, right.metricValue)
        && left.acceptedSequence === right.acceptedSequence
        && left.verifiedAtUnixMs === right.verifiedAtUnixMs
        && left.runId === right.runId;
}

/** Mirrors robin_run_protocol::query::validate_leaderboard_order. */
export function validateLeaderboardOrder(
    metric: BoardMetric,
    prior: LeaderboardOrderAnchor | null,
    current: LeaderboardEntry,
): void {
    if (prior === null) return;
    if (current.position !== prior.position + 1) {
        throw new Error('leaderboard positions must be gap-free across pages');
    }
    const priorPrimary = metricPrimaryValue(metric, prior.metricValue);
    const currentPrimary = metricPrimaryValue(metric, current.metricValue);
    const comparison = metric === 'original_score'
        ? priorPrimary - currentPrimary
        : currentPrimary - priorPrimary;
    if (comparison < 0) throw new Error('leaderboard primary metric order is invalid');
    if (comparison === 0) {
        if (current.rank !== prior.rank || compareLeaderboardTieTuple(current, prior) <= 0) {
            throw new Error('leaderboard tied entries have invalid rank or stable order');
        }
    } else if (current.rank !== current.position) {
        throw new Error('leaderboard rank after a changed metric must equal its position');
    }
}

export function metricPrimaryValue(metric: BoardMetric, value: BoardMetricValue): number {
    if (metric !== value.metric) throw new Error('leaderboard metric value does not match its filter');
    return value.metric === 'original_score' ? value.points : value.activeSimulationTicks;
}

export function compareLeaderboardTieTuple(
    left: Pick<LeaderboardOrderAnchor, 'acceptedSequence' | 'verifiedAtUnixMs' | 'runId'>,
    right: Pick<LeaderboardOrderAnchor, 'acceptedSequence' | 'verifiedAtUnixMs' | 'runId'>,
): number {
    if (left.acceptedSequence !== right.acceptedSequence) {
        return left.acceptedSequence - right.acceptedSequence;
    }
    if (left.verifiedAtUnixMs !== right.verifiedAtUnixMs) {
        return left.verifiedAtUnixMs - right.verifiedAtUnixMs;
    }
    return compareUtf8(left.runId, right.runId);
}

export function parseRunSummary(value: unknown, path: string): RunSummary {
    const obj = versionedObjectV2(value, path, [
        'run_id', 'board_id', 'mission_id', 'max_concurrent_players',
        'participant_instance_count', 'uploader', 'metrics',
    ]);
    const maxConcurrentPlayers = replaySeatCount(obj.max_concurrent_players, `${path}.max_concurrent_players`);
    const participantInstanceCount = u16(obj.participant_instance_count, `${path}.participant_instance_count`);
    validateParticipantCounts(maxConcurrentPlayers, participantInstanceCount, path);
    return {
        runId: opaqueId(obj.run_id, `${path}.run_id`),
        boardId: opaqueId(obj.board_id, `${path}.board_id`),
        missionId: boundedString(obj.mission_id, `${path}.mission_id`, 256),
        maxConcurrentPlayers,
        participantInstanceCount,
        uploader: parseUploader(obj.uploader, `${path}.uploader`),
        metrics: parseRunMetrics(obj.metrics, `${path}.metrics`),
    };
}

export function parsePlayerRunHistoryEntry(value: unknown, path: string): PlayerRunHistoryEntry {
    const obj = strictObject(value, path, ['player_public_key', 'run', 'verified_at_unix_ms']);
    return {
        playerPublicKey: publicKey(obj.player_public_key, `${path}.player_public_key`),
        run: parseRunSummary(obj.run, `${path}.run`),
        verifiedAtUnixMs: positiveUnixMilliseconds(obj.verified_at_unix_ms, `${path}.verified_at_unix_ms`),
    };
}

export function parsePlayerPersonalBest(value: unknown, path: string): PlayerPersonalBest {
    const obj = strictObject(value, path, ['filter', 'run_id', 'metric_value']);
    const filter = parseRunFilter(obj.filter, `${path}.filter`);
    const metricValue = parseMetricValue(obj.metric_value, `${path}.metric_value`);
    if (filter.playerPublicKey === null) throw new Error(`${path}.filter must name its player`);
    if (filter.metric !== metricValue.metric) {
        throw new Error(`${path}.metric_value does not match its board filter`);
    }
    return {
        filter: { ...filter, playerPublicKey: filter.playerPublicKey },
        runId: opaqueId(obj.run_id, `${path}.run_id`),
        metricValue,
    };
}

export function parsePlayerRunHistoryPage(value: unknown): PlayerRunHistoryPage {
    const obj = versionedObjectV2(value, 'player_run_history', [
        'player', 'accepted_sequence_watermark', 'runs', 'personal_bests', 'next_cursor',
    ]);
    const player = parsePlayerProfile(obj.player);
    const runs = array(obj.runs, 'player_run_history.runs').map((item, index) =>
        parsePlayerRunHistoryEntry(item, `player_run_history.runs[${index}]`));
    const rawBests = array(obj.personal_bests, 'player_run_history.personal_bests');
    const personalBests = rawBests.map((item, index) =>
        parsePlayerPersonalBest(item, `player_run_history.personal_bests[${index}]`));
    if (runs.length > 100 || personalBests.length > 512) {
        throw new Error('player_run_history collections exceed protocol limits');
    }
    if (new Set(runs.map(entry => entry.run.runId)).size !== runs.length) {
        throw new Error('player_run_history contains duplicate run identities');
    }
    if (runs.some(entry => entry.playerPublicKey !== player.publicKey)) {
        throw new Error('player_run_history run belongs to a different public key');
    }
    if (personalBests.some(best => best.filter.playerPublicKey !== player.publicKey)) {
        throw new Error('player_run_history personal best belongs to a different public key');
    }
    const filterDigests = new Set(rawBests.map(item => canonicalDocumentSha256Sync(object(item, 'personal best').filter)));
    if (filterDigests.size !== personalBests.length) {
        throw new Error('player_run_history contains duplicate personal-best filters');
    }
    return {
        player,
        acceptedSequenceWatermark: nonNegativeInteger(
            obj.accepted_sequence_watermark,
            'player_run_history.accepted_sequence_watermark',
        ),
        runs,
        personalBests,
        nextCursor: obj.next_cursor === null
            ? null
            : boundedString(obj.next_cursor, 'player_run_history.next_cursor', 4096),
    };
}

export function parseReplayArtifact(value: unknown, path: string): ReplayArtifact {
    const obj = strictObject(value, path, ['artifact', 'replay_schema_version']);
    const artifact = strictObject(obj.artifact, `${path}.artifact`, ['sha256', 'byte_length', 'media_type']);
    if (artifact.media_type !== RANKED_REPLAY_MEDIA_TYPE) {
        throw new Error(`${path}.artifact.media_type must be ${RANKED_REPLAY_MEDIA_TYPE}`);
    }
    return {
        artifact: {
            sha256: nonzeroSha256(artifact.sha256, `${path}.artifact.sha256`),
            byteLength: positiveInteger(artifact.byte_length, `${path}.artifact.byte_length`),
            mediaType: RANKED_REPLAY_MEDIA_TYPE,
        },
        replaySchemaVersion: positiveInteger(obj.replay_schema_version, `${path}.replay_schema_version`),
    };
}

export function parseViewerLaunch(value: unknown, path: string): ViewerLaunch {
    const obj = strictObject(value, path, ['availability', 'content_requirement', 'runtime_build']);
    const availabilityObject = object(obj.availability, `${path}.availability`);
    const status = enumeration(availabilityObject.status, ['available', 'unavailable'] as const, `${path}.availability.status`);
    const availability: ViewerLaunch['availability'] = status === 'available'
        ? (assertExactKeys(availabilityObject, `${path}.availability`, ['status']), { status })
        : {
            status,
            safeReason: boundedString(
                strictObject(availabilityObject, `${path}.availability`, ['status', 'safe_reason']).safe_reason,
                `${path}.availability.safe_reason`,
                500,
            ),
        };
    return {
        availability,
        contentRequirement: enumeration(
            obj.content_requirement,
            ['bundled_demo', 'user_local_retail'] as const,
            `${path}.content_requirement`,
        ),
        runtimeBuild: boundedString(obj.runtime_build, `${path}.runtime_build`, 64),
    };
}

export function parseAchievement(value: unknown, path: string): Achievement {
    const obj = strictObject(value, path, ['display_name', 'verified']);
    const verified = strictObject(obj.verified, `${path}.verified`, ['achievement_id', 'evaluation']);
    return {
        id: opaqueId(verified.achievement_id, `${path}.verified.achievement_id`),
        label: boundedString(obj.display_name, `${path}.display_name`, 100),
        evaluation: enumeration(
            verified.evaluation,
            ['unverifiable', 'not_earned', 'earned'] as const,
            `${path}.verified.evaluation`,
        ),
    };
}

export function parseRunDetail(value: unknown): RunDetail {
    const obj = versionedObjectV2(value, 'run', [
        'run_id', 'board_id', 'mission_id', 'edition', 'metrics', 'max_concurrent_players',
        'participant_instance_count', 'uploader', 'verified_at_unix_ms', 'replay',
        'recorded_engine_version', 'sim_config', 'starting_campaign_score', 'final_campaign_score',
        'achievements', 'viewer',
    ]);
    const maxConcurrentPlayers = replaySeatCount(obj.max_concurrent_players, 'run.max_concurrent_players');
    const participantInstanceCount = u16(obj.participant_instance_count, 'run.participant_instance_count');
    validateParticipantCounts(maxConcurrentPlayers, participantInstanceCount, 'run');
    const recordedEngineVersion = boundedString(obj.recorded_engine_version, 'run.recorded_engine_version', 64);
    const viewer = parseViewerLaunch(obj.viewer, 'run.viewer');
    if (viewer.runtimeBuild !== recordedEngineVersion) {
        throw new Error('run.viewer.runtime_build must be the recorded engine version');
    }
    if (typeof obj.sim_config !== 'object' || obj.sim_config === null || Array.isArray(obj.sim_config)) {
        throw new Error('run.sim_config must be an object');
    }
    const achievements = array(obj.achievements, 'run.achievements').map((item, index) =>
        parseAchievement(item, `run.achievements[${index}]`));
    if (achievements.length > 256) throw new Error('run.achievements exceeds the protocol limit');
    return {
        runId: opaqueId(obj.run_id, 'run.run_id'),
        boardId: opaqueId(obj.board_id, 'run.board_id'),
        missionId: boundedString(obj.mission_id, 'run.mission_id', 256),
        edition: enumeration(obj.edition, ['demo', 'full'] as const, 'run.edition'),
        metrics: parseRunMetrics(obj.metrics, 'run.metrics'),
        maxConcurrentPlayers,
        participantInstanceCount,
        uploader: parseUploader(obj.uploader, 'run.uploader'),
        verifiedAtUnixMs: positiveUnixMilliseconds(obj.verified_at_unix_ms, 'run.verified_at_unix_ms'),
        replay: parseReplayArtifact(obj.replay, 'run.replay'),
        recordedEngineVersion,
        simConfig: parseCanonicalMap(obj.sim_config, 'run.sim_config', 64),
        startingCampaignScore: i32(obj.starting_campaign_score, 'run.starting_campaign_score'),
        finalCampaignScore: i32(obj.final_campaign_score, 'run.final_campaign_score'),
        achievements,
        viewer,
    };
}
