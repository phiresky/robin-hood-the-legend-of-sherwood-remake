import runtimeContract from '../../runtime-contract.json' with { type: 'json' };
import {
    sha,
    publicKey,
    publicFingerprint,
    secondPublicKey,
    secondPublicFingerprint,
    playerHistoryPage,
    campaignArtifact,
    replayArtifact,
    publicParticipant,
    canonicalDigest,
    missionRun,
    setMissionContentEdition,
    fullCampaignRun,
    discloseSequentialSeatOccupants,
    rulesConfigDocument,
    rulesetDocument,
} from './model-fixtures.js';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import {
    canonicalDocumentSha256,
    campaignContentCatalogDigest,
    parseAndVerifyBuildManifest,
    parseAndVerifyContentManifest,
    parseAndVerifyCampaignContentManifest,
    parseAndVerifyRulesConfigIdentity,
    parseAndVerifyPublishedRuleset,
    parseAndVerifyRulesetManifest,
    parseBoardMetadata,
    parseBoardPage,
    parseBuildManifest,
    parseCampaignSessionDetail,
    parseCampaignContentManifest,
    parseContentManifest,
    parseDeletionChallenge,
    parseDeletionRequestEnvelope,
    parsePlayerProfile,
    parsePlayerRunHistoryPage,
    parseReplayArtifact,
    parseRulesConfigIdentity,
    parseRulesetManifest,
    rankedSimulationPolicyDisplay,
    rankedSimulationPolicyLabels,
    parseRunDetail,
    parseUsernameChallenge,
    parseUsernameUpdateEnvelope,
    verifyRunDetailProofDigests,
    verifyCampaignSessionDetailProofDigest,
    validateRunCampaignContentBinding,
    validateRulesetFacetScopeBinding,
    validateRulesetRulesConfigBinding,
} from './model.js';

test('metadata and leaderboard pages preserve typed mission/full-campaign subjects', async () => {
    const competitionManifest = {
        schema_version: 1,
        competition_id: 'campaign-2026',
        competition_version: 1,
        display_name: 'Campaign Cup',
        description: 'Complete verified campaigns.',
        subject: { kind: 'full_campaign' },
        metric: 'fastest_success',
        rules_config_sha256: sha('b'),
        ruleset_manifest_sha256: sha('c'),
        content: { kind: 'full_campaign', campaign_content_manifest_sha256: sha('d') },
        seed_policy: { kind: 'pinned', simulation_seed: '18446744073709551615' },
        participant_composition: { kind: 'single_player' },
        competition_run_grant_public_key: sha('e'),
        starts_at_unix_ms: 1_700_000_000_000,
        ends_at_unix_ms: 1_700_086_400_000,
    };
    const metadata = parseBoardMetadata({
        schema_version: 1,
        missions: [{ mission_id: 'm01', display_name: 'Leicester', content_manifest_sha256: sha('a') }],
        rulesets: [{
            ruleset_manifest_sha256: sha('c'),
            rules_config_sha256: sha('b'),
            display_name: 'Fresh · Normal',
            preset_id: 'fresh',
            preset_name: 'Fresh Features',
            difficulty_id: 'normal',
            difficulty_name: 'Normal',
            content: { kind: 'full_campaign', campaign_content_manifest_sha256: sha('d') },
            categories: ['individual_level', 'campaign'],
            metrics: ['original_score', 'fastest_success'],
            supports_full_campaign_boards: true,
        }],
        competitions: [{
            competition_manifest_sha256: await canonicalDocumentSha256(competitionManifest),
            manifest: competitionManifest,
            state: 'active',
        }],
        full_campaign: { display_name: 'Full Campaign', description: 'Every field and HQ session.' },
    });
    assert.equal(metadata.rulesets[0]?.supportsFullCampaign, true);
    assert.deepEqual(metadata.competitions[0]?.subject, { kind: 'full_campaign' });
    assert.deepEqual(metadata.competitions[0]?.seedPolicy, {
        kind: 'pinned',
        simulationSeed: '18446744073709551615',
    });
    assert.equal(metadata.fullCampaign?.label, 'Full Campaign');

    for (const invalidSeed of ['00', '18446744073709551616', 42]) {
        const invalidManifest = structuredClone(competitionManifest) as Record<string, unknown>;
        invalidManifest.seed_policy = { kind: 'pinned', simulation_seed: invalidSeed };
        const invalidDigest = await canonicalDocumentSha256(invalidManifest);
        assert.throws(() => parseBoardMetadata({
            schema_version: 1,
            missions: [{ mission_id: 'm01', display_name: 'Leicester', content_manifest_sha256: sha('a') }],
            rulesets: [],
            competitions: [{
                competition_manifest_sha256: invalidDigest,
                manifest: invalidManifest,
                state: 'active',
            }],
            full_campaign: null,
        }), /canonical unsigned decimal string|unsigned 64-bit range/u);
    }

    const page = parseBoardPage({
        schema_version: 1,
        filter: {
            schema_version: 1,
            subject: { kind: 'full_campaign' },
            metric: 'original_score',
            content: { kind: 'full_campaign', campaign_content_manifest_sha256: sha('d') },
            rules_config_sha256: sha('b'),
            ruleset_manifest_sha256: sha('c'),
            competition_manifest_sha256: null,
            max_concurrent_players: 1,
        },
        accepted_sequence_watermark: 9,
        previous_cursor: null,
        entries: [{
            position: 1,
            rank: 1,
            run_id: 'full-1',
            composition: { kind: 'full_campaign', ordered_session_run_ids: ['run-1'] },
            metric_value: { metric: 'original_score', points: 100 },
            max_concurrent_players: 1,
            participant_instance_count: 1,
            named_participant_instance_count: 1,
            named_participants: [],
            aggregate_named_participants: [{
                current_display_name: 'Robin',
                public_key: publicKey,
                public_key_fingerprint: publicFingerprint,
            }],
            anonymous_participant_instance_count: 0,
            accepted_sequence: 9,
            verified_at_unix_ms: 1_700_000_000_000,
        }],
        next_cursor: null,
    });
    assert.equal(page.entries[0]?.composition.kind, 'full_campaign');
    assert.equal(page.filter.subject.kind, 'full_campaign');
});

test('public identity fingerprints are derived from the exact key and hostile names remain inert text', () => {
    const profile = parsePlayerProfile({
        schema_version: 1,
        username: '<img src=x onerror=alert(1)>',
        public_key: publicKey,
        public_key_fingerprint: publicFingerprint,
    });
    assert.equal(profile.username, '<img src=x onerror=alert(1)>');
    assert.equal(profile.publicKeyFingerprint, publicFingerprint);
    assert.equal(parsePlayerProfile({
        schema_version: 1,
        username: 'Protocol vector',
        public_key: 'ab'.repeat(32),
        public_key_fingerprint: '4386e85fa8fe41e53c9be90d18458bbd',
    }).publicKeyFingerprint, '4386e85fa8fe41e53c9be90d18458bbd');
    assert.throws(() => parsePlayerProfile({
        schema_version: 1,
        username: 'Robin',
        public_key: publicKey,
        public_key_fingerprint: '2'.repeat(32),
    }), /canonical public-key fingerprint/u);
    assert.throws(() => parsePlayerProfile({
        schema_version: 1,
        username: 'Robin\u202eexe',
        public_key: publicKey,
        public_key_fingerprint: publicFingerprint,
    }), /control characters/u);
});

test('leaderboard rows reject private participant-instance identifiers', () => {
    const page = {
        schema_version: 1,
        filter: {
            schema_version: 1,
            subject: { kind: 'mission', mission_id: 'm01', category: 'individual_level' },
            metric: 'original_score',
            content: { kind: 'mission', content_manifest_sha256: sha('a') },
            rules_config_sha256: sha('b'),
            ruleset_manifest_sha256: sha('c'),
            competition_manifest_sha256: null,
            max_concurrent_players: 1,
        },
        accepted_sequence_watermark: 1,
        previous_cursor: null,
        entries: [{
            position: 1,
            rank: 1,
            run_id: 'run-1',
            composition: { kind: 'mission', replay_sha256: sha('f') },
            metric_value: { metric: 'original_score', points: 100 },
            max_concurrent_players: 1,
            participant_instance_count: 1,
            named_participant_instance_count: 1,
            named_participants: [publicParticipant],
            aggregate_named_participants: [],
            anonymous_participant_instance_count: 0,
            accepted_sequence: 1,
            verified_at_unix_ms: 1_700_000_000_000,
        }],
        next_cursor: null,
    };
    const parsed = parseBoardPage(page);
    assert.deepEqual(parsed.entries[0]?.namedParticipants.map(participant => ({
        seat: participant.seat,
        publicKey: participant.publicKey,
    })), [{ seat: 0, publicKey }]);
    assert.doesNotMatch(JSON.stringify(parsed), /participantInstanceId|participant_instance_id/u);

    const leaked = structuredClone(page);
    const entry = (leaked.entries as Record<string, unknown>[])[0]!;
    const participant = (entry.named_participants as Record<string, unknown>[])[0]!;
    participant.participant_instance_id = 'private-participant-instance-sentinel';
    assert.throws(() => parseBoardPage(leaked), /unknown field participant_instance_id/u);
});

