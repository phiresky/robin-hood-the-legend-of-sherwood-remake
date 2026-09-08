// Run/campaign proof decoding and cross-document consistency checks.
import {
    type ParsedVerificationProof,
    type PublicPlaybackProof,
    type InputProvenance,
    type PublicBuild,
    type ViewerLaunch,
    type ViewerContentRequirement,
    type ParsedPublicVerificationRequest,
    type ArtifactRef,
    type ParsedCampaignSessionKind,
    type ParsedAchievementDecision,
    type ParsedCampaignAggregate,
    type ParsedAggregateSession,
    type ParsedFullSession,
    type CommonRunProof,
    type RunArtifacts,
    type ExpectedVerificationProof,
    type Achievement,
} from './types.js';
import {
    object,
    enumeration,
    assertExactKeys,
    array,
    strictObject,
    TAINT_ORDER,
    u32,
    strictlyIncreasingByOrder,
    nonzeroSha256,
    gitCommit,
    boundedString,
    versionedObject,
    replaySeatCount,
    participantCount,
    u16,
    i32,
    strictlySorted,
    u32Positive,
    u64DecimalString,
    nullableSha256,
    positiveInteger,
    publicKey,
    protocolValuesEqual,
    opaqueId,
    equalArrays,
    checkedAdd,
} from './decode.js';
import { canonicalDocumentSha256Sync } from './canonical.js';
import { parseCampaignArtifactRef, parseReplayArtifact } from './build-contract.js';
import {
    parseRunMetrics,
    compareSeatPublicKey,
    parseParticipant,
    validateRoster,
    metricsEqual,
    provenanceEqual,
    campaignSessionKindsEqual,
    achievementsEqual,
} from './participants-metrics.js';
import { parseOfficialContentSubject } from './content-contract.js';
import { parseMission } from './subject-contract.js';

export function publicPlaybackProof(proof: ParsedVerificationProof): PublicPlaybackProof {
    if (proof.outcome !== 'won') {
        throw new Error('A watchable public playback proof must have a Won terminal outcome.');
    }
    return {
        replay: proof.replay,
        scopeKind: proof.scopeKind,
        campaignSessionKind: proof.campaignSessionKind,
        campaignSessionOrdinal: proof.campaignSessionOrdinal,
        startingCampaign: proof.startingCampaign,
        finalCampaign: proof.finalCampaign,
        finalStateSha256: proof.finalStateSha256,
        replayFrameCount: proof.replayFrameCount,
        outcome: proof.outcome,
    };
}

export function parseInputProvenance(value: unknown, path: string): InputProvenance {
    const obj = object(value, path);
    const status = enumeration(obj.status, ['rankable', 'tainted'] as const, `${path}.status`);
    if (status !== 'tainted') {
        assertExactKeys(obj, path, ['status']);
        return { status };
    }
    assertExactKeys(obj, path, ['status', 'taints']);
    const taints = array(obj.taints, `${path}.taints`).map((item, index) => {
        const taintPath = `${path}.taints[${index}]`;
        const taint = strictObject(item, taintPath, ['kind', 'first_frame']);
        return {
            kind: enumeration(taint.kind, TAINT_ORDER, `${taintPath}.kind`),
            firstFrame: u32(taint.first_frame, `${taintPath}.first_frame`),
        };
    });
    if (taints.length === 0 || !strictlyIncreasingByOrder(taints.map(taint => taint.kind), TAINT_ORDER)) {
        throw new Error(`${path}.taints must be non-empty and in canonical order without duplicate kinds`);
    }
    return { status, taints };
}

export function parsePublicBuild(value: unknown, path: string): PublicBuild {
    const obj = strictObject(value, path, ['manifest_sha256', 'source_commit', 'display_name']);
    return {
        manifestSha256: nonzeroSha256(obj.manifest_sha256, `${path}.manifest_sha256`),
        sourceCommit: gitCommit(obj.source_commit, `${path}.source_commit`),
        displayName: boundedString(obj.display_name, `${path}.display_name`, 100),
    };
}

export function parseViewerLaunch(value: unknown, path: string): ViewerLaunch {
    const obj = strictObject(value, path, ['build_manifest_sha256', 'availability']);
    const availabilityObj = object(obj.availability, `${path}.availability`);
    const status = enumeration(availabilityObj.status, ['available', 'unavailable'] as const, `${path}.availability.status`);
    let availability: ViewerLaunch['availability'];
    if (status === 'available') {
        assertExactKeys(availabilityObj, `${path}.availability`, ['status', 'content_requirement']);
        availability = {
            status,
            contentRequirement: parseViewerContentRequirement(
                availabilityObj.content_requirement,
                `${path}.availability.content_requirement`,
            ),
        };
    } else {
        assertExactKeys(availabilityObj, `${path}.availability`, ['status', 'safe_reason']);
        availability = {
            status,
            safeReason: boundedString(availabilityObj.safe_reason, `${path}.availability.safe_reason`, 500),
        };
    }
    return {
        buildManifestSha256: nonzeroSha256(obj.build_manifest_sha256, `${path}.build_manifest_sha256`),
        availability,
    };
}

export function parseViewerContentRequirement(value: unknown, path: string): ViewerContentRequirement {
    const obj = strictObject(value, path, ['kind', 'content_manifest_sha256']);
    const kind = enumeration(obj.kind, ['bundled_demo', 'user_local_retail'] as const, `${path}.kind`);
    return { kind, contentManifestSha256: nonzeroSha256(obj.content_manifest_sha256, `${path}.content_manifest_sha256`) };
}

