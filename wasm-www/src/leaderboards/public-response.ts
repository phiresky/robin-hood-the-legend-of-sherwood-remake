// Public leaderboard, player history and run response mapping.
import {
    type RunDetail,
    type CampaignContentManifest,
    type OfficialContentSubject,
    type CampaignSessionDetail,
    type BoardMetadata,
    type BoardPage,
    type PlayerRunHistoryPage,
    type RulesetFacet,
    type Competition,
    type FullCampaignFacet,
    type VerifiedRunComposition,
    type RunSummary,
    type PlayerRunHistoryEntry,
    type PlayerRunFilter,
    type PlayerPersonalBest,
    type LeaderboardCursor,
    type LeaderboardOrderAnchor,
    type LeaderboardEntry,
    type BoardMetric,
    type BoardMetricValue,
} from './types.js';
import {
    protocolValuesEqual,
    versionedObject,
    array,
    strictlySorted,
    nonNegativeInteger,
    opaqueId,
    enumeration,
    replaySeatCount,
    u32Positive,
    u32,
    nonzeroSha256,
    nullableSha256,
    i32,
    nullablePositiveInteger,
    positiveUnixMilliseconds,
    boundedString,
    strictObject,
    CATEGORY_ORDER,
    METRIC_ORDER,
    strictlyIncreasingByOrder,
    boolean,
    object,
    assertExactKeys,
    u64DecimalString,
    multiplayerSeatCount,
    publicKey,
    positiveInteger,
} from './decode.js';
import { parseMission, parseSubject, parseRunContentIdentity, runContentDigest } from './subject-contract.js';
import { canonicalDocumentSha256Sync } from './canonical.js';
import {
    parseRunMetrics,
    parseMetricValue,
    parseParticipant,
    parseAggregateParticipant,
    validateRoster,
} from './participants-metrics.js';
import { parseCampaignArtifactRef, parseReplayArtifact } from './build-contract.js';
import {
    parseInputProvenance,
    parsePublicBuild,
    parseViewerLaunch,
    parseVerificationProof,
    parseCampaignAggregate,
    parseFullCampaignSession,
    parseAchievement,
    validateMissionRunArtifacts,
    validateFullCampaignArtifacts,
    publicPlaybackProof,
    validateFullSessionProof,
} from './run-proof.js';
import { parsePlayerProfile } from './account-contract.js';

export function campaignContentCatalogDigest(run: RunDetail): string | null {
    return run.content.kind === 'full_campaign'
        ? run.content.campaignContentManifestSha256
        : run.campaignContentManifestSha256;
}

/** Cross-bind a digest-authenticated campaign catalog to every public run subject. */
export function validateRunCampaignContentBinding(
    run: RunDetail,
    catalog: CampaignContentManifest | null,
): void {
    const digest = campaignContentCatalogDigest(run);
    if ((digest === null) !== (catalog === null)) {
        throw new Error('The run campaign content catalog was omitted or unexpectedly substituted.');
    }
    if (catalog === null) return;
    const contentFor = (subject: OfficialContentSubject): string | null =>
        catalog.entries.find(entry => entry.subject.kind === subject.kind
            && entry.subject.missionId === subject.missionId)?.contentManifestSha256 ?? null;
    if (run.subject.kind === 'mission') {
        if (run.content.kind !== 'mission'
            || contentFor({ kind: 'field_mission', missionId: run.subject.missionId })
                !== run.content.contentManifestSha256) {
            throw new Error('The campaign mission does not match its authenticated content catalog entry.');
        }
        return;
    }
    if (run.fullCampaignSessions.some(session =>
        contentFor(session.contentSubject) !== session.contentManifestSha256)) {
        throw new Error('A Full Campaign session does not match its authenticated content catalog entry.');
    }
}

/** Prove that a separately fetched session is the exact session embedded by the aggregate. */
export function validateCampaignSessionDetailBinding(
    aggregate: RunDetail,
    detail: CampaignSessionDetail,
): void {
    if (aggregate.subject.kind !== 'full_campaign' || aggregate.composition.kind !== 'full_campaign') {
        throw new Error('A campaign-session detail requires a verified Full Campaign aggregate.');
    }
    if (detail.aggregateRunId !== aggregate.runId
        || detail.publicAggregateResultSha256 !== aggregate.publicResultSha256
        || detail.ordinal !== detail.session.ordinal) {
        throw new Error('Campaign session detail does not match its aggregate route and result identity.');
    }
    const inline = aggregate.fullCampaignSessions[detail.ordinal];
    if (inline === undefined || !protocolValuesEqual(inline, detail.session)) {
        throw new Error('Campaign session detail does not match the aggregate session at its ordinal.');
    }
}