test('player history parses the public-only run, best, provenance, and snapshot contract', () => {
    const page = parsePlayerRunHistoryPage(playerHistoryPage());
    assert.equal(page.player.username, '<img src=x onerror=alert(1)>');
    assert.equal(page.player.publicKey, publicKey);
    assert.equal(page.acceptedSequenceWatermark, 9);
    assert.equal(page.runs[0]?.run.runId, 'run-9');
    assert.equal(page.runs[0]?.run.metrics.activeSimulationTicks, 240);
    assert.equal(page.personalBests[0]?.metricValue.metric, 'original_score');
    assert.equal(page.nextCursor, 'authenticated-cursor');

    const wideScore = structuredClone(playerHistoryPage());
    (wideScore.personal_bests as Array<Record<string, unknown>>)[0]!.metric_value = {
        metric: 'original_score',
        points: 5_000_000_000,
    };
    assert.equal(
        parsePlayerRunHistoryPage(wideScore).personalBests[0]?.metricValue.metric,
        'original_score',
    );
});

test('player history rejects private fields, substituted keys, duplicate identities, and broken bindings', () => {
    const privateLeak = structuredClone(playerHistoryPage());
    privateLeak.private_verification_request_sha256 = sha('e');
    assert.throws(
        () => parsePlayerRunHistoryPage(privateLeak),
        /unknown field private_verification_request_sha256/u,
    );

    const legacyRun = structuredClone(playerHistoryPage());
    const leakedRun = (legacyRun.runs as Array<Record<string, unknown>>)[0]?.run as Record<string, unknown>;
    leakedRun.public_replay_sha256 = sha('e');
    assert.throws(() => parsePlayerRunHistoryPage(legacyRun), /unknown field public_replay_sha256/u);

    const substituted = structuredClone(playerHistoryPage());
    (substituted.runs as Array<Record<string, unknown>>)[0]!.player_public_key = sha('2');
    assert.throws(() => parsePlayerRunHistoryPage(substituted), /different public key/u);

    const duplicateRuns = structuredClone(playerHistoryPage());
    (duplicateRuns.runs as unknown[]).push(structuredClone((duplicateRuns.runs as unknown[])[0]));
    assert.throws(() => parsePlayerRunHistoryPage(duplicateRuns), /duplicate run identities/u);

    const duplicateBests = structuredClone(playerHistoryPage());
    (duplicateBests.personal_bests as unknown[]).push(
        structuredClone((duplicateBests.personal_bests as unknown[])[0]),
    );
    assert.throws(() => parsePlayerRunHistoryPage(duplicateBests), /duplicate personal-best filters/u);

    const wrongMetric = structuredClone(playerHistoryPage());
    (wrongMetric.personal_bests as Array<Record<string, unknown>>)[0]!.metric_value = {
        metric: 'fastest_success',
        active_simulation_ticks: 240,
        tick_duration: { numerator_micros: 100_000, denominator: 1 },
    };
    assert.throws(() => parsePlayerRunHistoryPage(wrongMetric), /does not match its board filter/u);

    const wrongComposition = structuredClone(playerHistoryPage());
    const run = (wrongComposition.runs as Array<Record<string, unknown>>)[0]!.run as Record<string, unknown>;
    run.composition = {
        kind: 'full_campaign',
        ordered_session_run_ids: ['run-1'],
    };
    assert.throws(() => parsePlayerRunHistoryPage(wrongComposition), /composition does not match/u);
});

test('player history rejects impossible and hostile pagination documents', () => {
    const noWatermark = structuredClone(playerHistoryPage());
    noWatermark.accepted_sequence_watermark = 0;
    assert.throws(() => parsePlayerRunHistoryPage(noWatermark), /positive snapshot watermark/u);

    const emptyWithCursor = structuredClone(playerHistoryPage());
    emptyWithCursor.runs = [];
    emptyWithCursor.next_cursor = 'cursor-after-empty';
    assert.throws(() => parsePlayerRunHistoryPage(emptyWithCursor), /empty player_run_history/u);

    const controlCursor = structuredClone(playerHistoryPage());
    controlCursor.next_cursor = 'cursor\nsmuggle';
    assert.throws(() => parsePlayerRunHistoryPage(controlCursor), /forbidden control characters/u);
});

test('leaderboard cursors bind the canonical filter, immutable snapshot, and stable tie order', async () => {
    const filter = {
        schema_version: 1,
        subject: { kind: 'full_campaign' },
        metric: 'original_score',
        content: { kind: 'full_campaign', campaign_content_manifest_sha256: sha('d') },
        rules_config_sha256: sha('b'),
        ruleset_manifest_sha256: sha('c'),
        competition_manifest_sha256: null,
        max_concurrent_players: 1,
    };
    const aggregateParticipant = {
        current_display_name: 'Robin',
        public_key: publicKey,
        public_key_fingerprint: publicFingerprint,
    };
    const entry = (position: number, acceptedSequence: number, runId: string) => ({
        position,
        rank: 1,
        run_id: runId,
        composition: { kind: 'full_campaign', ordered_session_run_ids: [`s-${position}`] },
        metric_value: { metric: 'original_score', points: 100 },
        max_concurrent_players: 1,
        participant_instance_count: 1,
        named_participant_instance_count: 1,
        named_participants: [],
        aggregate_named_participants: [aggregateParticipant],
        anonymous_participant_instance_count: 0,
        accepted_sequence: acceptedSequence,
        verified_at_unix_ms: 1_700_000_000_000 + position,
    });
    const entries = [entry(1, 8, 'full-1'), entry(2, 9, 'full-2')];
    const querySha256 = await canonicalDocumentSha256(filter);
    const page = {
        schema_version: 1,
        filter,
        accepted_sequence_watermark: 10,
        previous_cursor: null,
        entries,
        next_cursor: {
            schema_version: 1,
            query_sha256: querySha256,
            accepted_sequence_watermark: 10,
            last: {
                position: 2,
                rank: 1,
                metric_value: entries[1]!.metric_value,
                accepted_sequence: 9,
                verified_at_unix_ms: 1_700_000_000_002,
                run_id: 'full-2',
            },
            opaque_token: 'authenticated-cursor',
        },
    };
    assert.equal(parseBoardPage(page).nextCursor, 'authenticated-cursor');
    const reordered = structuredClone(page);
    (reordered.entries as Record<string, unknown>[])[1]!.accepted_sequence = 7;
    assert.throws(() => parseBoardPage(reordered), /stable order/u);
    const substituted = structuredClone(page);
    const cursor = substituted.next_cursor as Record<string, unknown>;
    cursor.query_sha256 = sha('f');
    assert.throws(() => parseBoardPage(substituted), /query|final page row/u);

    // These protocol counters are currently JSON numbers. They therefore fail
    // closed before any digest/order comparison once they exceed JavaScript's
    // exact integer range; only decimal-string protocol fields may span u64.
    for (const field of ['accepted_sequence_watermark', 'accepted_sequence'] as const) {
        const unsafe = structuredClone(page);
        if (field === 'accepted_sequence_watermark') {
            unsafe.accepted_sequence_watermark = Number.MAX_SAFE_INTEGER + 1;
        } else {
            (unsafe.entries as Record<string, unknown>[])[0]![field] = Number.MAX_SAFE_INTEGER + 1;
        }
        assert.throws(() => parseBoardPage(unsafe), /exact safe integer|safe nonnegative integer/u);
    }
});