export function parseVerificationProof(value: unknown, path: string): ParsedVerificationProof {
    const obj = versionedObject(value, path, [
        'public_request', 'public_request_sha256', 'input_provenance',
        'campaign_session_kind', 'campaign_session_ordinal',
        'starting_campaign', 'final_campaign', 'final_state_sha256',
        'replay_frame_count', 'outcome', 'metrics', 'starting_campaign_score', 'final_campaign_score',
        'max_concurrent_players', 'participant_instance_count', 'named_participant_instance_count',
        'anonymous_participant_instance_count', 'campaign_complete_evidence', 'achievements',
    ]);
    const request = parsePublicVerificationRequest(obj.public_request, `${path}.public_request`);
    const publicRequestSha256 = nonzeroSha256(obj.public_request_sha256, `${path}.public_request_sha256`);
    if (canonicalDocumentSha256Sync(obj.public_request) !== publicRequestSha256) {
        throw new Error(`${path}.public_request_sha256 does not match the canonical public request`);
    }
    const campaignSessionKind = obj.campaign_session_kind === null
        ? null
        : parseCampaignSessionKind(obj.campaign_session_kind, `${path}.campaign_session_kind`);
    const campaignSessionOrdinal = obj.campaign_session_ordinal === null
        ? null
        : u32(obj.campaign_session_ordinal, `${path}.campaign_session_ordinal`);
    if (request.scopeKind === 'individual_level') {
        if (request.campaignAggregationConsent !== 'not_authorized'
            || campaignSessionKind !== null || campaignSessionOrdinal !== null) {
            throw new Error(`${path} individual-level run cannot authorize campaign aggregation`);
        }
    } else if (request.campaignAggregationConsent !== 'authorize_signed_session_in_server_recognized_chain_v1'
        || campaignSessionKind === null || campaignSessionOrdinal === null || campaignSessionOrdinal >= 4096) {
        throw new Error(`${path} campaign run must bind its authorized chain session`);
    }
    const maxConcurrentPlayers = replaySeatCount(obj.max_concurrent_players, `${path}.max_concurrent_players`);
    const participantInstanceCount = participantCount(
        obj.participant_instance_count,
        `${path}.participant_instance_count`,
    );
    const namedParticipantInstanceCount = u16(
        obj.named_participant_instance_count,
        `${path}.named_participant_instance_count`,
    );
    const anonymousParticipantInstanceCount = u16(
        obj.anonymous_participant_instance_count,
        `${path}.anonymous_participant_instance_count`,
    );
    if (participantInstanceCount < maxConcurrentPlayers
        || namedParticipantInstanceCount + anonymousParticipantInstanceCount !== participantInstanceCount
        || maxConcurrentPlayers !== request.maxConcurrentPlayers
        || participantInstanceCount !== request.participantInstanceCount
        || namedParticipantInstanceCount !== request.namedParticipantInstanceCount
        || anonymousParticipantInstanceCount !== request.anonymousParticipantInstanceCount) {
        throw new Error(`${path} participant identity counts are inconsistent`);
    }
    const startingCampaignScore = i32(obj.starting_campaign_score, `${path}.starting_campaign_score`);
    const finalCampaignScore = i32(obj.final_campaign_score, `${path}.final_campaign_score`);
    const outcome = enumeration(obj.outcome, ['won', 'lost', 'interrupted'] as const, `${path}.outcome`);
    if (outcome !== 'won') throw new Error(`${path}.outcome must be won`);
    const finalCampaign = parseCampaignArtifactRef(obj.final_campaign, `${path}.final_campaign`);
    const finalStateSha256 = nonzeroSha256(obj.final_state_sha256, `${path}.final_state_sha256`);
    const campaignCompleteEvidence = obj.campaign_complete_evidence === null
        ? null
        : parsePublicCampaignCompleteEvidence(
            obj.campaign_complete_evidence,
            request,
            publicRequestSha256,
            finalCampaign,
            finalStateSha256,
            `${path}.campaign_complete_evidence`,
        );
    const publicCampaignCompleteEvidenceSha256 = campaignCompleteEvidence === null
        ? null
        : canonicalDocumentSha256Sync(obj.campaign_complete_evidence);
    if (campaignCompleteEvidence !== null && request.scopeKind !== 'campaign') {
        throw new Error(`${path}.campaign_complete_evidence requires a successful campaign session`);
    }
    const achievements = array(obj.achievements, `${path}.achievements`).map((item, index) =>
        parseAchievementDecision(item, `${path}.achievements[${index}]`));
    if (achievements.length > 256 || !strictlySorted(achievements.map(achievement => achievement.id))) {
        throw new Error(`${path}.achievements must be canonical and contain at most 256 decisions`);
    }
    return {
        publicResultSha256: canonicalDocumentSha256Sync(value),
        publicRequestSha256,
        replay: request.replay,
        contentEdition: request.contentEdition,
        buildManifestSha256: request.buildManifestSha256,
        contentManifestSha256: request.contentManifestSha256,
        rulesConfigSha256: request.rulesConfigSha256,
        rulesetManifestSha256: request.rulesetManifestSha256,
        competitionManifestSha256: request.competitionManifestSha256,
        inputProvenance: parseInputProvenance(obj.input_provenance, `${path}.input_provenance`),
        scopeKind: request.scopeKind,
        campaignAggregationConsent: request.campaignAggregationConsent,
        campaignSessionKind,
        campaignSessionOrdinal,
        maxConcurrentPlayers,
        participantInstanceCount,
        namedParticipantInstanceCount,
        anonymousParticipantInstanceCount,
        replayFrameCount: u32Positive(obj.replay_frame_count, `${path}.replay_frame_count`),
        outcome,
        startingCampaign: parseCampaignArtifactRef(
            obj.starting_campaign,
            `${path}.starting_campaign`,
        ),
        finalCampaign,
        finalStateSha256,
        startingCampaignScore,
        finalCampaignScore,
        publicCampaignCompleteEvidenceSha256,
        namedParticipants: request.namedParticipants,
        achievements,
        metrics: parseRunMetrics(obj.metrics, `${path}.metrics`),
    };
}

