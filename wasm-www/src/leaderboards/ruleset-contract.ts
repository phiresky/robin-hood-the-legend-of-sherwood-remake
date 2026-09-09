// Immutable ruleset and simulation policy contracts.
import {
    type RulesConfigIdentity,
    type RankedSimulationPolicy,
    type CampaignCompletionPolicyRequirement,
    type CanonicalCampaignStateRequirement,
    type RulesetManifest,
    type RulesetBoardScope,
    type PublishedRuleset,
    type ParticipantEligibility,
    type ImmutablePolicyKind,
    type ImmutablePolicyIdentity,
} from './types.js';
import {
    versionedObject,
    u32Positive,
    strictObject,
    enumeration,
    object,
    assertExactKeys,
    nonzeroSha256,
    array,
    METRIC_ORDER,
    strictlyIncreasingByOrder,
    equalArrays,
    opaqueId,
    strictlySorted,
    boundedString,
    boolean,
    positiveUnixMilliseconds,
    replaySeatCount,
    participantCount,
} from './decode.js';
import { parseCanonicalMap, verifyCanonicalDocument, type DigestVerified } from './canonical.js';
import { parseOfficialContentSubject } from './content-contract.js';
import { CURRENT_RANKED_REPLAY_SCHEMA_VERSION } from './build-contract.js';
import { parseTickDuration } from './participants-metrics.js';
import { rankedSimulationPolicyLabels } from './policy-display.js';

export function parseRulesConfigIdentity(value: unknown): RulesConfigIdentity {
    const obj = versionedObject(value, 'rules_config', [
        'replay_schema_version', 'ranked_simulation_policy', 'sim_config', 'rules',
    ]);
    const rankedSimulationPolicy = parseRankedSimulationPolicy(obj.ranked_simulation_policy);
    const simConfig = parseCanonicalMap(obj.sim_config, 'rules_config.sim_config');
    const rules = parseCanonicalMap(obj.rules, 'rules_config.rules');
    if (Object.keys(simConfig).length === 0 || Object.keys(rules).length === 0) {
        throw new Error('rules_config sim_config and rules maps must both be non-empty');
    }
    const expectedDifficulty = { easy: 'Easy', medium: 'Medium', hard: 'Hard', legendary: 'Legendary', custom: 'Custom' }[rankedSimulationPolicy.difficulty];
    if (rankedSimulationPolicy.difficulty === 'custom') {
        const difficulty = strictObject(simConfig.difficulty, 'rules_config.sim_config.difficulty', ['Custom']);
        object(difficulty.Custom, 'rules_config.sim_config.difficulty.Custom');
    } else if (simConfig.difficulty !== expectedDifficulty) {
        throw new Error('rules_config.ranked_simulation_policy.difficulty does not match sim_config.difficulty');
    }
    return {
        replaySchemaVersion: u32Positive(obj.replay_schema_version, 'rules_config.replay_schema_version'),
        rankedSimulationPolicy,
        simConfig,
        rules,
    };
}

export function parseRankedSimulationPolicy(value: unknown): RankedSimulationPolicy {
    const obj = strictObject(value, 'rules_config.ranked_simulation_policy', [
        'version', 'preset', 'difficulty',
    ]);
    if (obj.version !== 1) {
        throw new Error('rules_config.ranked_simulation_policy.version must be 1');
    }
    if (obj.preset !== 'custom' && (obj.difficulty === 'custom' || obj.difficulty === 'legendary')) {
        throw new Error('rules_config.ranked_simulation_policy.difficulty requires custom policy');
    }
    return {
        version: 1,
        preset: enumeration(
            obj.preset,
            ['standard', 'original_parity', 'custom'] as const,
            'rules_config.ranked_simulation_policy.preset',
        ),
        difficulty: enumeration(
            obj.difficulty,
            ['easy', 'medium', 'hard', 'legendary', 'custom'] as const,
            'rules_config.ranked_simulation_policy.difficulty',
        ),
    };
}