test('mission detail cross-checks every public field against its redacted verification proof', () => {
    const parsed = parseRunDetail(missionRun());
    assert.equal(parsed.subject.kind, 'mission');
    assert.equal(parsed.replay?.artifact.sha256, sha('f'));
    assert.equal(parsed.playbackProof?.replay.artifact.byteLength, 123);
    assert.deepEqual(parsed.playbackProof?.startingCampaign, parsed.startingCampaign);
    assert.deepEqual(parsed.playbackProof?.finalCampaign, parsed.finalCampaign);
    assert.equal(parsed.build?.manifestSha256, sha('e'));
    assert.equal(parsed.viewer?.availability.status, 'available');

    const substituted = structuredClone(missionRun());
    const proof = substituted.verification_proof as Record<string, unknown>;
    const publicRequest = proof.public_request as Record<string, unknown>;
    publicRequest.build_manifest_sha256 = sha('4');
    assert.throws(() => parseRunDetail(substituted), /canonical public request/u);

    const unknown = structuredClone(missionRun());
    const unknownViewer = unknown.viewer as Record<string, unknown>;
    const availability = unknownViewer.availability as Record<string, unknown>;
    availability.redirect_url = 'https://evil.invalid';
    assert.throws(() => parseRunDetail(unknown), /unknown field redirect_url/u);

    const privateLeak = structuredClone(missionRun());
    const privateProof = privateLeak.verification_proof as Record<string, unknown>;
    privateProof.replay_session_transcript = { sentinel: 'must-remain-private' };
    assert.throws(() => parseRunDetail(privateLeak), /unknown field replay_session_transcript/u);

    const consentSubstitution = structuredClone(missionRun());
    const consentProof = consentSubstitution.verification_proof as Record<string, unknown>;
    const consentRequest = consentProof.public_request as Record<string, unknown>;
    consentRequest.campaign_aggregation_consent = 'authorize_signed_session_in_server_recognized_chain_v1';
    assert.throws(() => parseRunDetail(consentSubstitution), /public scope tuple|canonical public request/u);

    const multiplayer = structuredClone(missionRun());
    const multiplayerProof = multiplayer.verification_proof as Record<string, unknown>;
    const multiplayerRequest = multiplayerProof.public_request as Record<string, unknown>;
    multiplayer.max_concurrent_players = 2;
    multiplayer.participant_instance_count = 2;
    multiplayer.named_participant_instance_count = 2;
    multiplayer.named_participants = [publicParticipant, {
        seat: 1,
        username: 'Marian',
        public_key: '2'.repeat(64),
        public_key_fingerprint: '94ca02d832b9e0d11b766cf04148bccf',
    }];
    multiplayerProof.max_concurrent_players = 2;
    multiplayerProof.participant_instance_count = 2;
    multiplayerProof.named_participant_instance_count = 2;
    multiplayerRequest.max_concurrent_players = 2;
    multiplayerRequest.participant_instance_count = 2;
    multiplayerRequest.named_participant_instance_count = 2;
    multiplayerRequest.named_participants = [
        { seat: 0, public_key: publicKey },
        { seat: 1, public_key: '2'.repeat(64) },
    ];
    multiplayerProof.public_request_sha256 = canonicalDigest(multiplayerRequest);
    multiplayer.public_request_sha256 = multiplayerProof.public_request_sha256;
    multiplayer.public_result_sha256 = canonicalDigest(multiplayerProof);
    assert.equal(parseRunDetail(multiplayer).namedParticipants.length, 2);
});

test('viewer requirement is exactly bound to Demo or Full content edition', () => {
    const invertedDemo = missionRun();
    const invertedDemoViewer = invertedDemo.viewer as Record<string, unknown>;
    const invertedDemoAvailability = invertedDemoViewer.availability as Record<string, unknown>;
    const invertedDemoRequirement = invertedDemoAvailability.content_requirement as Record<string, unknown>;
    invertedDemoRequirement.kind = 'user_local_retail';
    assert.throws(() => parseRunDetail(invertedDemo), /verified demo edition/u);

    const exactFull = missionRun('campaign');
    setMissionContentEdition(exactFull, 'full');
    const exactFullViewer = exactFull.viewer as Record<string, unknown>;
    const exactFullAvailability = exactFullViewer.availability as Record<string, unknown>;
    const exactFullRequirement = exactFullAvailability.content_requirement as Record<string, unknown>;
    exactFullRequirement.kind = 'user_local_retail';
    const parsedFull = parseRunDetail(exactFull);
    assert.equal(
        parsedFull.viewer?.availability.status === 'available'
            ? parsedFull.viewer.availability.contentRequirement.kind
            : null,
        'user_local_retail',
    );

    const invertedFull = structuredClone(exactFull);
    const invertedFullViewer = invertedFull.viewer as Record<string, unknown>;
    const invertedFullAvailability = invertedFullViewer.availability as Record<string, unknown>;
    const invertedFullRequirement = invertedFullAvailability.content_requirement as Record<string, unknown>;
    invertedFullRequirement.kind = 'bundled_demo';
    assert.throws(() => parseRunDetail(invertedFull), /verified full edition/u);

    const invertedCampaignSession = fullCampaignRun();
    const firstSession = (invertedCampaignSession.full_campaign_sessions as Record<string, unknown>[])[0]!;
    const sessionViewer = firstSession.viewer as Record<string, unknown>;
    const sessionAvailability = sessionViewer.availability as Record<string, unknown>;
    const sessionRequirement = sessionAvailability.content_requirement as Record<string, unknown>;
    sessionRequirement.kind = 'user_local_retail';
    assert.throws(() => parseRunDetail(invertedCampaignSession), /verified demo edition/u);
});

test('public mission identity is the canonical seat and public-key pair', () => {
    const sequential = missionRun();
    discloseSequentialSeatOccupants(sequential);
    const parsed = parseRunDetail(sequential);
    assert.deepEqual(parsed.namedParticipants.map(participant => ({
        seat: participant.seat,
        publicKey: participant.publicKey,
    })), [
        { seat: 0, publicKey },
        { seat: 0, publicKey: secondPublicKey },
    ]);
    assert.equal(new Set(parsed.namedParticipants.map(participant => participant.publicKey)).size, 2);
    assert.doesNotMatch(JSON.stringify(parsed), /participantInstanceId|participant_instance_id/u);

    const reversed = structuredClone(sequential);
    (reversed.named_participants as unknown[]).reverse();
    assert.throws(() => parseRunDetail(reversed), /invalid mission participant claims/u);

    const leakedRoster = structuredClone(sequential);
    const leakedParticipant = (leakedRoster.named_participants as Record<string, unknown>[])[0]!;
    leakedParticipant.participant_instance_id = 'private-participant-instance-sentinel';
    assert.throws(() => parseRunDetail(leakedRoster), /unknown field participant_instance_id/u);

    const leakedProof = structuredClone(sequential);
    const proof = leakedProof.verification_proof as Record<string, unknown>;
    const request = proof.public_request as Record<string, unknown>;
    const claim = (request.named_participants as Record<string, unknown>[])[0]!;
    claim.participant_instance_id = 'private-participant-instance-sentinel';
    assert.throws(() => parseRunDetail(leakedProof), /unknown field participant_instance_id/u);
});

test('achievement decisions preserve earned, not-earned, and unverifiable states', () => {
    const expectations = ['earned', 'not_earned', 'unverifiable'] as const;
    for (const evaluation of expectations) {
        const wire = structuredClone(missionRun());
        const summaries = wire.achievements as Record<string, unknown>[];
        const outerVerified = summaries[0]!.verified as Record<string, unknown>;
        outerVerified.evaluation = evaluation;
        const proof = wire.verification_proof as Record<string, unknown>;
        const decisions = proof.achievements as Record<string, unknown>[];
        decisions[0]!.evaluation = evaluation;
        wire.public_result_sha256 = canonicalDigest(proof);
        assert.equal(parseRunDetail(wire).achievements[0]!.evaluation, evaluation);
    }
});