export function parsePublicVerificationRequest(value: unknown, path: string): ParsedPublicVerificationRequest {
    const obj = versionedObject(value, path, [
        'replay', 'content_edition', 'content_subject', 'simulation_seed', 'scope_kind',
        'campaign_aggregation_consent', 'build_manifest_sha256', 'content_manifest_sha256',
        'campaign_content_manifest_sha256', 'rules_config_sha256', 'ruleset_manifest_sha256',
        'competition_manifest_sha256', 'requested_metrics', 'limits', 'max_concurrent_players',
        'participant_instance_count', 'named_participant_instance_count',
        'anonymous_participant_instance_count', 'named_participants',
    ]);
    const contentEdition = enumeration(
        obj.content_edition,
        ['demo', 'full'] as const,
        `${path}.content_edition`,
    );
    const contentSubject = parseOfficialContentSubject(obj.content_subject, `${path}.content_subject`);
    u64DecimalString(obj.simulation_seed, `${path}.simulation_seed`);
    const scopeKind = enumeration(obj.scope_kind, ['individual_level', 'campaign'] as const, `${path}.scope_kind`);
    const campaignAggregationConsent = enumeration(
        obj.campaign_aggregation_consent,
        ['not_authorized', 'authorize_signed_session_in_server_recognized_chain_v1'] as const,
        `${path}.campaign_aggregation_consent`,
    );
    const campaignContentManifestSha256 = nullableSha256(
        obj.campaign_content_manifest_sha256,
        `${path}.campaign_content_manifest_sha256`,
    );
    if ((scopeKind === 'individual_level'
        && (campaignAggregationConsent !== 'not_authorized' || campaignContentManifestSha256 !== null))
        || (scopeKind === 'campaign'
            && (campaignAggregationConsent !== 'authorize_signed_session_in_server_recognized_chain_v1'
                || campaignContentManifestSha256 === null))) {
        throw new Error(`${path} has an invalid public scope tuple`);
    }
    const requestedMetrics = array(obj.requested_metrics, `${path}.requested_metrics`).map((metric, index) =>
        enumeration(metric, ['original_score', 'fastest_success'] as const, `${path}.requested_metrics[${index}]`));
    const metricOrder = ['original_score', 'fastest_success'] as const;
    if (requestedMetrics.length === 0 || !strictlyIncreasingByOrder(requestedMetrics, metricOrder)) {
        throw new Error(`${path}.requested_metrics must be non-empty and canonical`);
    }
    const limits = strictObject(obj.limits, `${path}.limits`, [
        'max_input_bytes', 'max_compressed_bytes', 'max_decompressed_bytes',
        'max_base64_payload_bytes', 'max_campaign_bytes', 'max_frames', 'max_version_bytes',
        'max_mission_id_bytes', 'max_metadata_records', 'max_entries_per_frame',
    ]);
    for (const [name, limit] of Object.entries(limits)) positiveInteger(limit, `${path}.limits.${name}`);
    const maxConcurrentPlayers = replaySeatCount(obj.max_concurrent_players, `${path}.max_concurrent_players`);
    const participantInstanceCount = participantCount(obj.participant_instance_count, `${path}.participant_instance_count`);
    const namedParticipantInstanceCount = u16(obj.named_participant_instance_count, `${path}.named_participant_instance_count`);
    const anonymousParticipantInstanceCount = u16(
        obj.anonymous_participant_instance_count,
        `${path}.anonymous_participant_instance_count`,
    );
    const namedParticipants = array(obj.named_participants, `${path}.named_participants`).map((item, index) => {
        const claim = strictObject(item, `${path}.named_participants[${index}]`, [
            'seat', 'public_key',
        ]);
        const seat = u16(claim.seat, `${path}.named_participants[${index}].seat`);
        if (seat >= 64) throw new Error(`${path}.named_participants[${index}].seat exceeds the replay seat range`);
        return {
            seat,
            publicKey: publicKey(claim.public_key, `${path}.named_participants[${index}].public_key`),
        };
    });
    if (participantInstanceCount < maxConcurrentPlayers
        || namedParticipantInstanceCount + anonymousParticipantInstanceCount !== participantInstanceCount
        || namedParticipants.length !== namedParticipantInstanceCount
        || new Set(namedParticipants.map(participant => participant.publicKey)).size !== namedParticipants.length
        || namedParticipants.some((participant, index) => index > 0
            && compareSeatPublicKey(namedParticipants[index - 1]!, participant) >= 0)) {
        throw new Error(`${path} has invalid named participant claims`);
    }
    return {
        replay: parseReplayArtifact(obj.replay),
        contentEdition,
        contentSubject,
        scopeKind,
        campaignAggregationConsent,
        buildManifestSha256: nonzeroSha256(obj.build_manifest_sha256, `${path}.build_manifest_sha256`),
        contentManifestSha256: nonzeroSha256(obj.content_manifest_sha256, `${path}.content_manifest_sha256`),
        campaignContentManifestSha256,
        rulesConfigSha256: nonzeroSha256(obj.rules_config_sha256, `${path}.rules_config_sha256`),
        rulesetManifestSha256: nonzeroSha256(obj.ruleset_manifest_sha256, `${path}.ruleset_manifest_sha256`),
        competitionManifestSha256: nullableSha256(obj.competition_manifest_sha256, `${path}.competition_manifest_sha256`),
        maxConcurrentPlayers,
        participantInstanceCount,
        namedParticipantInstanceCount,
        anonymousParticipantInstanceCount,
        namedParticipants,
    };
}