export function parseCampaignCompletionPolicy(
    value: unknown,
    path: string,
): CampaignCompletionPolicyRequirement {
    const obj = object(value, path);
    const mode = enumeration(obj.mode, ['not_offered', 'required'] as const, `${path}.mode`);
    if (mode === 'not_offered') {
        assertExactKeys(obj, path, ['mode']);
        return { mode };
    }
    assertExactKeys(obj, path, ['mode', 'policy']);
    const policy = strictObject(obj.policy, `${path}.policy`, [
        'terminal_subject', 'required_progression_percent',
    ]);
    const requiredProgressionPercent = u32Positive(
        policy.required_progression_percent,
        `${path}.policy.required_progression_percent`,
    );
    if (requiredProgressionPercent > 100) {
        throw new Error(`${path}.policy.required_progression_percent must be between 1 and 100`);
    }
    return {
        mode,
        policy: {
            terminalSubject: parseOfficialContentSubject(
                policy.terminal_subject,
                `${path}.policy.terminal_subject`,
            ),
            requiredProgressionPercent,
        },
    };
}

export function parseCanonicalCampaignState(
    value: unknown,
    path: string,
): CanonicalCampaignStateRequirement {
    const obj = strictObject(value, path, ['edition', 'kind', 'rules_config_sha256']);
    const edition = enumeration(obj.edition, ['demo', 'full'] as const, `${path}.edition`);
    const kind = enumeration(
        obj.kind,
        ['individual_template', 'full_campaign_genesis'] as const,
        `${path}.kind`,
    );
    if ((edition === 'demo') !== (kind === 'individual_template')) {
        throw new Error(`${path}.edition and kind do not identify the same canonical state family`);
    }
    return {
        edition,
        kind,
        rulesConfigSha256: nonzeroSha256(obj.rules_config_sha256, `${path}.rules_config_sha256`),
    };
}