test('campaign mission exposes only fully cross-bound public artifacts', () => {
    const parsed = parseRunDetail(missionRun('campaign'));
    assert.equal(parsed.composition.kind, 'mission');
    assert.deepEqual(parsed.replay, parsed.playbackProof?.replay);
    assert.deepEqual(parsed.startingCampaign, parsed.playbackProof?.startingCampaign);
    assert.deepEqual(parsed.finalCampaign, parsed.playbackProof?.finalCampaign);

    for (const [field, replacement] of [
        ['replay', replayArtifact(sha('4'))],
        ['starting_campaign', campaignArtifact(sha('4'))],
        ['final_campaign', campaignArtifact(sha('4'))],
    ] as const) {
        const wire = structuredClone(missionRun('campaign'));
        wire[field] = replacement;
        assert.throws(() => parseRunDetail(wire), /bind|substituted|does not match/u, field);
    }
});

test('full campaign aggregate exposes every ordered labeled field and HQ replay', () => {
    const parsed = parseRunDetail(fullCampaignRun());
    assert.deepEqual(parsed.subject, { kind: 'full_campaign' });
    assert.equal(parsed.replay, null);
    assert.equal(parsed.build, null);
    assert.deepEqual(parsed.fullCampaignSessions.map(session => ({
        runId: session.runId,
        kind: session.kind,
        label: session.label,
    })), [
        { runId: 'session-field', kind: 'field_mission', label: 'Leicester' },
        { runId: 'session-hq', kind: 'headquarters', label: 'Sherwood Camp' },
    ]);
    assert.equal(parsed.fullCampaignSessions[0]?.replay.artifact.sha256, sha('2'));
    assert.equal(parsed.fullCampaignSessions[1]?.viewer.availability.status, 'available');
    assert.deepEqual(parsed.composition, {
        kind: 'full_campaign',
        orderedSessionRunIds: ['session-field', 'session-hq'],
    });
    assert.doesNotMatch(JSON.stringify(parsed), /chainId|chain_id|private-campaign-chain-sentinel/u);
});

test('full campaign roster preserves different keys which occupied the same seat sequentially', () => {
    const wire = fullCampaignRun();
    const sessions = wire.full_campaign_sessions as Record<string, unknown>[];
    const firstSession = sessions[0]!;
    discloseSequentialSeatOccupants(firstSession);

    wire.participant_instance_count = 3;
    wire.named_participant_instance_count = 3;
    wire.aggregate_named_participants = [{
        current_display_name: 'Robin',
        public_key: publicKey,
        public_key_fingerprint: publicFingerprint,
    }, {
        current_display_name: 'Marian',
        public_key: secondPublicKey,
        public_key_fingerprint: secondPublicFingerprint,
    }];

    const aggregate = wire.campaign_aggregate as Record<string, unknown>;
    const aggregateRequest = aggregate.public_request as Record<string, unknown>;
    const aggregateSessions = aggregateRequest.sessions as Record<string, unknown>[];
    aggregateSessions[0]!.public_verification_request_sha256 = firstSession.public_verification_request_sha256;
    aggregateSessions[0]!.public_verification_result_sha256 = firstSession.public_verification_result_sha256;
    aggregate.participant_instance_count = 3;
    aggregate.named_participant_instance_count = 3;
    aggregate.public_request_sha256 = canonicalDigest(aggregateRequest);
    wire.public_request_sha256 = aggregate.public_request_sha256;
    wire.public_result_sha256 = canonicalDigest(aggregate);

    const parsed = parseRunDetail(wire);
    assert.deepEqual(parsed.aggregateNamedParticipants.map(participant => participant.publicKey), [
        publicKey,
        secondPublicKey,
    ]);
    assert.deepEqual(parsed.fullCampaignSessions[0]?.namedParticipants.map(participant => [
        participant.seat,
        participant.publicKey,
    ]), [
        [0, publicKey],
        [0, secondPublicKey],
    ]);
    assert.doesNotMatch(JSON.stringify(parsed), /participantInstanceId|participant_instance_id/u);
});

test('full campaign parser rejects private chain IDs and nested proof substitutions', () => {
    const legacyComposition = structuredClone(fullCampaignRun());
    const composition = legacyComposition.composition as Record<string, unknown>;
    composition.chain_id = 'private-campaign-chain-sentinel';
    assert.throws(() => parseRunDetail(legacyComposition), /unknown field chain_id/u);

    const wrongAggregate = structuredClone(fullCampaignRun());
    const aggregate = wrongAggregate.campaign_aggregate as Record<string, unknown>;
    const aggregateRequest = aggregate.public_request as Record<string, unknown>;
    aggregateRequest.chain_id = 'private-campaign-chain-sentinel';
    assert.throws(() => parseRunDetail(wrongAggregate), /unknown field chain_id/u);

    const wrongSessionProof = structuredClone(fullCampaignRun());
    const sessions = wrongSessionProof.full_campaign_sessions as Record<string, unknown>[];
    const sessionProof = sessions[0]?.verification_proof as Record<string, unknown>;
    sessionProof.final_campaign = campaignArtifact(sha('4'));
    assert.throws(() => parseRunDetail(wrongSessionProof), /substituted|does not match|outer proof identities/u);

    const reordered = structuredClone(fullCampaignRun());
    const reorderedComposition = reordered.composition as Record<string, unknown>;
    reorderedComposition.ordered_session_run_ids = ['session-hq', 'session-field'];
    assert.throws(() => parseRunDetail(reordered), /session order/u);

    const wrongConsent = structuredClone(fullCampaignRun());
    const wrongConsentSessions = wrongConsent.full_campaign_sessions as Record<string, unknown>[];
    const wrongConsentProof = wrongConsentSessions[0]!.verification_proof as Record<string, unknown>;
    const wrongConsentRequest = wrongConsentProof.public_request as Record<string, unknown>;
    wrongConsentRequest.campaign_aggregation_consent = 'not_authorized';
    assert.throws(() => parseRunDetail(wrongConsent), /public scope tuple|canonical public request/u);

    const wrongCounts = structuredClone(fullCampaignRun());
    const wrongCountAggregate = wrongCounts.campaign_aggregate as Record<string, unknown>;
    wrongCountAggregate.named_participant_instance_count = 1;
    assert.throws(() => parseRunDetail(wrongCounts), /aggregate participant counts|aggregate metrics/u);

    const syntheticAchievement = structuredClone(fullCampaignRun());
    syntheticAchievement.achievements = [{
        display_name: 'Not aggregate-authenticated',
        verified: { achievement_id: 'clean-hands', evaluation: 'earned' },
    }];
    assert.throws(() => parseRunDetail(syntheticAchievement), /synthetic achievements/u);
});

test('campaign session detail authenticates its redacted verifier proof and route ordinal', async () => {
    const aggregate = fullCampaignRun();
    const session = structuredClone((aggregate.full_campaign_sessions as Record<string, unknown>[])[0]!);
    const detail = {
        schema_version: 1,
        aggregate_run_id: 'full-1',
        public_aggregate_result_sha256: (aggregate.public_result_sha256 as string),
        ordinal: 0,
        session,
    };
    assert.equal(parseCampaignSessionDetail(detail).session.ordinal, 0);
    await verifyCampaignSessionDetailProofDigest(detail);
    const substituted = structuredClone(detail);
    const substitutedSession = substituted.session as Record<string, unknown>;
    const proof = substitutedSession.verification_proof as Record<string, unknown>;
    const request = proof.public_request as Record<string, unknown>;
    request.build_manifest_sha256 = sha('4');
    await assert.rejects(verifyCampaignSessionDetailProofDigest(substituted), /canonical public request/u);

    const invertedEntitlement = structuredClone(detail);
    const invertedSession = invertedEntitlement.session as Record<string, unknown>;
    const invertedViewer = invertedSession.viewer as Record<string, unknown>;
    const invertedAvailability = invertedViewer.availability as Record<string, unknown>;
    const invertedRequirement = invertedAvailability.content_requirement as Record<string, unknown>;
    invertedRequirement.kind = 'user_local_retail';
    assert.throws(() => parseCampaignSessionDetail(invertedEntitlement), /verified demo edition/u);
    assert.throws(() => parseCampaignSessionDetail({ ...detail, ordinal: 1 }), /ordinal/u);
});

