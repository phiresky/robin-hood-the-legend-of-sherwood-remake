import runtimeContract from '../../runtime-contract.json' with { type: 'json' };
import { createHash } from 'node:crypto';

export const sha = (character: string): string => character.repeat(64);
export const publicKey = '1'.repeat(64);
export const publicFingerprint = '24752c730a3eea290df5600aacb45161';
export const secondPublicKey = '2'.repeat(64);
export const secondPublicFingerprint = '94ca02d832b9e0d11b766cf04148bccf';

export function playerHistoryPage(): Record<string, unknown> {
    return {
        schema_version: 1,
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
                schema_version: 1,
                run_id: 'run-9',
                subject: { kind: 'mission', mission_id: 'm01', category: 'individual_level' },
                composition: { kind: 'mission', replay_sha256: sha('a') },
                max_concurrent_players: 1,
                participant_instance_count: 1,
                outcome: 'won',
                metrics: {
                    original_score_delta: 120,
                    active_simulation_ticks: 240,
                    ransom_collected: 4,
                },
                content: { kind: 'mission', content_manifest_sha256: sha('b') },
                rules_config_sha256: sha('c'),
                ruleset_manifest_sha256: sha('d'),
                competition_manifest_sha256: null,
            },
            verified_at_unix_ms: 1_700_000_000_000,
        }],
        personal_bests: [{
            filter: {
                schema_version: 1,
                subject: { kind: 'mission', mission_id: 'm01', category: 'individual_level' },
                metric: 'original_score',
                content: { kind: 'mission', content_manifest_sha256: sha('b') },
                rules_config_sha256: sha('c'),
                ruleset_manifest_sha256: sha('d'),
                competition_manifest_sha256: null,
                max_concurrent_players: 1,
                player_public_key: publicKey,
            },
            run_id: 'run-9',
            metric_value: { metric: 'original_score', points: 120 },
        }],
        next_cursor: 'authenticated-cursor',
    };
}
export const campaignArtifact = (digest: string): Record<string, unknown> => ({
    sha256: digest,
    byte_length: 321,
    media_type: 'application/x-robin-campaign+bitcode',
});
export const replayArtifact = (digest: string): Record<string, unknown> => ({
    artifact: {
        sha256: digest,
        byte_length: 123,
        media_type: 'application/x-robin-rhrec+compact',
    },
    replay_schema_version: runtimeContract.replaySchema,
});
export const publicParticipant = {
    seat: 0,
    username: 'Robin',
    public_key: publicKey,
    public_key_fingerprint: publicFingerprint,
};
export const secondPublicParticipant = {
    seat: 0,
    username: 'Marian',
    public_key: secondPublicKey,
    public_key_fingerprint: secondPublicFingerprint,
};
export const rankable = { status: 'rankable' } as const;
export const canonicalJson = (value: unknown): string => {
    if (value === null || typeof value !== 'object') return JSON.stringify(value);
    if (Array.isArray(value)) return `[${value.map(canonicalJson).join(',')}]`;
    const object = value as Record<string, unknown>;
    return `{${Object.keys(object).sort().map(key => `${JSON.stringify(key)}:${canonicalJson(object[key])}`).join(',')}}`;
};
export const canonicalDigest = (value: unknown): string =>
    createHash('sha256').update(canonicalJson(value)).digest('hex');

export function viewer(buildManifestSha256 = sha('e')): Record<string, unknown> {
    return {
        build_manifest_sha256: buildManifestSha256,
        availability: {
            status: 'available',
            content_requirement: { kind: 'bundled_demo', content_manifest_sha256: sha('a') },
        },
    };
}

export function build(manifestSha256 = sha('e')): Record<string, unknown> {
    return {
        manifest_sha256: manifestSha256,
        source_commit: 'c'.repeat(40),
        display_name: 'Verifier 1',
    };
}

export type VerificationOptions = {
    readonly result?: string;
    readonly request?: string;
    readonly replay?: string;
    readonly initial?: string;
    readonly final?: string;
    readonly score?: number;
    readonly ticks?: number;
    readonly ransom?: number;
    readonly scope?: 'individual_level' | 'campaign';
    readonly sessionKind?: Record<string, unknown>;
    readonly ordinal?: number;
    readonly completeEvidence?: string | null;
    readonly startingScore?: number;
    readonly finalScore?: number;
};