export function parseBoardMetadata(value: unknown): BoardMetadata {
    const obj = versionedObject(value, 'metadata', ['missions', 'rulesets', 'competitions', 'full_campaign']);
    const missions = array(obj.missions, 'metadata.missions').map((item, index) =>
        parseMission(item, `metadata.missions[${index}]`));
    const rulesets = array(obj.rulesets, 'metadata.rulesets').map((item, index) =>
        parseRuleset(item, `metadata.rulesets[${index}]`));
    const competitions = array(obj.competitions, 'metadata.competitions').map((item, index) =>
        parseCompetition(item, `metadata.competitions[${index}]`));
    if (missions.length > 4096 || rulesets.length > 1024 || competitions.length > 1024
        || !strictlySorted(missions.map(mission => mission.id))
        || !strictlySorted(rulesets.map(ruleset => `${ruleset.id}:${ruleset.content.kind === 'mission' ? '0' : '1'}:${runContentDigest(ruleset.content)}`))
        || !strictlySorted(competitions.map(competition => competition.manifestSha256))) {
        throw new Error('metadata collections exceed protocol limits or are not canonically ordered');
    }
    return {
        missions,
        rulesets,
        competitions,
        fullCampaign: obj.full_campaign === null
            ? null
            : parseFullCampaignFacet(obj.full_campaign, 'metadata.full_campaign'),
    };
}