test('campaign session projection keeps sequential occupants distinct without exposing private IDs', () => {
    const aggregate = fullCampaignRun();
    const session = structuredClone((aggregate.full_campaign_sessions as Record<string, unknown>[])[0]!);
    discloseSequentialSeatOccupants(session);
    const detail = {
        schema_version: 1,
        aggregate_run_id: 'full-1',
        public_aggregate_result_sha256: aggregate.public_result_sha256,
        ordinal: 0,
        session,
    };
    const parsed = parseCampaignSessionDetail(detail);
    assert.deepEqual(parsed.session.namedParticipants.map(participant => [participant.seat, participant.publicKey]), [
        [0, publicKey],
        [0, secondPublicKey],
    ]);
    assert.doesNotMatch(JSON.stringify(parsed), /participantInstanceId|participant_instance_id/u);

    const leakedSession = structuredClone(detail);
    const leakedRoster = (leakedSession.session as Record<string, unknown>)
        .named_participants as Record<string, unknown>[];
    leakedRoster[0]!.participant_instance_id = 'private-participant-instance-sentinel';
    assert.throws(() => parseCampaignSessionDetail(leakedSession), /unknown field participant_instance_id/u);

    const leakedProof = structuredClone(detail);
    const sessionProof = (leakedProof.session as Record<string, unknown>)
        .verification_proof as Record<string, unknown>;
    const request = sessionProof.public_request as Record<string, unknown>;
    const claim = (request.named_participants as Record<string, unknown>[])[0]!;
    claim.participant_instance_id = 'private-participant-instance-sentinel';
    assert.throws(() => parseCampaignSessionDetail(leakedProof), /unknown field participant_instance_id/u);
});

test('public run fixtures recursively exclude private verifier and identity evidence', async () => {
    const forbidden = new Set([
        'verification_result', 'authenticated_participant_claims', 'participant_claims',
        'participant_instance_id', 'chain_id',
        'join_attestation', 'signature', 'host_endpoint_id', 'transport', 'transport_identity',
        'replay_session_transcript', 'diagnostics', 'authenticated_participant_keys',
        'campaign_controller_public_key', 'campaign_chain_receipt', 'evidence',
        'public_replay', 'private_initial_campaign', 'private_starting_campaign',
        'private_final_campaign', 'private_final_state_sha256', 'request_id',
        'verification_request_sha256', 'verification_result_sha256',
        'aggregate_request_sha256', 'aggregate_result_sha256',
    ]);
    const assertNoPrivateKeys = (value: unknown, path = '$'): void => {
        if (Array.isArray(value)) {
            value.forEach((item, index) => assertNoPrivateKeys(item, `${path}[${index}]`));
            return;
        }
        if (value === null || typeof value !== 'object') return;
        for (const [key, nested] of Object.entries(value as Record<string, unknown>)) {
            assert.ok(!forbidden.has(key), `private key ${path}.${key} leaked into a public fixture`);
            assertNoPrivateKeys(nested, `${path}.${key}`);
        }
    };
    const mission = missionRun();
    const aggregate = fullCampaignRun();
    assertNoPrivateKeys(mission);
    assertNoPrivateKeys(aggregate);
    await verifyRunDetailProofDigests(mission);
    await verifyRunDetailProofDigests(aggregate);

    for (const forbiddenKey of forbidden) {
        const substituted = structuredClone(mission);
        const proof = substituted.verification_proof as Record<string, unknown>;
        proof[forbiddenKey] = `private-sentinel-${forbiddenKey}`;
        assert.throws(() => parseRunDetail(substituted), new RegExp(`unknown field ${forbiddenKey}`, 'u'));
    }

    for (const forbiddenKey of ['campaign_chain_receipt', 'public_replay', 'private_final_campaign']) {
        const substituted = structuredClone(mission);
        substituted[forbiddenKey] = { private_sentinel: true };
        assert.throws(() => parseRunDetail(substituted), new RegExp(`unknown field ${forbiddenKey}`, 'u'));
    }
});

test('canonical object keys use Rust UTF-8 byte order rather than JavaScript UTF-16 order', async () => {
    const value = { '\u{10000}': 2, '\uE000': 1 };
    assert.equal(
        await canonicalDocumentSha256(value),
        '8706b5798a29d65739b3d1bbb1d009f87d005c71c8130c5353c4990c15ddfec8',
    );
    await assert.rejects(
        canonicalDocumentSha256({ unbounded_u64: Number.MAX_SAFE_INTEGER + 1 }),
        /inexact JSON number/u,
    );
});

test('maximum simulation seed matches the Rust-generated canonical digest fixture', async () => {
    const fixtureUrl = new URL('../../test-fixtures/protocol/simulation_seed64_max.json', import.meta.url);
    const digestUrl = new URL('../../test-fixtures/protocol/simulation_seed64_max.sha256', import.meta.url);
    const fixtureText = readFileSync(fixtureUrl, 'utf8').trimEnd();
    const fixture = JSON.parse(fixtureText) as unknown;
    assert.equal(JSON.stringify(fixture), fixtureText);
    assert.equal(await canonicalDocumentSha256(fixture), readFileSync(digestUrl, 'utf8').trim());
});

test('build and content manifests are recursively strict and content-addressed', async () => {
    const artifact = (digest: string, mediaType: string): Record<string, unknown> => ({
        sha256: digest,
        byte_length: 42,
        media_type: mediaType,
    });
    const buildManifest = {
        schema_version: 1,
        source_commit: 'c'.repeat(40),
        cargo_lock_sha256: sha('a'),
        target_triple: 'wasm32-unknown-unknown',
        cargo_profile: 'wasm-release',
        cargo_features: ['replay', 'web'],
        replay_schema_version: runtimeContract.replaySchema,
        save_schema_version: 1,
        network_protocol_version: 1,
        verifier: artifact(sha('b'), 'application/octet-stream'),
        viewer_artifacts: [
            {
                path: 'viewer/entry.js',
                role: { kind: 'entry_java_script' },
                artifact: artifact(sha('c'), 'text/javascript'),
            },
            {
                path: 'viewer/game.wasm',
                role: { kind: 'web_assembly' },
                artifact: artifact(sha('d'), 'application/wasm'),
            },
            {
                path: 'viewer/support.js',
                role: { kind: 'java_script_module', name: 'support.js' },
                artifact: artifact(sha('e'), 'text/javascript'),
            },
        ],
    };
    const buildDigest = await canonicalDocumentSha256(buildManifest);
    const parsedBuild = await parseAndVerifyBuildManifest(buildManifest, buildDigest);
    assert.equal(parsedBuild.schemaVersion, 1);
    if (parsedBuild.schemaVersion !== 1) throw new Error('fixture unexpectedly parsed as BuildManifestV2');
    assert.equal(parsedBuild.viewerArtifacts.length, 3);
    assert.equal(parseBuildManifest(buildManifest).sourceCommit, 'c'.repeat(40));
    await assert.rejects(parseAndVerifyBuildManifest({ ...buildManifest, cargo_profile: 'release' }, buildDigest), /canonical SHA-256/u);
    assert.throws(() => parseBuildManifest({
        ...buildManifest,
        viewer_artifacts: [{
            ...buildManifest.viewer_artifacts[0],
            role: { kind: 'entry_java_script', redirect: 'https://evil.invalid' },
        }, buildManifest.viewer_artifacts[1]],
    }), /unknown field redirect/u);
    assert.throws(() => parseBuildManifest({
        ...buildManifest,
        viewer_artifacts: buildManifest.viewer_artifacts.map(item =>
            item.role.kind === 'java_script_module'
                ? { ...item, role: { kind: 'java_script_module', name: 'substituted.js' } }
                : item),
    }), /module name does not match its authenticated viewer path/u);
    assert.throws(() => parseBuildManifest({
        ...buildManifest,
        viewer_artifacts: buildManifest.viewer_artifacts.map(item =>
            item.role.kind === 'java_script_module'
                ? { ...item, artifact: artifact(sha('e'), 'application/javascript') }
                : item),
    }), /must use text\/javascript/u);
    assert.throws(() => parseBuildManifest({
        ...buildManifest,
        viewer_artifacts: buildManifest.viewer_artifacts.map(item =>
            item.role.kind === 'java_script_module'
                ? { ...item, role: { kind: 'auxiliary', name: 'support.js' } }
                : item),
    }), /auxiliary artifacts must not use an executable media type/u);
    assert.throws(() => parseBuildManifest({
        ...buildManifest,
        viewer_artifacts: buildManifest.viewer_artifacts.map(item =>
            item.role.kind === 'java_script_module'
                ? {
                    ...item,
                    role: { kind: 'auxiliary', name: 'support.js' },
                    artifact: artifact(sha('e'), 'application/json'),
                }
                : item),
    }), /auxiliary artifacts must not use an executable path extension/u);
    assert.throws(() => parseBuildManifest({
        ...buildManifest,
        viewer_artifacts: buildManifest.viewer_artifacts.map(item =>
            item.role.kind === 'java_script_module'
                ? {
                    ...item,
                    path: 'viewer/support.json',
                    role: { kind: 'java_script_module', name: 'support.json' },
                }
                : item),
    }), /JavaScript artifacts must use a canonical \.js path/u);
    assert.throws(() => parseBuildManifest({
        ...buildManifest,
        viewer_artifacts: buildManifest.viewer_artifacts.map(item =>
            item.role.kind === 'entry_java_script'
                ? { ...item, path: 'viewer/entry.json' }
                : item),
    }), /JavaScript artifacts must use a canonical \.js path/u);
    assert.throws(() => parseBuildManifest({
        ...buildManifest,
        viewer_artifacts: buildManifest.viewer_artifacts.map(item =>
            item.role.kind === 'web_assembly'
                ? { ...item, path: 'viewer/game.json' }
                : item),
    }), /WASM artifacts must use a canonical \.wasm path/u);

    const contentManifest = {
        schema_version: 1,
        name: 'Leicester Demo',
        edition: 'demo',
        subject: { kind: 'field_mission', mission_id: 'm01' },
        closure: 'static_prepared_mission_content_projection',
        projection_schema_version: 2,
        resource_locale_root: '1033',
        speech_timing: { kind: 'base_installation' },
        components: [
            'profiles', 'loaded_level', 'mission_scripts', 'sprite_simulation_metadata',
            'map_geometry_metadata', 'localized_deterministic_text', 'sound_duration_tables',
            'interface_simulation_metadata',
        ].map((kind, index) => ({
            kind,
            component_schema_version: 1,
            artifact: artifact(String(index + 1).repeat(64),
                'application/vnd.robinhood.simulation-content-component-v2+bitcode'),
        })),
    };
    const contentDigest = await canonicalDocumentSha256(contentManifest);
    assert.equal((await parseAndVerifyContentManifest(contentManifest, contentDigest)).components.length, 8);
    assert.equal(parseContentManifest(contentManifest).name, 'Leicester Demo');
    assert.equal(parseContentManifest(contentManifest).resourceLocaleRoot, '1033');
    assert.throws(() => parseContentManifest({
        ...contentManifest,
        components: contentManifest.components.slice(1),
    }), /complete canonical simulation projection/u);
    for (const invalid of ['', '0', '01033', '123456789', 'en-US', '1033/data', '1033\\data']) {
        assert.throws(
            () => parseContentManifest({ ...contentManifest, resource_locale_root: invalid }),
            /one numeric component/u,
            invalid,
        );
    }
});