export function verificationProof(options: VerificationOptions = {}): Record<string, unknown> {
    const replay = options.replay ?? sha('f');
    const initial = options.initial ?? sha('8');
    const final = options.final ?? sha('9');
    const score = options.score ?? 100;
    const ticks = options.ticks ?? 25;
    const ransom = options.ransom ?? 3;
    const scope = options.scope ?? 'individual_level';
    const campaign = scope === 'campaign';
    const completeEvidence = options.completeEvidence ?? null;
    const startingScore = options.startingScore ?? 0;
    const finalScore = options.finalScore ?? startingScore + score;
    const sessionKind = campaign
        ? options.sessionKind ?? { kind: 'field_mission', mission_id: 'm01' }
        : null;
    const contentSubject = sessionKind?.kind === 'headquarters'
        ? { kind: 'headquarters', mission_id: 'S00_HQ_MP' }
        : { kind: 'field_mission', mission_id: 'm01' };
    const publicRequest = {
        schema_version: 1,
        replay: replayArtifact(replay),
        content_edition: 'demo',
        content_subject: contentSubject,
        simulation_seed: '42',
        scope_kind: scope,
        campaign_aggregation_consent: campaign
            ? 'authorize_signed_session_in_server_recognized_chain_v1'
            : 'not_authorized',
        build_manifest_sha256: sha('e'),
        content_manifest_sha256: sha('a'),
        campaign_content_manifest_sha256: campaign ? sha('d') : null,
        rules_config_sha256: sha('b'),
        ruleset_manifest_sha256: sha('c'),
        competition_manifest_sha256: null,
        requested_metrics: ['original_score'],
        limits: {
            max_input_bytes: 1024,
            max_compressed_bytes: 1024,
            max_decompressed_bytes: 4096,
            max_base64_payload_bytes: 2048,
            max_campaign_bytes: 4096,
            max_frames: 1000,
            max_version_bytes: 128,
            max_mission_id_bytes: 256,
            max_metadata_records: 64,
            max_entries_per_frame: 64,
        },
        max_concurrent_players: 1,
        participant_instance_count: 1,
        named_participant_instance_count: 1,
        anonymous_participant_instance_count: 0,
        named_participants: [{
            seat: 0,
            public_key: publicKey,
        }],
    };
    const publicRequestSha256 = canonicalDigest(publicRequest);
    const evidence = completeEvidence === null ? null : {
        schema_version: 1,
        campaign_content_manifest_sha256: sha('d'),
        content_manifest_sha256: sha('a'),
        rules_config_sha256: sha('b'),
        ruleset_manifest_sha256: sha('c'),
        public_verification_request_sha256: publicRequestSha256,
        terminal_subject: contentSubject,
        final_campaign: campaignArtifact(final),
        final_state_sha256: sha('7'),
        observed_progression_percent: 100,
    };
    return {
        schema_version: 1,
        public_request: publicRequest,
        public_request_sha256: publicRequestSha256,
        input_provenance: rankable,
        campaign_session_kind: sessionKind,
        campaign_session_ordinal: campaign ? options.ordinal ?? 0 : null,
        max_concurrent_players: 1,
        participant_instance_count: 1,
        named_participant_instance_count: 1,
        anonymous_participant_instance_count: 0,
        outcome: 'won',
        starting_campaign: campaignArtifact(initial),
        final_campaign: campaignArtifact(final),
        starting_campaign_score: startingScore,
        final_campaign_score: finalScore,
        final_state_sha256: sha('7'),
        replay_frame_count: 100,
        metrics: {
            original_score_delta: score,
            active_simulation_ticks: ticks,
            ransom_collected: ransom,
        },
        campaign_complete_evidence: evidence,
        achievements: [{ achievement_id: 'clean_hands', evaluation: 'earned' }],
    };
}