export function parseRulesetManifest(value: unknown): RulesetManifest {
    const obj = versionedObject(value, 'ruleset_manifest', [
        'display_name', 'preset_id', 'preset_name', 'difficulty_id', 'difficulty_name',
        'rules_config_sha256', 'rules_config_constraint', 'allowed_build_manifest_sha256',
        'allowed_content_manifest_sha256', 'allowed_campaign_content_manifest_sha256',
        'board_scopes', 'campaign_completion_policy', 'metrics', 'metric_ranking',
        'achievement_policies', 'canonical_start_policy', 'canonical_campaign_state',
        'run_preflight_grant_public_key', 'full_campaign_chain_policy', 'campaign_roster_continuity',
        'campaign_aggregation_consent_policy', 'participant_eligibility', 'replay_schema_versions',
        'network_protocol_versions', 'input_provenance_policy', 'command_admission_policy',
        'submission_admission_policy', 'verifier_policy', 'input_provenance_eligibility',
        'terminal_result_policy', 'score_algorithm', 'score_overflow_policy', 'visible_tie_policy',
        'pagination_tie_break', 'tick_duration', 'active_time_definition', 'frame_counting_policy',
        'full_campaign_time_aggregation', 'run_composition_policy', 'main_board_seed_policy',
        'competition_seed_policy', 'allow_save_creation', 'allow_autosave', 'allow_state_load',
        'allow_mission_restart',
    ]);
    const rulesConfigSha256 = nonzeroSha256(
        obj.rules_config_sha256,
        'ruleset_manifest.rules_config_sha256',
    );
    const scopeOrder = ['individual_level', 'campaign_mission', 'full_campaign'] as const;
    const boardScopes = array(obj.board_scopes, 'ruleset_manifest.board_scopes').map((scope, index) =>
        enumeration(scope, scopeOrder, `ruleset_manifest.board_scopes[${index}]`));
    const metrics = array(obj.metrics, 'ruleset_manifest.metrics').map((metric, index) =>
        enumeration(metric, METRIC_ORDER, `ruleset_manifest.metrics[${index}]`));
    if (boardScopes.length === 0 || !strictlyIncreasingByOrder(boardScopes, scopeOrder)) {
        throw new Error('ruleset_manifest.board_scopes must be non-empty and in canonical order');
    }
    if (metrics.length === 0 || !strictlyIncreasingByOrder(metrics, METRIC_ORDER)) {
        throw new Error('ruleset_manifest.metrics must be non-empty and in canonical order');
    }
    const metricRankingOrder = ['original_score_descending', 'fastest_success_ascending'] as const;
    const metricRanking = array(obj.metric_ranking, 'ruleset_manifest.metric_ranking').map((policy, index) =>
        enumeration(policy, metricRankingOrder, `ruleset_manifest.metric_ranking[${index}]`));
    if (!strictlyIncreasingByOrder(metricRanking, metricRankingOrder)
        || !equalArrays(metricRanking.map(policy => policy === 'original_score_descending'
            ? 'original_score'
            : 'fastest_success'), metrics)) {
        throw new Error('ruleset_manifest.metric_ranking must exactly and canonically match metrics');
    }
    const achievementPolicies = array(
        obj.achievement_policies,
        'ruleset_manifest.achievement_policies',
    ).map((item, index) => {
        const path = `ruleset_manifest.achievement_policies[${index}]`;
        const policy = strictObject(item, path, ['achievement_id', 'mode']);
        return {
            achievementId: opaqueId(policy.achievement_id, `${path}.achievement_id`),
            mode: enumeration(policy.mode, ['required', 'reported'] as const, `${path}.mode`),
        };
    });
    if (achievementPolicies.length === 0
        || !strictlySorted(achievementPolicies.map(policy => policy.achievementId))) {
        throw new Error('ruleset_manifest.achievement_policies must be non-empty and strictly ordered');
    }
    const allowedBuildManifestSha256 = parseSortedDigests(
        obj.allowed_build_manifest_sha256,
        'ruleset_manifest.allowed_build_manifest_sha256',
    );
    const allowedContentManifestSha256 = parseSortedDigests(
        obj.allowed_content_manifest_sha256,
        'ruleset_manifest.allowed_content_manifest_sha256',
    );
    const allowedCampaignContentManifestSha256 = parseSortedDigests(
        obj.allowed_campaign_content_manifest_sha256,
        'ruleset_manifest.allowed_campaign_content_manifest_sha256',
        true,
    );
    if (boardScopes.includes('full_campaign') !== (allowedCampaignContentManifestSha256.length > 0)) {
        throw new Error('ruleset_manifest campaign content allowlist does not match full-campaign scope');
    }
    const campaignCompletionPolicy = parseCampaignCompletionPolicy(
        obj.campaign_completion_policy,
        'ruleset_manifest.campaign_completion_policy',
    );
    if (boardScopes.includes('full_campaign') !== (campaignCompletionPolicy.mode === 'required')) {
        throw new Error('ruleset_manifest campaign completion policy does not match full-campaign scope');
    }
    const canonicalCampaignState = parseCanonicalCampaignState(
        obj.canonical_campaign_state,
        'ruleset_manifest.canonical_campaign_state',
    );
    if (canonicalCampaignState.rulesConfigSha256 !== rulesConfigSha256) {
        throw new Error('ruleset_manifest canonical campaign state does not match rules_config_sha256');
    }
    const runPreflightGrantPublicKey = nonzeroSha256(
        obj.run_preflight_grant_public_key,
        'ruleset_manifest.run_preflight_grant_public_key',
    );
    const replaySchemaVersions = parseSortedPositiveU32s(
        obj.replay_schema_versions,
        'ruleset_manifest.replay_schema_versions',
    );
    const networkProtocolVersions = parseSortedPositiveU32s(
        obj.network_protocol_versions,
        'ruleset_manifest.network_protocol_versions',
    );
    const inputProvenanceEligibility = enumeration(
        obj.input_provenance_eligibility,
        ['current_schema_canonical_replay_only'] as const,
        'ruleset_manifest.input_provenance_eligibility',
    );
    if (!equalArrays(replaySchemaVersions, [CURRENT_RANKED_REPLAY_SCHEMA_VERSION])) {
        throw new Error(
            'ruleset_manifest input provenance policy does not match replay schema versions',
        );
    }
    return {
        displayName: boundedString(obj.display_name, 'ruleset_manifest.display_name', 100),
        presetId: opaqueId(obj.preset_id, 'ruleset_manifest.preset_id'),
        presetName: boundedString(obj.preset_name, 'ruleset_manifest.preset_name', 100),
        difficultyId: opaqueId(obj.difficulty_id, 'ruleset_manifest.difficulty_id'),
        difficultyName: boundedString(obj.difficulty_name, 'ruleset_manifest.difficulty_name', 100),
        rulesConfigSha256,
        rulesConfigConstraint: enumeration(
            obj.rules_config_constraint,
            ['exact_canonical_digest_only', 'any_canonical_sim_config'] as const,
            'ruleset_manifest.rules_config_constraint',
        ),
        allowedBuildManifestSha256,
        allowedContentManifestSha256,
        allowedCampaignContentManifestSha256,
        boardScopes,
        campaignCompletionPolicy,
        metrics,
        metricRanking,
        achievementPolicies,
        canonicalStartPolicy: enumeration(
            obj.canonical_start_policy,
            ['rules_config_bound_operator_state_and_verified_predecessor', 'rules_config_bound_mission_setup_and_verified_predecessor'] as const,
            'ruleset_manifest.canonical_start_policy',
        ),
        canonicalCampaignState,
        runPreflightGrantPublicKey,
        fullCampaignChainPolicy: enumeration(
            obj.full_campaign_chain_policy,
            ['canonical_genesis_every_field_and_headquarters_session_independent_completion'] as const,
            'ruleset_manifest.full_campaign_chain_policy',
        ),
        campaignRosterContinuity: enumeration(
            obj.campaign_roster_continuity,
            ['union_of_verified_session_subsets', 'exact_same_authenticated_keys_every_session'] as const,
            'ruleset_manifest.campaign_roster_continuity',
        ),
        campaignAggregationConsentPolicy: enumeration(
            obj.campaign_aggregation_consent_policy,
            ['every_authenticated_key_final_cosigns_each_session'] as const,
            'ruleset_manifest.campaign_aggregation_consent_policy',
        ),
        participantEligibility: parseParticipantEligibility(
            obj.participant_eligibility,
            'ruleset_manifest.participant_eligibility',
        ),
        replaySchemaVersions,
        networkProtocolVersions,
        inputProvenancePolicy: parseImmutablePolicyIdentity(
            obj.input_provenance_policy,
            'input_provenance',
            'ruleset_manifest.input_provenance_policy',
        ),
        commandAdmissionPolicy: parseImmutablePolicyIdentity(
            obj.command_admission_policy,
            'command_admission',
            'ruleset_manifest.command_admission_policy',
        ),
        submissionAdmissionPolicy: parseImmutablePolicyIdentity(
            obj.submission_admission_policy,
            'submission_admission',
            'ruleset_manifest.submission_admission_policy',
        ),
        verifierPolicy: parseImmutablePolicyIdentity(
            obj.verifier_policy,
            'verification',
            'ruleset_manifest.verifier_policy',
        ),
        inputProvenanceEligibility,
        terminalResultPolicy: enumeration(
            obj.terminal_result_policy,
            ['independently_reached_won_only'] as const,
            'ruleset_manifest.terminal_result_policy',
        ),
        scoreAlgorithm: enumeration(
            obj.score_algorithm,
            ['original_mission_attempt_wrapping_subtotal_campaign_delta_v1'] as const,
            'ruleset_manifest.score_algorithm',
        ),
        scoreOverflowPolicy: enumeration(
            obj.score_overflow_policy,
            ['reject_campaign_or_aggregate_overflow'] as const,
            'ruleset_manifest.score_overflow_policy',
        ),
        visibleTiePolicy: enumeration(
            obj.visible_tie_policy,
            ['equal_primary_metric_shares_rank'] as const,
            'ruleset_manifest.visible_tie_policy',
        ),
        paginationTieBreak: enumeration(
            obj.pagination_tie_break,
            ['accepted_sequence_then_verification_time_then_run_id_only'] as const,
            'ruleset_manifest.pagination_tie_break',
        ),
        tickDuration: parseTickDuration(obj.tick_duration, 'ruleset_manifest.tick_duration'),
        activeTimeDefinition: enumeration(
            obj.active_time_definition,
            ['successful_simulation_ticks'] as const,
            'ruleset_manifest.active_time_definition',
        ),
        frameCountingPolicy: enumeration(
            obj.frame_counting_policy,
            ['zero_based_events_before_exclusive_replay_frame_count'] as const,
            'ruleset_manifest.frame_counting_policy',
        ),
        fullCampaignTimeAggregation: enumeration(
            obj.full_campaign_time_aggregation,
            ['checked_sum_every_verified_field_and_headquarters_session'] as const,
            'ruleset_manifest.full_campaign_time_aggregation',
        ),
        runCompositionPolicy: enumeration(
            obj.run_composition_policy,
            ['mission_single_replay_full_campaign_ordered_sessions_no_synthetic_replay'] as const,
            'ruleset_manifest.run_composition_policy',
        ),
        mainBoardSeedPolicy: enumeration(
            obj.main_board_seed_policy,
            ['open', 'server_pinned'] as const,
            'ruleset_manifest.main_board_seed_policy',
        ),
        competitionSeedPolicy: enumeration(
            obj.competition_seed_policy,
            ['open', 'server_pinned'] as const,
            'ruleset_manifest.competition_seed_policy',
        ),
        allowSaveCreation: boolean(obj.allow_save_creation, 'ruleset_manifest.allow_save_creation'),
        allowAutosave: boolean(obj.allow_autosave, 'ruleset_manifest.allow_autosave'),
        allowStateLoad: boolean(obj.allow_state_load, 'ruleset_manifest.allow_state_load'),
        allowMissionRestart: boolean(obj.allow_mission_restart, 'ruleset_manifest.allow_mission_restart'),
    };
}