test('public BuildManifestV2 is exact, private-authority-free, and keeps origins separated', async () => {
    const artifact = (digest: string, mediaType: string): Record<string, unknown> => ({
        sha256: digest,
        byte_length: 42,
        media_type: mediaType,
    });
    const tool = (version: string, digest: string): Record<string, unknown> => ({
        version,
        authority_sha256: digest,
    });
    const rustToolchain = {
        schema_version: 1,
        channel: 'nightly-2026-08-25',
        components: ['rust-src', 'rustc-codegen-cranelift-preview'],
        targets: ['wasm32-unknown-unknown'],
    };
    const rustToolchainSha256 = await canonicalDocumentSha256(rustToolchain);
    const buildManifest = {
        schema_version: 2,
        source_commit: 'c'.repeat(40),
        cargo_lock_sha256: sha('a'),
        replay_schema_version: 24,
        save_schema_version: 60,
        network_protocol_version: 28,
        verifier: {
            platform: 'x86_64_unknown_linux_musl',
            target_triple: 'x86_64-unknown-linux-musl',
            cargo_profile: 'release',
            cargo_features: [],
            cargo_package: 'robin_replay_verifier',
            cargo_binary: 'robin-replay-verifier',
            linkage: 'fully_static_no_interpreter_or_needed_libraries',
            artifact: artifact(
                sha('b'),
                'application/vnd.robinhood.ranked-replay-verifier-v2',
            ),
        },
        viewer: {
            engine: {
                target_triple: 'wasm32-unknown-unknown',
                cargo_profile: 'wasm-release',
                cargo_features: ['audio'],
                cargo_package: 'robin_rs',
                cargo_binary: 'robin',
                recipe: 'wasm_bindgen_web_binaryen_oz_strip_debug_dwarf_wabt_strip_v1',
                rust_toolchain: rustToolchain,
                rust_toolchain_sha256: rustToolchainSha256,
                wasm_bindgen_cli: tool(
                    '0.2.127',
                    '68ee22d8da662e20a7aa63d43354c59194530378898e2052079b9084580b89ba',
                ),
                binaryen_wasm_opt: tool('132.0.0', sha('d')),
                wabt_wasm_strip: tool('1.0.41', sha('e')),
                artifacts: [{
                    path: 'viewer/robin.js',
                    role: { kind: 'entry_java_script' },
                    artifact: artifact(sha('1'), 'text/javascript'),
                }, {
                    path: 'viewer/robin_bg.wasm',
                    role: { kind: 'web_assembly' },
                    artifact: artifact(sha('2'), 'application/wasm'),
                }],
            },
            pages_shell: {
                recipe: 'pnpm_frozen_lockfile_vite_static_shell_v1',
                node: tool('24.19.0', sha('3')),
                pnpm: tool('12.3.4', sha('4')),
                package_json_sha256: sha('5'),
                pnpm_lock_sha256: sha('6'),
                public_origin_artifacts: [{
                    path: 'index.html',
                    artifact: artifact(sha('7'), 'text/html'),
                }],
            },
            identity_signer: {
                target_triple: 'wasm32-unknown-unknown',
                cargo_profile: 'wasm-release',
                cargo_features: ['identity-signer-bridge'],
                cargo_package: 'robin_rs',
                cargo_binary: 'leaderboard_identity_bridge',
                recipe: 'wasm_bindgen_web_separate_origin_bridge_v1',
                deployment_policy: 'separate_allowlisted_origin_csp_frame_ancestors_and_bridge_sha_v1',
                rust_toolchain: rustToolchain,
                rust_toolchain_sha256: rustToolchainSha256,
                wasm_bindgen_cli: tool(
                    '0.2.127',
                    '68ee22d8da662e20a7aa63d43354c59194530378898e2052079b9084580b89ba',
                ),
                identity_signer_origin_artifacts: [{
                    path: 'identity-signer/bridge/leaderboard_identity_bridge.js',
                    artifact: artifact(sha('a'), 'text/javascript'),
                }, {
                    path: 'identity-signer/bridge/leaderboard_identity_bridge_bg.wasm',
                    artifact: artifact(sha('f'), 'application/wasm'),
                }, {
                    path: 'identity-signer/index.html',
                    artifact: artifact(sha('9'), 'text/html'),
                }],
            },
        },
    };
    const digest = await canonicalDocumentSha256(buildManifest);
    const parsed = await parseAndVerifyBuildManifest(buildManifest, digest);
    assert.equal(parsed.schemaVersion, 2);
    if (parsed.schemaVersion !== 2) throw new Error('fixture unexpectedly parsed as BuildManifestV1');
    assert.deepEqual(parsed.viewer.engine.artifacts.map(item => item.path), [
        'viewer/robin.js', 'viewer/robin_bg.wasm',
    ]);
    assert.deepEqual(parsed.viewer.pagesShell.publicOriginArtifacts.map(item => item.path), ['index.html']);
    assert.equal(parsed.viewer.identitySigner.identitySignerOriginArtifacts.length, 3);

    assert.equal(parsed.viewer.identitySigner.cargoPackage, 'robin_rs');
    const extractedSigner = structuredClone(buildManifest) as Record<string, any>;
    extractedSigner.viewer.identity_signer.cargo_package = 'robin_identity_signer';
    const extractedDigest = await canonicalDocumentSha256(extractedSigner);
    const extracted = await parseAndVerifyBuildManifest(extractedSigner, extractedDigest);
    if (extracted.schemaVersion !== 2) throw new Error('expected V2 build');
    assert.equal(extracted.viewer.identitySigner.cargoPackage, 'robin_identity_signer');
    assert.notEqual(extractedDigest, digest);
    await assert.rejects(parseAndVerifyBuildManifest(extractedSigner, digest), /canonical SHA-256/u);
    extractedSigner.viewer.identity_signer.cargo_package = 'untrusted_signer';
    assert.throws(() => parseBuildManifest(extractedSigner), /cargo_package/u);
    extractedSigner.viewer.identity_signer.cargo_package = 'robin_identity_signer';
    extractedSigner.viewer.identity_signer.cargo_features.push('audio');
    assert.throws(() => parseBuildManifest(extractedSigner), /cargo_features/u);

    for (const privateField of [
        'projection_exporter', 'projection_authority', 'receipt', 'source_tree',
    ]) {
        assert.throws(
            () => parseBuildManifest({ ...buildManifest, [privateField]: { sentinel: true } }),
            new RegExp(`unknown field ${privateField}`, 'u'),
        );
    }
    const nestedPrivate = structuredClone(buildManifest) as Record<string, any>;
    nestedPrivate.viewer.engine.projection_authority_manifest_sha256 = sha('e');
    assert.throws(() => parseBuildManifest(nestedPrivate), /unknown field projection_authority_manifest_sha256/u);

    const signerSubstitution = structuredClone(buildManifest) as Record<string, any>;
    signerSubstitution.viewer.identity_signer.identity_signer_origin_artifacts[2].artifact.sha256 = sha('2');
    assert.throws(() => parseBuildManifest(signerSubstitution), /cross-origin artifact substitution/u);

    const engineSubstitution = structuredClone(buildManifest) as Record<string, any>;
    engineSubstitution.viewer.engine.artifacts[0].path = 'pages/robin.js';
    assert.throws(() => parseBuildManifest(engineSubstitution), /canonical published engine bundle/u);

    const pathEscape = structuredClone(buildManifest) as Record<string, any>;
    pathEscape.viewer.pages_shell.public_origin_artifacts[0].path = '../private/receipt.json';
    assert.throws(() => parseBuildManifest(pathEscape), /canonical relative path/u);

    const wrongToolchainDigest = structuredClone(buildManifest) as Record<string, any>;
    wrongToolchainDigest.viewer.identity_signer.rust_toolchain.channel = 'nightly-2026-08-24';
    assert.throws(() => parseBuildManifest(wrongToolchainDigest), /channel must be nightly-2026-08-25/u);

    await assert.rejects(
        parseAndVerifyBuildManifest({ ...buildManifest, save_schema_version: 59 }, digest),
        /canonical SHA-256/u,
    );
});