export function missionRun(category: 'individual_level' | 'campaign' = 'individual_level'): Record<string, unknown> {
    const proof = verificationProof({ scope: category });
    return {
        schema_version: 1,
        run_id: 'run-1',
        rank: 3,
        subject: { kind: 'mission', mission_id: 'm01', category },
        mission: { mission_id: 'm01', display_name: 'Leicester', content_manifest_sha256: sha('a') },
        composition: { kind: 'mission', replay_sha256: sha('f') },
        outcome: 'won',
        metrics: { original_score_delta: 100, active_simulation_ticks: 25, ransom_collected: 3 },
        metric_value: { metric: 'original_score', points: 100 },
        max_concurrent_players: 1,
        participant_instance_count: 1,
        named_participant_instance_count: 1,
        named_participants: [publicParticipant],
        aggregate_named_participants: [],
        anonymous_participant_instance_count: 0,
        verified_at_unix_ms: 1_700_000_000_000,
        public_request_sha256: proof.public_request_sha256,
        public_result_sha256: canonicalDigest(proof),
        verification_proof: proof,
        campaign_aggregate: null,
        replay: replayArtifact(sha('f')),
        content: { kind: 'mission', content_manifest_sha256: sha('a') },
        campaign_content_manifest_sha256: category === 'campaign' ? sha('d') : null,
        rules_config_sha256: sha('b'),
        ruleset_manifest_sha256: sha('c'),
        competition_manifest_sha256: null,
        starting_campaign: campaignArtifact(sha('8')),
        final_campaign: campaignArtifact(sha('9')),
        starting_campaign_score: 0,
        final_campaign_score: 100,
        input_provenance: rankable,
        build: build(),
        achievements: [{
            display_name: 'Clean Hands',
            verified: { achievement_id: 'clean_hands', evaluation: 'earned' },
        }],
        viewer: viewer(),
        full_campaign_sessions: [],
        trust_statement: 'Server replay verification passed.',
    };
}

export function setMissionContentEdition(
    run: Record<string, unknown>,
    edition: 'demo' | 'full',
): void {
    const proof = run.verification_proof as Record<string, unknown>;
    const request = proof.public_request as Record<string, unknown>;
    request.content_edition = edition;
    proof.public_request_sha256 = canonicalDigest(request);
    run.public_request_sha256 = proof.public_request_sha256;
    run.public_result_sha256 = canonicalDigest(proof);
}

export function fullSession(
    ordinal: number,
    runId: string,
    session: Record<string, unknown>,
    displayName: string,
    mission: Record<string, unknown> | null,
    replay: string,
    _request: string,
    _resultDigest: string,
    initial: string,
    final: string,
    startingScore: number,
    finalScore: number,
    ticks: number,
    ransom: number,
    completeEvidence: string | null,
): Record<string, unknown> {
    const proof = verificationProof({
        replay,
        initial,
        final,
        score: finalScore - startingScore,
        ticks,
        ransom,
        scope: 'campaign',
        sessionKind: session,
        ordinal,
        completeEvidence,
        startingScore,
        finalScore,
    });
    const publicEvidence = proof.campaign_complete_evidence;
    return {
        ordinal,
        run_id: runId,
        session,
        content_subject: session.kind === 'field_mission'
            ? { kind: 'field_mission', mission_id: session.mission_id }
            : { kind: 'headquarters', mission_id: 'S00_HQ_MP' },
        display_name: displayName,
        mission,
        replay: replayArtifact(replay),
        public_verification_request_sha256: proof.public_request_sha256,
        public_verification_result_sha256: canonicalDigest(proof),
        verification_proof: proof,
        starting_campaign: campaignArtifact(initial),
        final_campaign: campaignArtifact(final),
        starting_campaign_score: startingScore,
        final_campaign_score: finalScore,
        public_campaign_complete_evidence_sha256: publicEvidence === null
            ? null
            : canonicalDigest(publicEvidence),
        content_manifest_sha256: sha('a'),
        rules_config_sha256: sha('b'),
        ruleset_manifest_sha256: sha('c'),
        competition_manifest_sha256: null,
        max_concurrent_players: 1,
        participant_instance_count: 1,
        named_participant_instance_count: 1,
        anonymous_participant_instance_count: 0,
        named_participants: [publicParticipant],
        input_provenance: rankable,
        metrics: {
            original_score_delta: finalScore - startingScore,
            active_simulation_ticks: ticks,
            ransom_collected: ransom,
        },
        achievements: [{
            display_name: 'Clean Hands',
            verified: { achievement_id: 'clean_hands', evaluation: 'earned' },
        }],
        build: build(),
        viewer: viewer(),
    };
}