export function parsePublicCampaignCompleteEvidence(
    value: unknown,
    request: ParsedPublicVerificationRequest,
    publicRequestSha256: string,
    finalCampaign: ArtifactRef,
    finalStateSha256: string,
    path: string,
): void {
    const obj = versionedObject(value, path, [
        'campaign_content_manifest_sha256', 'content_manifest_sha256', 'rules_config_sha256',
        'ruleset_manifest_sha256', 'public_verification_request_sha256', 'terminal_subject',
        'final_campaign', 'final_state_sha256', 'observed_progression_percent',
    ]);
    const progression = positiveInteger(obj.observed_progression_percent, `${path}.observed_progression_percent`);
    if (progression > 100
        || nonzeroSha256(obj.campaign_content_manifest_sha256, `${path}.campaign_content_manifest_sha256`)
            !== request.campaignContentManifestSha256
        || nonzeroSha256(obj.content_manifest_sha256, `${path}.content_manifest_sha256`)
            !== request.contentManifestSha256
        || nonzeroSha256(obj.rules_config_sha256, `${path}.rules_config_sha256`) !== request.rulesConfigSha256
        || nonzeroSha256(obj.ruleset_manifest_sha256, `${path}.ruleset_manifest_sha256`)
            !== request.rulesetManifestSha256
        || nonzeroSha256(obj.public_verification_request_sha256, `${path}.public_verification_request_sha256`)
            !== publicRequestSha256
        || !protocolValuesEqual(parseOfficialContentSubject(obj.terminal_subject, `${path}.terminal_subject`), request.contentSubject)
        || !protocolValuesEqual(parseCampaignArtifactRef(obj.final_campaign, `${path}.final_campaign`), finalCampaign)
        || nonzeroSha256(obj.final_state_sha256, `${path}.final_state_sha256`) !== finalStateSha256) {
        throw new Error(`${path} does not bind the public request and terminal state`);
    }
}

export function parseCampaignSessionKind(value: unknown, path: string): ParsedCampaignSessionKind {
    const obj = object(value, path);
    const kind = enumeration(obj.kind, ['field_mission', 'headquarters'] as const, `${path}.kind`);
    if (kind === 'field_mission') {
        assertExactKeys(obj, path, ['kind', 'mission_id']);
        return { kind, missionId: boundedString(obj.mission_id, `${path}.mission_id`, 256) };
    }
    assertExactKeys(obj, path, ['kind', 'hq_sequence']);
    return { kind, hqSequence: u32Positive(obj.hq_sequence, `${path}.hq_sequence`) };
}

export function parseAchievementDecision(value: unknown, path: string): ParsedAchievementDecision {
    const obj = strictObject(value, path, ['achievement_id', 'evaluation']);
    return {
        id: opaqueId(obj.achievement_id, `${path}.achievement_id`),
        evaluation: enumeration(
            obj.evaluation,
            ['unverifiable', 'not_earned', 'earned'] as const,
            `${path}.evaluation`,
        ),
    };
}

export function parseCampaignAggregate(value: unknown, path: string): ParsedCampaignAggregate {
    const obj = versionedObject(value, path, [
        'public_request', 'public_request_sha256', 'max_concurrent_players',
        'participant_instance_count', 'named_participant_instance_count',
        'anonymous_participant_instance_count',
        'canonical_genesis_campaign', 'final_campaign', 'starting_campaign_score',
        'final_campaign_score', 'metrics',
    ]);
    const request = parsePublicCampaignAggregateRequest(obj.public_request, `${path}.public_request`);
    const publicRequestSha256 = nonzeroSha256(obj.public_request_sha256, `${path}.public_request_sha256`);
    if (canonicalDocumentSha256Sync(obj.public_request) !== publicRequestSha256) {
        throw new Error(`${path}.public_request_sha256 does not match the canonical public aggregate request`);
    }
    const maxConcurrentPlayers = replaySeatCount(obj.max_concurrent_players, `${path}.max_concurrent_players`);
    const participantInstanceCount = u32Positive(obj.participant_instance_count, `${path}.participant_instance_count`);
    const namedParticipantInstanceCount = u32(
        obj.named_participant_instance_count,
        `${path}.named_participant_instance_count`,
    );
    const anonymousParticipantInstanceCount = u32(
        obj.anonymous_participant_instance_count,
        `${path}.anonymous_participant_instance_count`,
    );
    if (participantInstanceCount < maxConcurrentPlayers
        || namedParticipantInstanceCount + anonymousParticipantInstanceCount !== participantInstanceCount) {
        throw new Error(`${path} has inconsistent aggregate participant counts`);
    }
    const startingCampaignScore = i32(obj.starting_campaign_score, `${path}.starting_campaign_score`);
    const finalCampaignScore = i32(obj.final_campaign_score, `${path}.final_campaign_score`);
    const canonicalGenesisCampaign = parseCampaignArtifactRef(
        obj.canonical_genesis_campaign,
        `${path}.canonical_genesis_campaign`,
    );
    const finalCampaign = parseCampaignArtifactRef(
        obj.final_campaign,
        `${path}.final_campaign`,
    );
    const scoreDelta = finalCampaignScore - startingCampaignScore;
    const metrics = parseRunMetrics(obj.metrics, `${path}.metrics`);
    if (scoreDelta !== metrics.originalScoreDelta) throw new Error(`${path} score metric does not match its campaign delta`);
    return {
        publicResultSha256: canonicalDocumentSha256Sync(value),
        publicRequestSha256,
        fullCampaignRunId: request.fullCampaignRunId,
        campaignCompleteTerminalRunId: request.campaignCompleteTerminalRunId,
        publicCampaignCompleteEvidenceSha256: request.publicCampaignCompleteEvidenceSha256,
        sessions: request.sessions,
        maxConcurrentPlayers,
        participantInstanceCount,
        namedParticipantInstanceCount,
        anonymousParticipantInstanceCount,
        campaignContentManifestSha256: request.campaignContentManifestSha256,
        rulesConfigSha256: request.rulesConfigSha256,
        rulesetManifestSha256: request.rulesetManifestSha256,
        competitionManifestSha256: request.competitionManifestSha256,
        canonicalGenesisCampaign,
        finalCampaign,
        startingCampaignScore,
        finalCampaignScore,
        metrics,
    };
}