export function validateRulesetRulesConfigBinding(
    ruleset: RulesetManifest,
    rulesConfig: RulesConfigIdentity,
): void {
    if (!ruleset.replaySchemaVersions.includes(rulesConfig.replaySchemaVersion)) {
        throw new Error('The rules configuration replay schema is outside its immutable ruleset allowlist.');
    }
    if (ruleset.rulesConfigConstraint === 'any_canonical_sim_config') {
        if (ruleset.presetId !== 'any' || ruleset.difficultyId !== 'any') throw new Error('Open ruleset labels are invalid.');
        return;
    }
    const labels = rankedSimulationPolicyLabels(rulesConfig.rankedSimulationPolicy);
    if (ruleset.presetId !== labels.presetId
        || ruleset.presetName !== labels.presetName
        || ruleset.difficultyId !== labels.difficultyId
        || ruleset.difficultyName !== labels.difficultyName) {
        throw new Error('The rules configuration simulation policy does not match its immutable ruleset labels.');
    }
}

export function validateRulesetFacetScopeBinding(
    ruleset: RulesetManifest,
    offeredScopes: readonly RulesetBoardScope[],
): void {
    if (offeredScopes.some(scope => !ruleset.boardScopes.includes(scope))) {
        throw new Error('The immutable ruleset manifest does not include every scope offered by its board facet.');
    }
}