export function fullCampaignRun(): Record<string, unknown> {
    const field = fullSession(
        0,
        'session-field',
        { kind: 'field_mission', mission_id: 'm01' },
        'Leicester',
        { mission_id: 'm01', display_name: 'Leicester', content_manifest_sha256: sha('a') },
        sha('2'), sha('3'), sha('4'), sha('8'), sha('9'), 0, 60, 10, 1,
        null,
    );
    const headquarters = fullSession(
        1,
        'session-hq',
        { kind: 'headquarters', hq_sequence: 1 },
        'Sherwood Camp',
        null,
        sha('5'), sha('6'), sha('7'), sha('9'), sha('a'), 60, 100, 15, 2,
        sha('2'),
    );
    const aggregateSessions = [
        {
            ordinal: 0,
            run_id: 'session-field',
            public_verification_request_sha256: field.public_verification_request_sha256,
            public_verification_result_sha256: field.public_verification_result_sha256,
        },
        {
            ordinal: 1,
            run_id: 'session-hq',
            public_verification_request_sha256: headquarters.public_verification_request_sha256,
            public_verification_result_sha256: headquarters.public_verification_result_sha256,
        },
    ];
    const aggregateRequest = {
        schema_version: 1,
        full_campaign_run_id: 'full-1',
        campaign_complete_terminal_run_id: 'session-hq',
        public_campaign_complete_evidence_sha256:
            headquarters.public_campaign_complete_evidence_sha256,
        sessions: aggregateSessions,
        campaign_content_manifest_sha256: sha('d'),
        rules_config_sha256: sha('b'),
        ruleset_manifest_sha256: sha('c'),
        competition_manifest_sha256: null,
    };
    const aggregateProof = {
        schema_version: 1,
        public_request: aggregateRequest,
        public_request_sha256: canonicalDigest(aggregateRequest),
        max_concurrent_players: 1,
        participant_instance_count: 2,
        named_participant_instance_count: 2,
        anonymous_participant_instance_count: 0,
        canonical_genesis_campaign: campaignArtifact(sha('8')),
        final_campaign: campaignArtifact(sha('a')),
        starting_campaign_score: 0,
        final_campaign_score: 100,
        metrics: { original_score_delta: 100, active_simulation_ticks: 25, ransom_collected: 3 },
    };
    return {
        schema_version: 1,
        run_id: 'full-1',
        rank: 1,
        subject: { kind: 'full_campaign' },
        mission: null,
        composition: {
            kind: 'full_campaign',
            ordered_session_run_ids: ['session-field', 'session-hq'],
        },
        outcome: 'won',
        metrics: { original_score_delta: 100, active_simulation_ticks: 25, ransom_collected: 3 },
        metric_value: {
            metric: 'fastest_success',
            active_simulation_ticks: 25,
            tick_duration: { numerator_micros: 40_000, denominator: 1 },
        },
        max_concurrent_players: 1,
        participant_instance_count: 2,
        named_participant_instance_count: 2,
        named_participants: [],
        aggregate_named_participants: [{
            current_display_name: 'Robin',
            public_key: publicKey,
            public_key_fingerprint: publicFingerprint,
        }],
        anonymous_participant_instance_count: 0,
        verified_at_unix_ms: 1_700_000_000_000,
        public_request_sha256: aggregateProof.public_request_sha256,
        public_result_sha256: canonicalDigest(aggregateProof),
        verification_proof: null,
        campaign_aggregate: aggregateProof,
        replay: null,
        content: { kind: 'full_campaign', campaign_content_manifest_sha256: sha('d') },
        campaign_content_manifest_sha256: null,
        rules_config_sha256: sha('b'),
        ruleset_manifest_sha256: sha('c'),
        competition_manifest_sha256: null,
        starting_campaign: campaignArtifact(sha('8')),
        final_campaign: campaignArtifact(sha('a')),
        starting_campaign_score: 0,
        final_campaign_score: 100,
        input_provenance: rankable,
        build: null,
        achievements: [],
        viewer: null,
        full_campaign_sessions: [field, headquarters],
        trust_statement: 'Every ordered session was replay-verified.',
    };
}

export function discloseSequentialSeatOccupants(runOrSession: Record<string, unknown>): void {
    runOrSession.participant_instance_count = 2;
    runOrSession.named_participant_instance_count = 2;
    runOrSession.named_participants = [publicParticipant, secondPublicParticipant];

    const proof = runOrSession.verification_proof as Record<string, unknown>;
    const request = proof.public_request as Record<string, unknown>;
    proof.participant_instance_count = 2;
    proof.named_participant_instance_count = 2;
    request.participant_instance_count = 2;
    request.named_participant_instance_count = 2;
    request.named_participants = [
        { seat: 0, public_key: publicKey },
        { seat: 0, public_key: secondPublicKey },
    ];
    proof.public_request_sha256 = canonicalDigest(request);

    if ('public_verification_request_sha256' in runOrSession) {
        runOrSession.public_verification_request_sha256 = proof.public_request_sha256;
        runOrSession.public_verification_result_sha256 = canonicalDigest(proof);
    } else {
        runOrSession.public_request_sha256 = proof.public_request_sha256;
        runOrSession.public_result_sha256 = canonicalDigest(proof);
    }
}