export function parseBoardPage(value: unknown): BoardPage {
    const obj = versionedObject(value, 'leaderboard', [
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
    } else if (acceptedSequenceWatermark === 0) {
        throw new Error('a non-empty leaderboard page requires a positive snapshot watermark');
    }
    for (const entry of entries) {
        if (entry.composition.kind !== filter.subject.kind || entry.metricValue.metric !== filter.metric) {
            throw new Error('leaderboard entries do not match the exact subject and metric filter');
        }
        if (filter.maxConcurrentPlayers !== null
            && entry.maxConcurrentPlayers !== filter.maxConcurrentPlayers) {
            throw new Error('leaderboard entry max_concurrent_players does not match its filter');
        }
        if (entry.acceptedSequence > acceptedSequenceWatermark) {
            throw new Error('leaderboard entry is newer than the page snapshot watermark');
        }
    }
    if (new Set(entries.map(entry => entry.runId)).size !== entries.length
        || new Set(entries.map(entry => entry.acceptedSequence)).size !== entries.length) {
        throw new Error('leaderboard entries contain duplicate run or acceptance identities');
    }
    if (previousCursor !== null && previousCursor.acceptedSequenceWatermark !== acceptedSequenceWatermark) {
        throw new Error('previous leaderboard cursor snapshot does not match the page');
    }
    const querySha256 = canonicalDocumentSha256Sync(obj.filter);
    if (previousCursor !== null && previousCursor.querySha256 !== querySha256) {
        throw new Error('previous leaderboard cursor query does not match the page filter');
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

export function parseRunDetail(value: unknown): RunDetail {
    const obj = versionedObject(value, 'run', [
        'run_id', 'rank', 'subject', 'mission', 'composition', 'outcome', 'metrics', 'metric_value',
        'max_concurrent_players', 'participant_instance_count', 'named_participant_instance_count',
        'named_participants', 'aggregate_named_participants', 'anonymous_participant_instance_count', 'verified_at_unix_ms',
        'public_request_sha256', 'public_result_sha256', 'verification_proof', 'campaign_aggregate', 'replay',
        'content', 'campaign_content_manifest_sha256', 'rules_config_sha256',
        'ruleset_manifest_sha256', 'competition_manifest_sha256', 'starting_campaign',
        'final_campaign', 'starting_campaign_score', 'final_campaign_score',
        'input_provenance', 'build', 'achievements', 'viewer', 'full_campaign_sessions',
        'trust_statement',
    ]);
    const runId = opaqueId(obj.run_id, 'run.run_id');
    const subject = parseSubject(obj.subject, 'run.subject');
    const composition = parseComposition(obj.composition, 'run.composition');
    if (subject.kind !== composition.kind) throw new Error('run subject and composition kinds do not match');
    const mission = obj.mission === null ? null : parseMission(obj.mission, 'run.mission');
    const outcome = enumeration(obj.outcome, ['won', 'lost', 'interrupted'] as const, 'run.outcome');
    if (outcome !== 'won') throw new Error('public ranked run outcome must be won');
    const metrics = parseRunMetrics(obj.metrics, 'run.metrics');
    const metricValue = parseMetricValue(obj.metric_value, 'run.metric_value');
    if (metricValue.metric === 'original_score' && metricValue.points !== metrics.originalScoreDelta) {
        throw new Error('run score metric does not match verifier-derived metrics');
    }
    if (metricValue.metric === 'fastest_success'
        && metricValue.activeSimulationTicks !== metrics.activeSimulationTicks) {
        throw new Error('run time metric does not match verifier-derived metrics');
    }
    const maxConcurrentPlayers = replaySeatCount(obj.max_concurrent_players, 'run.max_concurrent_players');
    const participantInstanceCount = u32Positive(obj.participant_instance_count, 'run.participant_instance_count');
    const namedParticipantInstanceCount = u32(
        obj.named_participant_instance_count,
        'run.named_participant_instance_count',
    );
    const namedParticipants = array(obj.named_participants, 'run.named_participants').map((item, index) =>
        parseParticipant(item, `run.named_participants[${index}]`));
    const aggregateNamedParticipants = array(
        obj.aggregate_named_participants,
        'run.aggregate_named_participants',
    ).map((item, index) => parseAggregateParticipant(item, `run.aggregate_named_participants[${index}]`));
    const anonymousParticipantInstanceCount = u32(
        obj.anonymous_participant_instance_count,
        'run.anonymous_participant_instance_count',
    );
    validateRoster(
        maxConcurrentPlayers,
        participantInstanceCount,
        namedParticipantInstanceCount,
        namedParticipants,
        aggregateNamedParticipants,
        anonymousParticipantInstanceCount,
        subject.kind === 'mission',
        'run',
    );
    const publicRequestSha256 = nonzeroSha256(
        obj.public_request_sha256,
        'run.public_request_sha256',
    );
    const publicResultSha256 = nonzeroSha256(obj.public_result_sha256, 'run.public_result_sha256');
    const content = parseRunContentIdentity(obj.content, 'run.content', subject);
    const contentManifestSha256 = runContentDigest(content);
    const campaignContentManifestSha256 = nullableSha256(
        obj.campaign_content_manifest_sha256,
        'run.campaign_content_manifest_sha256',
    );
    const expectsCampaignCatalog = subject.kind === 'mission' && subject.category === 'campaign';
    if (expectsCampaignCatalog !== (campaignContentManifestSha256 !== null)) {
        throw new Error('run campaign content catalog does not match its subject');
    }
    const rulesConfigSha256 = nonzeroSha256(obj.rules_config_sha256, 'run.rules_config_sha256');
    const rulesetManifestSha256 = nonzeroSha256(obj.ruleset_manifest_sha256, 'run.ruleset_manifest_sha256');
    const competitionManifestSha256 = nullableSha256(
        obj.competition_manifest_sha256,
        'run.competition_manifest_sha256',
    );
    const startingCampaign = parseCampaignArtifactRef(
        obj.starting_campaign,
        'run.starting_campaign',
    );
    const finalCampaign = parseCampaignArtifactRef(
        obj.final_campaign,
        'run.final_campaign',
    );
    const startingCampaignScore = i32(obj.starting_campaign_score, 'run.starting_campaign_score');
    const finalCampaignScore = i32(obj.final_campaign_score, 'run.final_campaign_score');
    const inputProvenance = parseInputProvenance(obj.input_provenance, 'run.input_provenance');
    if (inputProvenance.status !== 'rankable') throw new Error('public verified run input provenance must be rankable');
    const build = obj.build === null ? null : parsePublicBuild(obj.build, 'run.build');
    const viewer = obj.viewer === null ? null : parseViewerLaunch(obj.viewer, 'run.viewer');
    const verificationProof = obj.verification_proof === null
        ? null
        : parseVerificationProof(obj.verification_proof, 'run.verification_proof');
    const campaignAggregate = obj.campaign_aggregate === null
        ? null
        : parseCampaignAggregate(obj.campaign_aggregate, 'run.campaign_aggregate');
    const replay = obj.replay === null ? null : parseReplayArtifact(obj.replay);
    const fullCampaignSessions = array(obj.full_campaign_sessions, 'run.full_campaign_sessions').map((item, index) =>
        parseFullCampaignSession(item, `run.full_campaign_sessions[${index}]`));

    const common = {
        runId, subject, composition, mission, metrics, maxConcurrentPlayers, participantInstanceCount,
        namedParticipantInstanceCount, namedParticipants, aggregateNamedParticipants,
        anonymousParticipantInstanceCount, publicRequestSha256,
        publicResultSha256, content, contentManifestSha256, campaignContentManifestSha256,
        rulesConfigSha256, rulesetManifestSha256,
        competitionManifestSha256,
        startingCampaign, finalCampaign, startingCampaignScore, finalCampaignScore,
        inputProvenance,
        achievements: array(obj.achievements, 'run.achievements').map((item, index) =>
            parseAchievement(item, `run.achievements[${index}]`)),
    };
    if (subject.kind === 'mission') {
        validateMissionRunArtifacts(common, {
            mission, composition, build, viewer, verificationProof, campaignAggregate, replay,
            fullCampaignSessions,
        });
    } else {
        validateFullCampaignArtifacts(common, {
            mission, composition, build, viewer, verificationProof, campaignAggregate, replay,
            fullCampaignSessions,
        });
    }

    return {
        runId,
        rank: nullablePositiveInteger(obj.rank, 'run.rank'),
        subject,
        mission,
        composition,
        outcome,
        metrics,
        metricValue,
        maxConcurrentPlayers,
        participantInstanceCount,
        namedParticipantInstanceCount,
        namedParticipants,
        aggregateNamedParticipants,
        anonymousParticipantInstanceCount,
        verifiedAtUnixMs: positiveUnixMilliseconds(obj.verified_at_unix_ms, 'run.verified_at_unix_ms'),
        publicRequestSha256,
        publicResultSha256,
        replay,
        replayFrameCount: verificationProof?.replayFrameCount ?? null,
        playbackProof: verificationProof === null ? null : publicPlaybackProof(verificationProof),
        content,
        campaignContentManifestSha256,
        rulesConfigSha256,
        rulesetManifestSha256,
        competitionManifestSha256,
        startingCampaign,
        finalCampaign,
        startingCampaignScore,
        finalCampaignScore,
        inputProvenance,
        build,
        achievements: common.achievements,
        viewer,
        fullCampaignSessions,
        trustStatement: boundedString(obj.trust_statement, 'run.trust_statement', 1000),
    };
}

export function parseCampaignSessionDetail(value: unknown): CampaignSessionDetail {
    const obj = versionedObject(value, 'campaign_session_detail', [
        'aggregate_run_id', 'public_aggregate_result_sha256', 'ordinal', 'session',
    ]);
    const ordinal = u32(obj.ordinal, 'campaign_session_detail.ordinal');
    const session = parseFullCampaignSession(obj.session, 'campaign_session_detail.session');
    if (session.ordinal !== ordinal) {
        throw new Error('campaign_session_detail ordinal does not match its session');
    }
    validateFullSessionProof(session, 'campaign_session_detail.session');
    return {
        aggregateRunId: opaqueId(obj.aggregate_run_id, 'campaign_session_detail.aggregate_run_id'),
        publicAggregateResultSha256: nonzeroSha256(
            obj.public_aggregate_result_sha256,
            'campaign_session_detail.public_aggregate_result_sha256',
        ),
        ordinal,
        session,
    };
}

/**
 * Re-validates the authenticated public projection before it is displayed.
 *
 * Public request and result identities are canonical digests of the embedded redacted documents.
 * Private verifier and aggregate digests never enter this API, so the frontend recomputes both.
 */
export async function verifyRunDetailProofDigests(value: unknown): Promise<void> {
    parseRunDetail(value);
}

/** Re-validates the redacted verifier projection carried by an aggregate session route. */
export async function verifyCampaignSessionDetailProofDigest(value: unknown): Promise<void> {
    parseCampaignSessionDetail(value);
}

export function parsePlayerRunHistoryPage(value: unknown): PlayerRunHistoryPage {
    const obj = versionedObject(value, 'player_run_history', [
        'player', 'accepted_sequence_watermark', 'runs', 'personal_bests', 'next_cursor',
    ]);
    const player = parsePlayerProfile(obj.player);
    const acceptedSequenceWatermark = nonNegativeInteger(
        obj.accepted_sequence_watermark,
        'player_run_history.accepted_sequence_watermark',
    );
    const runs = array(obj.runs, 'player_run_history.runs').map((value, index) =>
        parsePlayerRunHistoryEntry(value, `player_run_history.runs[${index}]`));
    const personalBests = array(obj.personal_bests, 'player_run_history.personal_bests').map(
        (entry, index) => parsePlayerPersonalBest(
            entry,
            `player_run_history.personal_bests[${index}]`,
        ),
    );
    if (runs.length > 100 || personalBests.length > 512) {
        throw new Error('player_run_history collections exceed protocol limits');
    }
    if (runs.length > 0 && acceptedSequenceWatermark === 0) {
        throw new Error('player_run_history with runs requires a positive snapshot watermark');
    }
    if (new Set(runs.map(entry => entry.run.runId)).size !== runs.length) {
        throw new Error('player_run_history contains duplicate run identities');
    }
    for (const entry of runs) {
        if (entry.playerPublicKey !== player.publicKey) {
            throw new Error('player_run_history run belongs to a different public key');
        }
    }
    const bestFilterDigests = new Set<string>();
    for (const best of personalBests) {
        if (best.filter.playerPublicKey !== player.publicKey) {
            throw new Error('player_run_history personal best belongs to a different public key');
        }
        const filterDigest = canonicalDocumentSha256Sync(playerRunFilterDocument(best.filter));
        if (bestFilterDigests.has(filterDigest)) {
            throw new Error('player_run_history contains duplicate personal-best filters');
        }
        bestFilterDigests.add(filterDigest);
    }
    const nextCursor = obj.next_cursor === null
        ? null
        : boundedString(obj.next_cursor, 'player_run_history.next_cursor', 4096);
    if (runs.length === 0 && nextCursor !== null) {
        throw new Error('an empty player_run_history page cannot have a next cursor');
    }
    return { player, acceptedSequenceWatermark, runs, personalBests, nextCursor };
}

export function parseRuleset(value: unknown, path: string): RulesetFacet {
    const obj = strictObject(value, path, [
        'ruleset_manifest_sha256', 'rules_config_sha256', 'display_name', 'preset_id',
        'preset_name', 'difficulty_id', 'difficulty_name', 'content',
        'categories', 'metrics', 'supports_full_campaign_boards',
    ]);
    const categories = array(obj.categories, `${path}.categories`).map((item, index) =>
        enumeration(item, CATEGORY_ORDER, `${path}.categories[${index}]`));
    const metrics = array(obj.metrics, `${path}.metrics`).map((item, index) =>
        enumeration(item, METRIC_ORDER, `${path}.metrics[${index}]`));
    if (categories.length === 0 || !strictlyIncreasingByOrder(categories, CATEGORY_ORDER)) {
        throw new Error(`${path}.categories must be non-empty and in canonical order without duplicates`);
    }
    if (metrics.length === 0 || !strictlyIncreasingByOrder(metrics, METRIC_ORDER)) {
        throw new Error(`${path}.metrics must be non-empty and in canonical order without duplicates`);
    }
    return {
        id: nonzeroSha256(obj.ruleset_manifest_sha256, `${path}.ruleset_manifest_sha256`),
        rulesConfigSha256: nonzeroSha256(obj.rules_config_sha256, `${path}.rules_config_sha256`),
        label: boundedString(obj.display_name, `${path}.display_name`, 100),
        presetId: opaqueId(obj.preset_id, `${path}.preset_id`),
        presetName: boundedString(obj.preset_name, `${path}.preset_name`, 100),
        difficultyId: opaqueId(obj.difficulty_id, `${path}.difficulty_id`),
        difficultyName: boundedString(obj.difficulty_name, `${path}.difficulty_name`, 100),
        content: parseRunContentIdentity(obj.content, `${path}.content`),
        categories,
        metrics,
        supportsFullCampaign: boolean(obj.supports_full_campaign_boards, `${path}.supports_full_campaign_boards`),
    };
}

export function parseCompetition(value: unknown, path: string): Competition {
    const summary = strictObject(value, path, ['competition_manifest_sha256', 'manifest', 'state']);
    const manifestSha256 = nonzeroSha256(
        summary.competition_manifest_sha256,
        `${path}.competition_manifest_sha256`,
    );
    const obj = versionedObject(summary.manifest, `${path}.manifest`, [
        'competition_id', 'competition_version', 'display_name', 'description', 'subject', 'metric',
        'rules_config_sha256', 'ruleset_manifest_sha256', 'content', 'seed_policy',
        'participant_composition', 'competition_run_grant_public_key',
        'starts_at_unix_ms', 'ends_at_unix_ms',
    ]);
    if (canonicalDocumentSha256Sync(summary.manifest) !== manifestSha256) {
        throw new Error(`${path}.manifest does not match competition_manifest_sha256`);
    }
    const startsAtUnixMs = positiveUnixMilliseconds(obj.starts_at_unix_ms, `${path}.manifest.starts_at_unix_ms`);
    nonzeroSha256(
        obj.competition_run_grant_public_key,
        `${path}.manifest.competition_run_grant_public_key`,
    );
    const endsAtUnixMs = positiveUnixMilliseconds(obj.ends_at_unix_ms, `${path}.manifest.ends_at_unix_ms`);
    if (endsAtUnixMs <= startsAtUnixMs) throw new Error(`${path}.ends_at_unix_ms must follow its start`);
    const seedObj = object(obj.seed_policy, `${path}.manifest.seed_policy`);
    const seedKind = enumeration(seedObj.kind, ['open', 'pinned'] as const, `${path}.manifest.seed_policy.kind`);
    const seedPolicy: Competition['seedPolicy'] = seedKind === 'open'
        ? (assertExactKeys(seedObj, `${path}.manifest.seed_policy`, ['kind']), { kind: 'open' })
        : (assertExactKeys(seedObj, `${path}.manifest.seed_policy`, ['kind', 'simulation_seed']), {
            kind: 'pinned',
            simulationSeed: u64DecimalString(
                seedObj.simulation_seed,
                `${path}.manifest.seed_policy.simulation_seed`,
            ),
        });
    const participantObj = object(obj.participant_composition, `${path}.manifest.participant_composition`);
    const participantKind = enumeration(
        participantObj.kind,
        ['single_player', 'multiplayer'] as const,
        `${path}.manifest.participant_composition.kind`,
    );
    const participantComposition: Competition['participantComposition'] = participantKind === 'single_player'
        ? (assertExactKeys(participantObj, `${path}.manifest.participant_composition`, ['kind']), {
            kind: 'single_player', maxConcurrentPlayers: 1,
        })
        : (assertExactKeys(participantObj, `${path}.manifest.participant_composition`, ['kind', 'max_concurrent_players']), {
            kind: 'multiplayer',
            maxConcurrentPlayers: multiplayerSeatCount(
                participantObj.max_concurrent_players,
                `${path}.manifest.participant_composition.max_concurrent_players`,
            ),
        });
    const subject = parseSubject(obj.subject, `${path}.manifest.subject`);
    const content = parseRunContentIdentity(obj.content, `${path}.manifest.content`, subject);
    return {
        id: opaqueId(obj.competition_id, `${path}.manifest.competition_id`),
        version: u32Positive(obj.competition_version, `${path}.manifest.competition_version`),
        manifestSha256,
        label: boundedString(obj.display_name, `${path}.manifest.display_name`, 100),
        description: boundedString(obj.description, `${path}.manifest.description`, 500),
        subject,
        metric: enumeration(obj.metric, METRIC_ORDER, `${path}.manifest.metric`),
        rulesetId: nonzeroSha256(obj.ruleset_manifest_sha256, `${path}.manifest.ruleset_manifest_sha256`),
        rulesConfigSha256: nonzeroSha256(obj.rules_config_sha256, `${path}.manifest.rules_config_sha256`),
        content,
        seedPolicy,
        participantComposition,
        startsAtUnixMs,
        endsAtUnixMs,
        state: enumeration(summary.state, ['upcoming', 'active', 'ended'] as const, `${path}.state`),
    };
}

export function parseFullCampaignFacet(value: unknown, path: string): FullCampaignFacet {
    const obj = strictObject(value, path, ['display_name', 'description']);
    return {
        label: boundedString(obj.display_name, `${path}.display_name`, 100),
        description: boundedString(obj.description, `${path}.description`, 500),
    };
}

export function parseComposition(value: unknown, path: string): VerifiedRunComposition {
    const obj = object(value, path);
    const kind = enumeration(obj.kind, ['mission', 'full_campaign'] as const, `${path}.kind`);
    if (kind === 'mission') {
        assertExactKeys(obj, path, ['kind', 'replay_sha256']);
        return {
            kind,
            replaySha256: nonzeroSha256(obj.replay_sha256, `${path}.replay_sha256`),
        };
    }
    assertExactKeys(obj, path, ['kind', 'ordered_session_run_ids']);
    const orderedSessionRunIds = array(obj.ordered_session_run_ids, `${path}.ordered_session_run_ids`).map(
        (item, index) => opaqueId(item, `${path}.ordered_session_run_ids[${index}]`),
    );
    if (orderedSessionRunIds.length === 0 || orderedSessionRunIds.length > 4096
        || new Set(orderedSessionRunIds).size !== orderedSessionRunIds.length) {
        throw new Error(`${path}.ordered_session_run_ids must contain 1–4096 unique run IDs`);
    }
    return { kind, orderedSessionRunIds };
}

export function parseRunFilter(value: unknown, path: string): BoardPage['filter'] {
    const obj = versionedObject(value, path, [
        'subject', 'metric', 'content', 'rules_config_sha256',
        'ruleset_manifest_sha256', 'competition_manifest_sha256', 'max_concurrent_players',
    ]);
    const subject = parseSubject(obj.subject, `${path}.subject`);
    const filter = {
        subject,
        metric: enumeration(obj.metric, METRIC_ORDER, `${path}.metric`),
        content: parseRunContentIdentity(obj.content, `${path}.content`, subject),
        rulesConfigSha256: nullableSha256(obj.rules_config_sha256, `${path}.rules_config_sha256`),
        rulesetManifestSha256: nullableSha256(obj.ruleset_manifest_sha256, `${path}.ruleset_manifest_sha256`),
        competitionManifestSha256: nullableSha256(
            obj.competition_manifest_sha256,
            `${path}.competition_manifest_sha256`,
        ),
        maxConcurrentPlayers: obj.max_concurrent_players === null
            ? null
            : replaySeatCount(obj.max_concurrent_players, `${path}.max_concurrent_players`),
    };
    if ((filter.rulesConfigSha256 === null) !== (filter.rulesetManifestSha256 === null)
        || (filter.competitionManifestSha256 !== null && filter.rulesetManifestSha256 === null)) {
        throw new Error(`${path} must select both rules digests or a non-competition combined board`);
    }
    return filter;
}

export function parseRunSummary(value: unknown, path: string): RunSummary {
    const obj = versionedObject(value, path, [
        'run_id', 'subject', 'composition', 'max_concurrent_players',
        'participant_instance_count', 'outcome', 'metrics', 'content',
        'rules_config_sha256', 'ruleset_manifest_sha256', 'competition_manifest_sha256',
    ]);
    const subject = parseSubject(obj.subject, `${path}.subject`);
    const composition = parseComposition(obj.composition, `${path}.composition`);
    const content = parseRunContentIdentity(obj.content, `${path}.content`, subject);
    if (composition.kind !== subject.kind) {
        throw new Error(`${path}.composition does not match its leaderboard subject`);
    }
    const maxConcurrentPlayers = replaySeatCount(
        obj.max_concurrent_players,
        `${path}.max_concurrent_players`,
    );
    const participantInstanceCount = u32Positive(
        obj.participant_instance_count,
        `${path}.participant_instance_count`,
    );
    if (participantInstanceCount < maxConcurrentPlayers) {
        throw new Error(`${path}.participant_instance_count is smaller than its seat count`);
    }
    return {
        runId: opaqueId(obj.run_id, `${path}.run_id`),
        subject,
        composition,
        maxConcurrentPlayers,
        participantInstanceCount,
        outcome: enumeration(obj.outcome, ['won'] as const, `${path}.outcome`),
        metrics: parseRunMetrics(obj.metrics, `${path}.metrics`),
        content,
        rulesConfigSha256: nonzeroSha256(obj.rules_config_sha256, `${path}.rules_config_sha256`),
        rulesetManifestSha256: nonzeroSha256(
            obj.ruleset_manifest_sha256,
            `${path}.ruleset_manifest_sha256`,
        ),
        competitionManifestSha256: nullableSha256(
            obj.competition_manifest_sha256,
            `${path}.competition_manifest_sha256`,
        ),
    };
}

export function parsePlayerRunHistoryEntry(value: unknown, path: string): PlayerRunHistoryEntry {
    const obj = strictObject(value, path, ['player_public_key', 'run', 'verified_at_unix_ms']);
    return {
        playerPublicKey: publicKey(obj.player_public_key, `${path}.player_public_key`),
        run: parseRunSummary(obj.run, `${path}.run`),
        verifiedAtUnixMs: positiveUnixMilliseconds(
            obj.verified_at_unix_ms,
            `${path}.verified_at_unix_ms`,
        ),
    };
}

export function parsePlayerRunFilter(value: unknown, path: string): PlayerRunFilter {
    const obj = versionedObject(value, path, [
        'subject', 'metric', 'content', 'rules_config_sha256',
        'ruleset_manifest_sha256', 'competition_manifest_sha256', 'max_concurrent_players',
        'player_public_key',
    ]);
    const subject = parseSubject(obj.subject, `${path}.subject`);
    return {
        subject,
        metric: enumeration(obj.metric, METRIC_ORDER, `${path}.metric`),
        content: parseRunContentIdentity(obj.content, `${path}.content`, subject),
        rulesConfigSha256: nonzeroSha256(obj.rules_config_sha256, `${path}.rules_config_sha256`),
        rulesetManifestSha256: nonzeroSha256(
            obj.ruleset_manifest_sha256,
            `${path}.ruleset_manifest_sha256`,
        ),
        competitionManifestSha256: nullableSha256(
            obj.competition_manifest_sha256,
            `${path}.competition_manifest_sha256`,
        ),
        maxConcurrentPlayers: obj.max_concurrent_players === null
            ? null
            : replaySeatCount(obj.max_concurrent_players, `${path}.max_concurrent_players`),
        playerPublicKey: publicKey(obj.player_public_key, `${path}.player_public_key`),
    };
}

export function parsePlayerPersonalBest(value: unknown, path: string): PlayerPersonalBest {
    const obj = strictObject(value, path, ['filter', 'run_id', 'metric_value']);
    const filter = parsePlayerRunFilter(obj.filter, `${path}.filter`);
    const metricValue = parseMetricValue(obj.metric_value, `${path}.metric_value`);
    if (filter.metric !== metricValue.metric) {
        throw new Error(`${path}.metric_value does not match its board filter`);
    }
    return { filter, runId: opaqueId(obj.run_id, `${path}.run_id`), metricValue };
}

export function playerRunFilterDocument(filter: PlayerRunFilter): unknown {
    const subject = filter.subject.kind === 'mission'
        ? { kind: 'mission', mission_id: filter.subject.missionId, category: filter.subject.category }
        : { kind: 'full_campaign' };
    const content = filter.content.kind === 'mission'
        ? { kind: 'mission', content_manifest_sha256: filter.content.contentManifestSha256 }
        : {
            kind: 'full_campaign',
            campaign_content_manifest_sha256: filter.content.campaignContentManifestSha256,
        };
    return {
        schema_version: 1,
        subject,
        metric: filter.metric,
        content,
        rules_config_sha256: filter.rulesConfigSha256,
        ruleset_manifest_sha256: filter.rulesetManifestSha256,
        competition_manifest_sha256: filter.competitionManifestSha256,
        max_concurrent_players: filter.maxConcurrentPlayers,
        player_public_key: filter.playerPublicKey,
    };
}

export function parseLeaderboardCursor(value: unknown, path: string): LeaderboardCursor {
    const obj = versionedObject(value, path, [
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
        && metricValuesEqual(left.metricValue, right.metricValue)
        && left.acceptedSequence === right.acceptedSequence
        && left.verifiedAtUnixMs === right.verifiedAtUnixMs
        && left.runId === right.runId;
}

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
        if (current.rank !== prior.rank
            || compareLeaderboardTieTuple(current, prior) <= 0) {
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

export function metricValuesEqual(left: BoardMetricValue, right: BoardMetricValue): boolean {
    return left.metric === right.metric
        && (left.metric === 'original_score'
            ? right.metric === 'original_score' && left.points === right.points
            : right.metric === 'fastest_success'
                && left.activeSimulationTicks === right.activeSimulationTicks
                && left.tickDuration.numeratorMicros === right.tickDuration.numeratorMicros
                && left.tickDuration.denominator === right.tickDuration.denominator);
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
    return left.runId < right.runId ? -1 : left.runId > right.runId ? 1 : 0;
}

export function parseLeaderboardEntry(value: unknown, path: string): LeaderboardEntry {
    const obj = strictObject(value, path, [
        'position', 'rank', 'run_id', 'composition', 'metric_value', 'max_concurrent_players',
        'participant_instance_count', 'named_participant_instance_count', 'named_participants',
        'aggregate_named_participants', 'anonymous_participant_instance_count', 'accepted_sequence',
        'verified_at_unix_ms',
    ]);
    const maxConcurrentPlayers = replaySeatCount(obj.max_concurrent_players, `${path}.max_concurrent_players`);
    const participantInstanceCount = u32Positive(obj.participant_instance_count, `${path}.participant_instance_count`);
    const namedParticipantInstanceCount = u32(
        obj.named_participant_instance_count,
        `${path}.named_participant_instance_count`,
    );
    const namedParticipants = array(obj.named_participants, `${path}.named_participants`).map((item, index) =>
        parseParticipant(item, `${path}.named_participants[${index}]`));
    const aggregateNamedParticipants = array(
        obj.aggregate_named_participants,
        `${path}.aggregate_named_participants`,
    ).map((item, index) => parseAggregateParticipant(item, `${path}.aggregate_named_participants[${index}]`));
    const anonymousParticipantInstanceCount = u32(
        obj.anonymous_participant_instance_count,
        `${path}.anonymous_participant_instance_count`,
    );
    const composition = parseComposition(obj.composition, `${path}.composition`);
    validateRoster(
        maxConcurrentPlayers,
        participantInstanceCount,
        namedParticipantInstanceCount,
        namedParticipants,
        aggregateNamedParticipants,
        anonymousParticipantInstanceCount,
        composition.kind === 'mission',
        path,
    );
    const position = positiveInteger(obj.position, `${path}.position`);
    const rank = positiveInteger(obj.rank, `${path}.rank`);
    if (rank > position) throw new Error(`${path}.rank cannot exceed its snapshot position`);
    return {
        position,
        rank,
        runId: opaqueId(obj.run_id, `${path}.run_id`),
        composition,
        metricValue: parseMetricValue(obj.metric_value, `${path}.metric_value`),
        maxConcurrentPlayers,
        participantInstanceCount,
        namedParticipantInstanceCount,
        namedParticipants,
        aggregateNamedParticipants,
        anonymousParticipantInstanceCount,
        acceptedSequence: positiveInteger(obj.accepted_sequence, `${path}.accepted_sequence`),
        verifiedAtUnixMs: positiveUnixMilliseconds(obj.verified_at_unix_ms, `${path}.verified_at_unix_ms`),
    };
}