export async function parseAndVerifyRulesConfigIdentity(
    value: unknown,
    expectedSha256: string,
): Promise<DigestVerified<RulesConfigIdentity>> {
    const parsed = parseRulesConfigIdentity(value);
    return await verifyCanonicalDocument(value, parsed, expectedSha256, 'rules_config');
}

export async function parseAndVerifyRulesetManifest(
    value: unknown,
    expectedSha256: string,
): Promise<DigestVerified<RulesetManifest>> {
    const parsed = parseRulesetManifest(value);
    return await verifyCanonicalDocument(value, parsed, expectedSha256, 'ruleset_manifest');
}

export async function parseAndVerifyPublishedRuleset(
    value: unknown,
    expectedSha256: string,
): Promise<PublishedRuleset> {
    const obj = versionedObject(value, 'published_ruleset', [
        'ruleset_manifest_sha256', 'manifest', 'operational_status',
    ]);
    const rulesetManifestSha256 = nonzeroSha256(
        obj.ruleset_manifest_sha256,
        'published_ruleset.ruleset_manifest_sha256',
    );
    if (rulesetManifestSha256 !== nonzeroSha256(expectedSha256, 'published_ruleset expected digest')) {
        throw new Error('published_ruleset identity does not match its requested route');
    }
    const manifest = await parseAndVerifyRulesetManifest(obj.manifest, rulesetManifestSha256);
    const statusObj = object(obj.operational_status, 'published_ruleset.operational_status');
    const status = enumeration(
        statusObj.status,
        ['active', 'quarantined'] as const,
        'published_ruleset.operational_status.status',
    );
    let operationalStatus: PublishedRuleset['operationalStatus'];
    if (status === 'active') {
        assertExactKeys(statusObj, 'published_ruleset.operational_status', ['status']);
        operationalStatus = { status };
    } else {
        assertExactKeys(statusObj, 'published_ruleset.operational_status', [
            'status', 'audit_id', 'reason_code', 'since_unix_ms',
        ]);
        operationalStatus = {
            status,
            auditId: opaqueId(statusObj.audit_id, 'published_ruleset.operational_status.audit_id'),
            reasonCode: boundedString(
                statusObj.reason_code,
                'published_ruleset.operational_status.reason_code',
                128,
            ),
            sinceUnixMs: positiveUnixMilliseconds(
                statusObj.since_unix_ms,
                'published_ruleset.operational_status.since_unix_ms',
            ),
        };
    }
    return { rulesetManifestSha256, manifest, operationalStatus };
}