test('campaign content catalog is immutable and subject ordered', async () => {
    const catalog = {
        schema_version: 1,
        edition: 'full',
        entries: [{
            subject: { kind: 'field_mission', mission_id: 'm01' },
            content_manifest_sha256: sha('a'),
        }, {
            subject: { kind: 'headquarters', mission_id: 'S00_HQ_MP' },
            content_manifest_sha256: sha('b'),
        }],
    };
    const digest = await canonicalDocumentSha256(catalog);
    assert.equal((await parseAndVerifyCampaignContentManifest(catalog, digest)).entries.length, 2);
    const reversed = { ...catalog, entries: [...catalog.entries].reverse() };
    assert.throws(() => parseCampaignContentManifest(reversed), /canonically ordered/u);
    await assert.rejects(
        parseAndVerifyCampaignContentManifest(catalog, sha('f')),
        /canonical SHA-256/u,
    );
});

test('campaign content catalog cross-binds mission and Full Campaign subjects', () => {
    const catalog = parseCampaignContentManifest({
        schema_version: 1,
        edition: 'full',
        entries: [{
            subject: { kind: 'field_mission', mission_id: 'm01' },
            content_manifest_sha256: sha('a'),
        }, {
            subject: { kind: 'headquarters', mission_id: 'S00_HQ_MP' },
            content_manifest_sha256: sha('a'),
        }],
    });
    const campaignMission = parseRunDetail(missionRun('campaign'));
    assert.equal(campaignContentCatalogDigest(campaignMission), sha('d'));
    assert.doesNotThrow(() => validateRunCampaignContentBinding(campaignMission, catalog));
    assert.throws(
        () => validateRunCampaignContentBinding(campaignMission, null),
        /catalog was omitted/u,
    );

    const aggregate = parseRunDetail(fullCampaignRun());
    assert.equal(campaignContentCatalogDigest(aggregate), sha('d'));
    assert.doesNotThrow(() => validateRunCampaignContentBinding(aggregate, catalog));
    const substituted = {
        ...catalog,
        entries: catalog.entries.map((entry, index) => index === 1
            ? { ...entry, contentManifestSha256: sha('b') }
            : entry),
    };
    assert.throws(
        () => validateRunCampaignContentBinding(aggregate, substituted),
        /session does not match/u,
    );
});

test('replay proof accepts only the current canonical CompactRhrec shape', () => {
    const replay = {
        artifact: {
            sha256: sha('f'),
            byte_length: 123,
            media_type: 'application/x-robin-rhrec+compact',
        },
        replay_schema_version: runtimeContract.replaySchema,
    };
    assert.equal(parseReplayArtifact(replay).replaySchemaVersion, runtimeContract.replaySchema);
    assert.throws(() => parseReplayArtifact({ ...replay, replay_schema_version: runtimeContract.replaySchema - 1 }), new RegExp(`must be ${runtimeContract.replaySchema}`, 'u'));
    assert.throws(() => parseReplayArtifact({
        ...replay,
        artifact: { ...replay.artifact, media_type: 'application/x-robin-rhrec+jsonl' },
    }), /canonical CompactRhrec media type/u);
    assert.throws(() => parseReplayArtifact({
        ...replay,
        format: 'compact_rhrec',
    }), /unknown field format/u);
    assert.throws(() => parseReplayArtifact({ ...replay, download_url: 'https://evil.invalid' }), /unknown field/u);
});