export function rulesConfigDocument() {
    return {
        schema_version: 1,
        replay_schema_version: runtimeContract.replaySchema,
        ranked_simulation_policy: {
            version: 1,
            preset: 'standard',
            difficulty: 'medium',
        },
        sim_config: {
            difficulty: 'Medium',
            count: 1,
            nested: { enabled: true, exact_values: [null, -2, 'normal'] },
        },
        rules: { state_load: false, mission_restart: false },
    };
}

export function rulesetDocument(rulesConfigDigest: string) {
    return {
        schema_version: 1,
        display_name: 'Standard · Normal',
        preset_id: 'standard',
        preset_name: 'Standard',
        difficulty_id: 'normal',
        difficulty_name: 'Normal',
        rules_config_sha256: rulesConfigDigest,
        rules_config_constraint: 'exact_canonical_digest_only',
        allowed_build_manifest_sha256: [sha('e')],
        allowed_content_manifest_sha256: [sha('a')],
        allowed_campaign_content_manifest_sha256: [sha('d')],
        board_scopes: ['individual_level', 'campaign_mission', 'full_campaign'],
        campaign_completion_policy: {
            mode: 'required',
            policy: {
                terminal_subject: { kind: 'field_mission', mission_id: 'H12_Not_MP' },
                required_progression_percent: 100,
            },
        },
        metrics: ['original_score', 'fastest_success'],
        metric_ranking: ['original_score_descending', 'fastest_success_ascending'],
        achievement_policies: [{ achievement_id: 'clean_hands', mode: 'required' }],
        canonical_start_policy: 'rules_config_bound_operator_state_and_verified_predecessor',
        canonical_campaign_state: {
            edition: 'full',
            kind: 'full_campaign_genesis',
            rules_config_sha256: rulesConfigDigest,
        },
        run_preflight_grant_public_key: sha('9'),
        full_campaign_chain_policy: 'canonical_genesis_every_field_and_headquarters_session_independent_completion',
        campaign_roster_continuity: 'union_of_verified_session_subsets',
        campaign_aggregation_consent_policy: 'every_authenticated_key_final_cosigns_each_session',
        participant_eligibility: {
            allow_single_player: true,
            allow_multiplayer: true,
            named_policy: 'host_genesis_guest_transport_join_attestation_and_final_cosign',
            anonymous_policy: 'allowed_authenticated_but_publicly_redacted',
            minimum_max_concurrent_players: 1,
            maximum_max_concurrent_players: 4,
            maximum_participant_instances: 16,
        },
        replay_schema_versions: [runtimeContract.replaySchema],
        network_protocol_versions: [runtimeContract.netProtocol],
        input_provenance_policy: { kind: 'input_provenance', version: 1, manifest_sha256: sha('1') },
        command_admission_policy: { kind: 'command_admission', version: 1, manifest_sha256: sha('2') },
        submission_admission_policy: { kind: 'submission_admission', version: 1, manifest_sha256: sha('3') },
        verifier_policy: { kind: 'verification', version: 1, manifest_sha256: sha('4') },
        input_provenance_eligibility: 'current_schema_canonical_replay_only',
        terminal_result_policy: 'independently_reached_won_only',
        score_algorithm: 'original_mission_attempt_wrapping_subtotal_campaign_delta_v1',
        score_overflow_policy: 'reject_campaign_or_aggregate_overflow',
        visible_tie_policy: 'equal_primary_metric_shares_rank',
        pagination_tie_break: 'accepted_sequence_then_verification_time_then_run_id_only',
        tick_duration: { numerator_micros: 40_000, denominator: 1 },
        active_time_definition: 'successful_simulation_ticks',
        frame_counting_policy: 'zero_based_events_before_exclusive_replay_frame_count',
        full_campaign_time_aggregation: 'checked_sum_every_verified_field_and_headquarters_session',
        run_composition_policy: 'mission_single_replay_full_campaign_ordered_sessions_no_synthetic_replay',
        main_board_seed_policy: 'open',
        competition_seed_policy: 'server_pinned',
        allow_save_creation: true,
        allow_autosave: true,
        allow_state_load: false,
        allow_mission_restart: false,
    };
}