export function parseParticipantEligibility(value: unknown, path: string): ParticipantEligibility {
    const obj = strictObject(value, path, [
        'allow_single_player', 'allow_multiplayer', 'named_policy', 'anonymous_policy',
        'minimum_max_concurrent_players', 'maximum_max_concurrent_players',
        'maximum_participant_instances',
    ]);
    const allowSinglePlayer = boolean(obj.allow_single_player, `${path}.allow_single_player`);
    const allowMultiplayer = boolean(obj.allow_multiplayer, `${path}.allow_multiplayer`);
    const minimumMaxConcurrentPlayers = replaySeatCount(
        obj.minimum_max_concurrent_players,
        `${path}.minimum_max_concurrent_players`,
    );
    const maximumMaxConcurrentPlayers = replaySeatCount(
        obj.maximum_max_concurrent_players,
        `${path}.maximum_max_concurrent_players`,
    );
    const maximumParticipantInstances = participantCount(
        obj.maximum_participant_instances,
        `${path}.maximum_participant_instances`,
    );
    if ((!allowSinglePlayer && !allowMultiplayer)
        || minimumMaxConcurrentPlayers > maximumMaxConcurrentPlayers
        || maximumParticipantInstances < maximumMaxConcurrentPlayers
        || (!allowMultiplayer && maximumMaxConcurrentPlayers !== 1)) {
        throw new Error(`${path} has inconsistent participant eligibility bounds`);
    }
    return {
        allowSinglePlayer,
        allowMultiplayer,
        namedPolicy: enumeration(
            obj.named_policy,
            ['host_genesis_guest_transport_join_attestation_and_final_cosign'] as const,
            `${path}.named_policy`,
        ),
        anonymousPolicy: enumeration(
            obj.anonymous_policy,
            ['allowed_authenticated_but_publicly_redacted', 'forbidden'] as const,
            `${path}.anonymous_policy`,
        ),
        minimumMaxConcurrentPlayers,
        maximumMaxConcurrentPlayers,
        maximumParticipantInstances,
    };
}

export function parseImmutablePolicyIdentity(
    value: unknown,
    expectedKind: ImmutablePolicyKind,
    path: string,
): ImmutablePolicyIdentity {
    const obj = strictObject(value, path, ['kind', 'version', 'manifest_sha256']);
    const kind = enumeration(obj.kind, [
        'input_provenance', 'command_admission', 'submission_admission', 'verification',
    ] as const, `${path}.kind`);
    if (kind !== expectedKind) throw new Error(`${path}.kind does not match its ruleset field`);
    return {
        kind,
        version: u32Positive(obj.version, `${path}.version`),
        manifestSha256: nonzeroSha256(obj.manifest_sha256, `${path}.manifest_sha256`),
    };
}

export function parseSortedDigests(value: unknown, path: string, allowEmpty = false): readonly string[] {
    const digests = array(value, path).map((digest, index) => nonzeroSha256(digest, `${path}[${index}]`));
    if ((!allowEmpty && digests.length === 0) || !strictlySorted(digests)) {
        throw new Error(`${path} must be unique and strictly sorted${allowEmpty ? '' : ' and non-empty'}`);
    }
    return digests;
}

export function parseSortedPositiveU32s(value: unknown, path: string): readonly number[] {
    const versions = array(value, path).map((version, index) => u32Positive(version, `${path}[${index}]`));
    if (versions.length === 0 || versions.some((version, index) => index > 0 && versions[index - 1]! >= version)) {
        throw new Error(`${path} must be non-empty, unique, and strictly sorted`);
    }
    return versions;
}