test('rules config and ranking policy are separate strict content-addressed documents', async () => {
    const rulesConfig = rulesConfigDocument();
    const rulesConfigDigest = await canonicalDocumentSha256(rulesConfig);
    assert.equal(
        (await parseAndVerifyRulesConfigIdentity(rulesConfig, rulesConfigDigest)).replaySchemaVersion,
        runtimeContract.replaySchema,
    );
    assert.equal(parseRulesConfigIdentity(rulesConfig).simConfig.difficulty, 'Medium');
    assert.deepEqual(parseRulesConfigIdentity(rulesConfig).rankedSimulationPolicy, {
        version: 1,
        preset: 'standard',
        difficulty: 'medium',
    });
    assert.throws(() => parseRulesConfigIdentity({
        ...rulesConfig,
        sim_config: { ...rulesConfig.sim_config, count: 1.5 },
    }), /safe integer/u);
    assert.throws(() => parseRulesConfigIdentity({ ...rulesConfig, server_default: true }), /unknown field/u);
    assert.throws(() => parseRulesConfigIdentity({
        ...rulesConfig,
        ranked_simulation_policy: { ...rulesConfig.ranked_simulation_policy, server_default: true },
    }), /ranked_simulation_policy contains unknown field server_default/u);
    assert.throws(() => parseRulesConfigIdentity({
        ...rulesConfig,
        ranked_simulation_policy: { ...rulesConfig.ranked_simulation_policy, version: 2 },
    }), /ranked_simulation_policy.version must be 1/u);
    assert.throws(() => parseRulesConfigIdentity({
        ...rulesConfig,
        ranked_simulation_policy: { ...rulesConfig.ranked_simulation_policy, preset: 'custom' },
    }), /ranked_simulation_policy.preset/u);
    assert.throws(() => parseRulesConfigIdentity({
        ...rulesConfig,
        ranked_simulation_policy: { ...rulesConfig.ranked_simulation_policy, difficulty: 'legendary' },
    }), /ranked_simulation_policy.difficulty/u);
    assert.throws(() => parseRulesConfigIdentity({
        ...rulesConfig,
        sim_config: { ...rulesConfig.sim_config, difficulty: 'Hard' },
    }), /does not match sim_config.difficulty/u);

    const policyVariants = [
        ['standard', 'easy', 'Easy', ['standard', 'Standard', 'easy', 'Easy', 'Standard / Easy (v1)']],
        ['standard', 'medium', 'Medium', ['standard', 'Standard', 'normal', 'Normal', 'Standard / Normal (v1)']],
        ['standard', 'hard', 'Hard', ['standard', 'Standard', 'hard', 'Hard', 'Standard / Hard (v1)']],
        ['original_parity', 'easy', 'Easy', ['original', 'Original', 'easy', 'Easy', 'Original / Easy (v1)']],
        ['original_parity', 'medium', 'Medium', ['original', 'Original', 'normal', 'Normal', 'Original / Normal (v1)']],
        ['original_parity', 'hard', 'Hard', ['original', 'Original', 'hard', 'Hard', 'Original / Hard (v1)']],
    ] as const;
    for (const [preset, difficulty, wireDifficulty, expected] of policyVariants) {
        const variant = parseRulesConfigIdentity({
            ...rulesConfig,
            ranked_simulation_policy: { version: 1, preset, difficulty },
            sim_config: { ...rulesConfig.sim_config, difficulty: wireDifficulty },
        });
        const labels = rankedSimulationPolicyLabels(variant.rankedSimulationPolicy);
        assert.deepEqual(
            [labels.presetId, labels.presetName, labels.difficultyId, labels.difficultyName,
                rankedSimulationPolicyDisplay(variant.rankedSimulationPolicy)],
            expected,
        );
    }

    const ruleset = rulesetDocument(rulesConfigDigest);
    const rulesetDigest = await canonicalDocumentSha256(ruleset);
    const parsed = await parseAndVerifyRulesetManifest(ruleset, rulesetDigest);
    assert.deepEqual(parsed.boardScopes, ['individual_level', 'campaign_mission', 'full_campaign']);
    assert.equal(parsed.campaignCompletionPolicy.mode, 'required');
    assert.equal(parsed.canonicalCampaignState.kind, 'full_campaign_genesis');
    assert.equal(parsed.runPreflightGrantPublicKey, sha('9'));
    assert.equal(parseRulesetManifest(ruleset).activeTimeDefinition, 'successful_simulation_ticks');
    assert.doesNotThrow(() => validateRulesetFacetScopeBinding(parsed, ['campaign_mission']));
    assert.doesNotThrow(() => validateRulesetFacetScopeBinding(parsed, [
        'campaign_mission', 'full_campaign',
    ]));
    assert.throws(
        () => validateRulesetFacetScopeBinding(
            { ...parsed, boardScopes: ['individual_level'] },
            ['campaign_mission'],
        ),
        /does not include every scope/u,
    );
    assert.doesNotThrow(() => validateRulesetRulesConfigBinding(parsed, parseRulesConfigIdentity(rulesConfig)));
    for (const [preset, difficulty, wireDifficulty, expected] of policyVariants) {
        const variant = parseRulesConfigIdentity({
            ...rulesConfig,
            ranked_simulation_policy: { version: 1, preset, difficulty },
            sim_config: { ...rulesConfig.sim_config, difficulty: wireDifficulty },
        });
        const [presetId, presetName, difficultyId, difficultyName] = expected;
        const matchingRuleset = {
            ...parsed,
            presetId,
            presetName,
            difficultyId,
            difficultyName,
        };
        assert.doesNotThrow(() => validateRulesetRulesConfigBinding(matchingRuleset, variant));
        for (const substitution of [
            { presetId: 'substituted' },
            { presetName: 'Substituted' },
            { difficultyId: 'substituted' },
            { difficultyName: 'Substituted' },
        ]) {
            assert.throws(
                () => validateRulesetRulesConfigBinding({ ...matchingRuleset, ...substitution }, variant),
                /does not match its immutable ruleset labels/u,
            );
        }
    }
    const demoRuleset = parseRulesetManifest({
        ...ruleset,
        allowed_campaign_content_manifest_sha256: [],
        board_scopes: ['individual_level'],
        campaign_completion_policy: { mode: 'not_offered' },
        canonical_campaign_state: {
            edition: 'demo',
            kind: 'individual_template',
            rules_config_sha256: rulesConfigDigest,
        },
    });
    assert.equal(demoRuleset.campaignCompletionPolicy.mode, 'not_offered');
    assert.deepEqual(demoRuleset.canonicalCampaignState, {
        edition: 'demo',
        kind: 'individual_template',
        rulesConfigSha256: rulesConfigDigest,
    });
    assert.notEqual(rulesetDigest, rulesConfigDigest);
    assert.equal((await parseAndVerifyPublishedRuleset({
        schema_version: 1,
        ruleset_manifest_sha256: rulesetDigest,
        manifest: ruleset,
        operational_status: { status: 'active' },
    }, rulesetDigest)).operationalStatus.status, 'active');
    assert.throws(() => parseRulesetManifest({
        ...ruleset,
        board_scopes: ['campaign_mission', 'individual_level'],
    }), /canonical order/u);
    assert.throws(() => parseRulesetManifest({
        ...ruleset,
        replay_schema_versions: [19],
    }), /provenance policy does not match replay schema/u);
    assert.throws(() => parseRulesetManifest({
        ...ruleset,
        campaign_completion_policy: { mode: 'not_offered' },
    }), /completion policy does not match full-campaign scope/u);
    assert.throws(() => parseRulesetManifest({
        ...ruleset,
        campaign_completion_policy: {
            ...ruleset.campaign_completion_policy,
            policy: { ...ruleset.campaign_completion_policy.policy, required_progression_percent: 101 },
        },
    }), /must be between 1 and 100/u);
    assert.throws(() => parseRulesetManifest({
        ...ruleset,
        campaign_completion_policy: {
            ...ruleset.campaign_completion_policy,
            server_default: true,
        },
    }), /campaign_completion_policy contains unknown field server_default/u);
    assert.throws(() => parseRulesetManifest({
        ...ruleset,
        canonical_campaign_state: { ...ruleset.canonical_campaign_state, edition: 'demo' },
    }), /do not identify the same canonical state family/u);
    assert.throws(() => parseRulesetManifest({
        ...ruleset,
        canonical_campaign_state: { ...ruleset.canonical_campaign_state, object_sha256: sha('7') },
    }), /canonical_campaign_state contains unknown field object_sha256/u);
    assert.throws(() => parseRulesetManifest({
        ...ruleset,
        canonical_campaign_state: { ...ruleset.canonical_campaign_state, rules_config_sha256: sha('8') },
    }), /does not match rules_config_sha256/u);
    assert.throws(() => parseRulesetManifest({
        ...ruleset,
        run_preflight_grant_public_key: sha('0'),
    }), /must not be the zero digest/u);
    assert.throws(() => parseRulesetManifest({
        ...ruleset,
        canonical_start_policy: 'operator_individual_template_or_genesis_and_verified_predecessor',
    }), /canonical_start_policy/u);
    await assert.rejects(
        parseAndVerifyRulesetManifest({ ...ruleset, allow_state_load: true }, rulesetDigest),
        /canonical SHA-256/u,
    );
});

test('owner-operation parsers retain exact signed claims', () => {
    assert.deepEqual(parseUsernameChallenge({
        schema_version: 1,
        username_challenge_id: 'rename-1',
        username_challenge_nonce: '2'.repeat(64),
        expires_at_unix_ms: 1_800_000_000_000,
    }), { id: 'rename-1', nonce: '2'.repeat(64), expiresAtUnixMs: 1_800_000_000_000 });
    assert.equal(parseUsernameUpdateEnvelope({
        schema_version: 1,
        username_challenge_id: 'rename-1',
        username_challenge_nonce: '2'.repeat(64),
        public_key: publicKey,
        username: 'Robin',
        signature: '3'.repeat(128),
    }).public_key, publicKey);

    const challenge = parseDeletionChallenge({
        schema_version: 1,
        deletion_challenge_id: 'delete-1',
        deletion_challenge_nonce: '4'.repeat(64),
        expires_at_unix_ms: 1_800_000_000_000,
        public_key: publicKey,
        target: { kind: 'run', run_id: 'run-1' },
    });
    assert.deepEqual(parseDeletionRequestEnvelope({
        schema_version: 1,
        challenge: {
            schema_version: 1,
            deletion_challenge_id: challenge.deletion_challenge_id,
            deletion_challenge_nonce: challenge.deletion_challenge_nonce,
            expires_at_unix_ms: challenge.expires_at_unix_ms,
            public_key: challenge.public_key,
            target: challenge.target,
        },
        signature: '5'.repeat(128),
    }).challenge.target, { kind: 'run', run_id: 'run-1' });
});