export function parsePublicCampaignAggregateRequest(value: unknown, path: string): {
    readonly fullCampaignRunId: string;
    readonly campaignCompleteTerminalRunId: string;
    readonly publicCampaignCompleteEvidenceSha256: string;
    readonly sessions: readonly ParsedAggregateSession[];
    readonly campaignContentManifestSha256: string;
    readonly rulesConfigSha256: string;
    readonly rulesetManifestSha256: string;
    readonly competitionManifestSha256: string | null;
} {
    const obj = versionedObject(value, path, [
        'full_campaign_run_id', 'campaign_complete_terminal_run_id',
        'public_campaign_complete_evidence_sha256', 'sessions',
        'campaign_content_manifest_sha256', 'rules_config_sha256', 'ruleset_manifest_sha256',
        'competition_manifest_sha256',
    ]);
    const sessions = array(obj.sessions, `${path}.sessions`).map((item, index) =>
        parseAggregateSession(item, `${path}.sessions[${index}]`));
    const campaignCompleteTerminalRunId = opaqueId(
        obj.campaign_complete_terminal_run_id,
        `${path}.campaign_complete_terminal_run_id`,
    );
    if (sessions.length === 0 || sessions.length > 4096
        || new Set(sessions.map(session => session.runId)).size !== sessions.length
        || sessions.some((session, index) => session.ordinal !== index)
        || sessions.at(-1)?.runId !== campaignCompleteTerminalRunId) {
        throw new Error(`${path}.sessions must be gap-free, unique, and end at the completion run`);
    }
    return {
        fullCampaignRunId: opaqueId(obj.full_campaign_run_id, `${path}.full_campaign_run_id`),
        campaignCompleteTerminalRunId,
        publicCampaignCompleteEvidenceSha256: nonzeroSha256(
            obj.public_campaign_complete_evidence_sha256,
            `${path}.public_campaign_complete_evidence_sha256`,
        ),
        sessions,
        campaignContentManifestSha256: nonzeroSha256(
            obj.campaign_content_manifest_sha256,
            `${path}.campaign_content_manifest_sha256`,
        ),
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

export function parseAggregateSession(value: unknown, path: string): ParsedAggregateSession {
    const obj = strictObject(value, path, [
        'ordinal', 'run_id', 'public_verification_request_sha256',
        'public_verification_result_sha256',
    ]);
    return {
        ordinal: u32(obj.ordinal, `${path}.ordinal`),
        runId: opaqueId(obj.run_id, `${path}.run_id`),
        publicVerificationRequestSha256: nonzeroSha256(
            obj.public_verification_request_sha256,
            `${path}.public_verification_request_sha256`,
        ),
        publicVerificationResultSha256: nonzeroSha256(
            obj.public_verification_result_sha256,
            `${path}.public_verification_result_sha256`,
        ),
    };
}

export function parseFullCampaignSession(value: unknown, path: string): ParsedFullSession {
    const obj = strictObject(value, path, [
        'ordinal', 'run_id', 'session', 'content_subject', 'display_name', 'mission', 'replay',
        'public_verification_request_sha256',
        'public_verification_result_sha256', 'verification_proof', 'starting_campaign',
        'final_campaign', 'starting_campaign_score', 'final_campaign_score',
        'public_campaign_complete_evidence_sha256', 'content_manifest_sha256', 'rules_config_sha256',
        'ruleset_manifest_sha256', 'competition_manifest_sha256', 'max_concurrent_players',
        'participant_instance_count', 'named_participant_instance_count',
        'anonymous_participant_instance_count', 'named_participants', 'input_provenance',
        'metrics', 'achievements', 'build', 'viewer',
    ]);
    const session = parseCampaignSessionKind(obj.session, `${path}.session`);
    const contentSubject = parseOfficialContentSubject(obj.content_subject, `${path}.content_subject`);
    const kind = session.kind;
    const mission = obj.mission === null ? null : parseMission(obj.mission, `${path}.mission`);
    if ((session.kind === 'field_mission'
        && (contentSubject.kind !== 'field_mission'
            || contentSubject.missionId !== session.missionId
            || mission === null
            || mission.id !== session.missionId))
        || (session.kind === 'headquarters'
            && (contentSubject.kind !== 'headquarters' || mission !== null))) {
        throw new Error(`${path}.session, content subject, and mission facet do not match`);
    }
    const label = boundedString(obj.display_name, `${path}.display_name`, 100);
    const ordinal = u32(obj.ordinal, `${path}.ordinal`);
    const replay = parseReplayArtifact(obj.replay);
    const build = parsePublicBuild(obj.build, `${path}.build`);
    const viewer = parseViewerLaunch(obj.viewer, `${path}.viewer`);
    if (build.manifestSha256 !== viewer.buildManifestSha256) {
        throw new Error(`${path}.viewer does not match the authenticated build manifest`);
    }
    const maxConcurrentPlayers = replaySeatCount(obj.max_concurrent_players, `${path}.max_concurrent_players`);
    const participantInstanceCount = participantCount(
        obj.participant_instance_count,
        `${path}.participant_instance_count`,
    );
    const namedParticipantInstanceCount = u16(
        obj.named_participant_instance_count,
        `${path}.named_participant_instance_count`,
    );
    const anonymousParticipantInstanceCount = u16(
        obj.anonymous_participant_instance_count,
        `${path}.anonymous_participant_instance_count`,
    );
    const namedParticipants = array(obj.named_participants, `${path}.named_participants`).map((item, index) =>
        parseParticipant(item, `${path}.named_participants[${index}]`));
    validateRoster(
        maxConcurrentPlayers,
        participantInstanceCount,
        namedParticipantInstanceCount,
        namedParticipants,
        [],
        anonymousParticipantInstanceCount,
        true,
        path,
    );
    const inputProvenance = parseInputProvenance(obj.input_provenance, `${path}.input_provenance`);
    if (inputProvenance.status !== 'rankable') throw new Error(`${path}.input_provenance must be rankable`);
    const metrics = parseRunMetrics(obj.metrics, `${path}.metrics`);
    const startingCampaignScore = i32(obj.starting_campaign_score, `${path}.starting_campaign_score`);
    const finalCampaignScore = i32(obj.final_campaign_score, `${path}.final_campaign_score`);
    const wrappedDelta = (finalCampaignScore - startingCampaignScore) >>> 0;
    if (metrics.originalScoreDelta !== wrappedDelta) throw new Error(`${path}.metrics score does not match campaign scores`);
    const contentManifestSha256 = nonzeroSha256(obj.content_manifest_sha256, `${path}.content_manifest_sha256`);
    if (mission !== null && mission.contentManifestSha256 !== contentManifestSha256) {
        throw new Error(`${path}.session mission does not match its content manifest`);
    }
    const verificationProof = parseVerificationProof(obj.verification_proof, `${path}.verification_proof`);
    validateViewerContent(
        viewer,
        contentManifestSha256,
        verificationProof.contentEdition,
        `${path}.viewer`,
    );
    return {
        ordinal,
        runId: opaqueId(obj.run_id, `${path}.run_id`),
        kind,
        missionId: session.kind === 'field_mission' ? session.missionId : null,
        headquartersSequence: session.kind === 'headquarters' ? session.hqSequence : null,
        contentSubject,
        label,
        mission,
        replay,
        replayFrameCount: verificationProof.replayFrameCount,
        playbackProof: publicPlaybackProof(verificationProof),
        publicVerificationRequestSha256: nonzeroSha256(
            obj.public_verification_request_sha256,
            `${path}.public_verification_request_sha256`,
        ),
        publicVerificationResultSha256: nonzeroSha256(
            obj.public_verification_result_sha256,
            `${path}.public_verification_result_sha256`,
        ),
        verificationProof,
        startingCampaign: parseCampaignArtifactRef(
            obj.starting_campaign,
            `${path}.starting_campaign`,
        ),
        finalCampaign: parseCampaignArtifactRef(
            obj.final_campaign,
            `${path}.final_campaign`,
        ),
        startingCampaignScore,
        finalCampaignScore,
        publicCampaignCompleteEvidenceSha256: nullableSha256(
            obj.public_campaign_complete_evidence_sha256,
            `${path}.public_campaign_complete_evidence_sha256`,
        ),
        maxConcurrentPlayers,
        participantInstanceCount,
        namedParticipantInstanceCount,
        anonymousParticipantInstanceCount,
        contentManifestSha256,
        rulesConfigSha256: nonzeroSha256(obj.rules_config_sha256, `${path}.rules_config_sha256`),
        rulesetManifestSha256: nonzeroSha256(obj.ruleset_manifest_sha256, `${path}.ruleset_manifest_sha256`),
        competitionManifestSha256: nullableSha256(
            obj.competition_manifest_sha256,
            `${path}.competition_manifest_sha256`,
        ),
        namedParticipants,
        inputProvenance,
        metrics,
        achievements: array(obj.achievements, `${path}.achievements`).map((item, index) =>
            parseAchievement(item, `${path}.achievements[${index}]`)),
        build,
        viewer,
    };
}

export function validateMissionRunArtifacts(common: CommonRunProof, artifacts: RunArtifacts): void {
    if (common.subject.kind !== 'mission' || artifacts.composition.kind !== 'mission'
        || artifacts.mission === null || artifacts.build === null || artifacts.viewer === null
        || artifacts.verificationProof === null || artifacts.campaignAggregate !== null
        || artifacts.replay === null || artifacts.fullCampaignSessions.length !== 0) {
        throw new Error('run mission subject has an invalid artifact/proof shape');
    }
    if (artifacts.mission.id !== common.subject.missionId
        || artifacts.mission.contentManifestSha256 !== common.contentManifestSha256
        || artifacts.build.manifestSha256 !== artifacts.viewer.buildManifestSha256
        || artifacts.replay.artifact.sha256 !== artifacts.composition.replaySha256) {
        throw new Error('run mission artifacts do not bind the exact subject, content, and build');
    }
    validateViewerContent(
        artifacts.viewer,
        common.contentManifestSha256,
        artifacts.verificationProof.contentEdition,
        'run.viewer',
    );
    const expectedScope = common.subject.category === 'campaign' ? 'campaign' : 'individual_level';
    validateVerificationProof(artifacts.verificationProof, {
        publicResultSha256: common.publicResultSha256,
        publicRequestSha256: common.publicRequestSha256,
        replay: artifacts.replay,
        buildManifestSha256: artifacts.build.manifestSha256,
        contentManifestSha256: common.contentManifestSha256,
        rulesConfigSha256: common.rulesConfigSha256,
        rulesetManifestSha256: common.rulesetManifestSha256,
        competitionManifestSha256: common.competitionManifestSha256,
        inputProvenance: common.inputProvenance,
        scopeKind: expectedScope,
        missionId: common.subject.missionId,
        maxConcurrentPlayers: common.maxConcurrentPlayers,
        participantInstanceCount: common.participantInstanceCount,
        namedParticipantInstanceCount: common.namedParticipantInstanceCount,
        anonymousParticipantInstanceCount: common.anonymousParticipantInstanceCount,
        namedParticipants: common.namedParticipants,
        startingCampaign: common.startingCampaign,
        finalCampaign: common.finalCampaign,
        startingCampaignScore: common.startingCampaignScore,
        finalCampaignScore: common.finalCampaignScore,
        achievements: common.achievements,
        metrics: common.metrics,
    }, 'run.verification_proof');
    const wrappedScoreDelta = (common.finalCampaignScore - common.startingCampaignScore) >>> 0;
    if (common.metrics.originalScoreDelta !== wrappedScoreDelta) {
        throw new Error('run mission score does not match its campaign-state transition');
    }
}

export function validateFullCampaignArtifacts(common: CommonRunProof, artifacts: RunArtifacts): void {
    if (common.subject.kind !== 'full_campaign' || artifacts.composition.kind !== 'full_campaign'
        || artifacts.mission !== null || artifacts.build !== null || artifacts.viewer !== null
        || artifacts.verificationProof !== null || artifacts.campaignAggregate === null
        || artifacts.replay !== null) {
        throw new Error('run full-campaign subject has an invalid aggregate artifact/proof shape');
    }
    if (common.achievements.length !== 0) {
        throw new Error('run full-campaign aggregate cannot publish synthetic achievements');
    }
    const sessions = artifacts.fullCampaignSessions;
    if (!equalArrays(artifacts.composition.orderedSessionRunIds, sessions.map(session => session.runId))) {
        throw new Error('run full-campaign session order does not match its composition');
    }
    const aggregate = artifacts.campaignAggregate;
    const disclosedSessionKeys = [...new Set(sessions.flatMap(session =>
        session.namedParticipants.map(participant => participant.publicKey)))].sort();
    if (aggregate.publicResultSha256 !== common.publicResultSha256
        || aggregate.publicRequestSha256 !== common.publicRequestSha256
        || aggregate.fullCampaignRunId !== common.runId
        || !equalArrays(aggregate.sessions.map(session => session.runId), artifacts.composition.orderedSessionRunIds)
        || aggregate.maxConcurrentPlayers !== common.maxConcurrentPlayers
        || aggregate.participantInstanceCount !== common.participantInstanceCount
        || aggregate.namedParticipantInstanceCount !== common.namedParticipantInstanceCount
        || aggregate.anonymousParticipantInstanceCount !== common.anonymousParticipantInstanceCount
        || !equalArrays(disclosedSessionKeys, common.aggregateNamedParticipants.map(participant => participant.publicKey))
        || common.content.kind !== 'full_campaign'
        || aggregate.campaignContentManifestSha256 !== common.content.campaignContentManifestSha256
        || aggregate.rulesConfigSha256 !== common.rulesConfigSha256
        || aggregate.rulesetManifestSha256 !== common.rulesetManifestSha256
        || aggregate.competitionManifestSha256 !== common.competitionManifestSha256
        || !protocolValuesEqual(aggregate.canonicalGenesisCampaign, common.startingCampaign)
        || !protocolValuesEqual(aggregate.finalCampaign, common.finalCampaign)
        || aggregate.startingCampaignScore !== common.startingCampaignScore
        || aggregate.finalCampaignScore !== common.finalCampaignScore
        || !metricsEqual(aggregate.metrics, common.metrics)
        || aggregate.sessions.length !== sessions.length) {
        throw new Error('run full-campaign aggregate proof does not bind the exact public result');
    }
    for (const [index, session] of sessions.entries()) {
        const proof = aggregate.sessions[index];
        if (proof === undefined
            || proof.ordinal !== session.ordinal
            || proof.runId !== session.runId
            || proof.publicVerificationRequestSha256 !== session.publicVerificationRequestSha256
            || proof.publicVerificationResultSha256 !== session.publicVerificationResultSha256) {
            throw new Error(`run full-campaign session ${index} does not match its aggregate proof`);
        }
        validateFullSessionBinding(session, common, index);
    }
    const first = sessions[0];
    const last = sessions[sessions.length - 1];
    if (first === undefined || last === undefined
        || !protocolValuesEqual(first.startingCampaign, common.startingCampaign)
        || first.startingCampaignScore !== common.startingCampaignScore
        || !protocolValuesEqual(last.finalCampaign, common.finalCampaign)
        || last.finalCampaignScore !== common.finalCampaignScore
        || last.runId !== aggregate.campaignCompleteTerminalRunId
        || last.publicCampaignCompleteEvidenceSha256
            !== aggregate.publicCampaignCompleteEvidenceSha256
        || sessions.slice(0, -1).some(session => session.publicCampaignCompleteEvidenceSha256 !== null)
        || sessions.slice(1).some((session, index) => {
            const previous = sessions[index];
            return previous === undefined
                || !protocolValuesEqual(previous.finalCampaign, session.startingCampaign)
                || previous.finalCampaignScore !== session.startingCampaignScore;
        })) {
        throw new Error('run full-campaign public sessions do not form a continuous chain');
    }
    const scoreDelta = common.finalCampaignScore - common.startingCampaignScore;
    if (scoreDelta < 0 || common.metrics.originalScoreDelta !== scoreDelta
        || sessions.reduce((sum, session) => checkedAdd(
            sum,
            session.finalCampaignScore - session.startingCampaignScore,
            'run full-campaign score',
        ), 0) !== scoreDelta
        || sessions.reduce((sum, session) => checkedAdd(
            sum,
            session.metrics.activeSimulationTicks,
            'run full-campaign ticks',
        ), 0) !== common.metrics.activeSimulationTicks
        || sessions.reduce((sum, session) => checkedAdd(
            sum,
            session.metrics.ransomCollected,
            'run full-campaign ransom',
        ), 0) !== common.metrics.ransomCollected) {
        throw new Error('run full-campaign metrics do not equal its ordered public sessions');
    }
}

export function validateFullSessionBinding(session: ParsedFullSession, common: CommonRunProof, index: number): void {
    const path = `run.full_campaign_sessions[${index}]`;
    if (session.rulesConfigSha256 !== common.rulesConfigSha256
        || session.rulesetManifestSha256 !== common.rulesetManifestSha256
        || session.competitionManifestSha256 !== common.competitionManifestSha256
        || session.build.manifestSha256 !== session.viewer.buildManifestSha256) {
        throw new Error(`${path} does not bind the aggregate identity and roster`);
    }
    validateFullSessionProof(session, path);
}

export function validateFullSessionProof(session: ParsedFullSession, path: string): void {
    validateVerificationProof(session.verificationProof, {
        publicResultSha256: session.publicVerificationResultSha256,
        publicRequestSha256: session.publicVerificationRequestSha256,
        replay: session.replay,
        buildManifestSha256: session.build.manifestSha256,
        contentManifestSha256: session.contentManifestSha256,
        rulesConfigSha256: session.rulesConfigSha256,
        rulesetManifestSha256: session.rulesetManifestSha256,
        competitionManifestSha256: session.competitionManifestSha256,
        inputProvenance: session.inputProvenance,
        scopeKind: 'campaign',
        missionId: session.missionId,
        campaignSession: session.kind === 'field_mission'
            ? { kind: 'field_mission', missionId: session.missionId! }
            : { kind: 'headquarters', hqSequence: session.headquartersSequence! },
        campaignSessionOrdinal: session.ordinal,
        maxConcurrentPlayers: session.maxConcurrentPlayers,
        participantInstanceCount: session.participantInstanceCount,
        namedParticipantInstanceCount: session.namedParticipantInstanceCount,
        anonymousParticipantInstanceCount: session.anonymousParticipantInstanceCount,
        namedParticipants: session.namedParticipants,
        startingCampaign: session.startingCampaign,
        finalCampaign: session.finalCampaign,
        startingCampaignScore: session.startingCampaignScore,
        finalCampaignScore: session.finalCampaignScore,
        publicCampaignCompleteEvidenceSha256: session.publicCampaignCompleteEvidenceSha256,
        achievements: session.achievements,
        metrics: session.metrics,
    }, `${path}.verification_proof`);
}

export function validateVerificationProof(
    proof: ParsedVerificationProof,
    expected: ExpectedVerificationProof,
    path: string,
): void {
    if (proof.publicResultSha256 !== expected.publicResultSha256
        || proof.publicRequestSha256 !== expected.publicRequestSha256
        || !protocolValuesEqual(proof.replay, expected.replay)
        || proof.buildManifestSha256 !== expected.buildManifestSha256
        || proof.contentManifestSha256 !== expected.contentManifestSha256
        || proof.rulesConfigSha256 !== expected.rulesConfigSha256
        || proof.rulesetManifestSha256 !== expected.rulesetManifestSha256
        || proof.competitionManifestSha256 !== expected.competitionManifestSha256
        || !provenanceEqual(proof.inputProvenance, expected.inputProvenance)) {
        throw new Error(`${path} outer proof identities do not match the public run`);
    }
    const campaignShapeMatches = expected.scopeKind === 'individual_level'
        ? proof.campaignAggregationConsent === 'not_authorized'
            && proof.campaignSessionKind === null && proof.campaignSessionOrdinal === null
        : proof.campaignAggregationConsent === 'authorize_signed_session_in_server_recognized_chain_v1'
            && proof.campaignSessionKind !== null
            && (expected.campaignSession === undefined
                ? proof.campaignSessionKind.kind === 'field_mission'
                    && proof.campaignSessionKind.missionId === expected.missionId
                : campaignSessionKindsEqual(proof.campaignSessionKind, expected.campaignSession))
            && (expected.campaignSessionOrdinal === undefined
                || proof.campaignSessionOrdinal === expected.campaignSessionOrdinal);
    if (proof.scopeKind !== expected.scopeKind
        || !campaignShapeMatches
        || proof.maxConcurrentPlayers !== expected.maxConcurrentPlayers
        || proof.participantInstanceCount !== expected.participantInstanceCount
        || proof.namedParticipantInstanceCount !== expected.namedParticipantInstanceCount
        || proof.anonymousParticipantInstanceCount !== expected.anonymousParticipantInstanceCount
        || proof.namedParticipants.length !== expected.namedParticipants.length
        || proof.namedParticipants.some((participant, index) => {
            const expectedParticipant = expected.namedParticipants[index];
            return expectedParticipant === undefined
                || participant.seat !== expectedParticipant.seat
                || participant.publicKey !== expectedParticipant.publicKey;
        })
        || proof.outcome !== 'won'
        || !protocolValuesEqual(proof.startingCampaign, expected.startingCampaign)
        || !protocolValuesEqual(proof.finalCampaign, expected.finalCampaign)
        || proof.startingCampaignScore !== expected.startingCampaignScore
        || proof.finalCampaignScore !== expected.finalCampaignScore
        || (expected.publicCampaignCompleteEvidenceSha256 !== undefined
            && proof.publicCampaignCompleteEvidenceSha256
                !== expected.publicCampaignCompleteEvidenceSha256)
        || !achievementsEqual(proof.achievements, expected.achievements)
        || !metricsEqual(proof.metrics, expected.metrics)) {
        throw new Error(`${path} verified result was substituted or does not match the public run`);
    }
}

export function validateViewerContent(
    viewer: ViewerLaunch,
    contentManifestSha256: string,
    contentEdition: 'demo' | 'full',
    path: string,
): void {
    if (viewer.availability.status !== 'available') return;
    const requirement = viewer.availability.contentRequirement;
    if (requirement.contentManifestSha256 !== contentManifestSha256) {
        throw new Error(`${path} content requirement does not match the verified run content`);
    }
    const exactKind = contentEdition === 'demo' ? 'bundled_demo' : 'user_local_retail';
    if (requirement.kind !== exactKind) {
        throw new Error(`${path} content requirement does not match the verified ${contentEdition} edition`);
    }
}

export function parseAchievement(value: unknown, path: string): Achievement {
    const obj = strictObject(value, path, ['display_name', 'verified']);
    const verified = parseAchievementDecision(obj.verified, `${path}.verified`);
    return {
        id: verified.id,
        label: boundedString(obj.display_name, `${path}.display_name`, 100),
        evaluation: verified.evaluation,
    };
}
