import {
    boardDocument,
    cursorFor,
    leaderboardEntry,
    leaderboardPage,
    metadataDocument,
    playerHistoryPage,
    publicFingerprint,
    publicKey,
    runDetail,
    runFilter,
    sha,
    uploader,
} from './model-fixtures.js';
import assert from 'node:assert/strict';
import test from 'node:test';
import {
    SIGNED_REQUEST_DOMAINS,
    parseDeletionRequestClaim,
    parsePlayerProfile,
    parseSignedDeletionRequest,
    parseSignedSubmissionOwnerStatusRequest,
    parseSignedUsernameUpdate,
    parseSubmissionOwnerStatusClaim,
    parseSubmissionOwnerStatusResponse,
    parseUsernameUpdateClaim,
    signedRequestSigningBytes,
} from './account-contract.js';
import { canonicalDocumentSha256, canonicalDocumentSha256Sync } from './canonical.js';
import { formatActiveTime, formatMetricValue } from './format.js';
import {
    parseBoardMetadata,
    parseBoardPage,
    parsePlayerRunHistoryPage,
    parseRunDetail,
} from './public-response.js';

test('board metadata parses V2 boards with fixed and any-config policies', () => {
    const metadata = parseBoardMetadata(metadataDocument());
    assert.deepEqual(metadata.tickDuration, { numeratorMicros: 50_000, denominator: 1 });
    // Rust String order: "demo-…" < "full-…".
    assert.deepEqual(metadata.boards.map(board => board.boardId), ['demo-standard-normal', 'full-any-config']);
    const [demo, full] = metadata.boards;
    assert.deepEqual(demo?.simulationPolicy, { kind: 'fixed', policy: { version: 1, preset: 'standard', difficulty: 'medium' } });
    assert.equal(demo?.missions[1]?.displayName, 'Lincoln');
    assert.deepEqual(full?.simulationPolicy, { kind: 'any_config' });
    assert.equal(full?.viewerContentRequirement, 'user_local_retail');
});

test('board metadata rejects removed V1 documents, unknown fields and non-canonical collections', () => {
    const mutate = (change: (document: Record<string, unknown>) => void): Record<string, unknown> => {
        const document = structuredClone(metadataDocument());
        change(document);
        return document;
    };
    assert.throws(() => parseBoardMetadata({ schema_version: 1, missions: [], rulesets: [], competitions: [], full_campaign: null }), /schema_version must be 2|unknown field/u);
    assert.throws(() => parseBoardMetadata(mutate(document => { (document.boards as unknown[]).reverse(); })), /strictly ordered/u);
    assert.throws(() => parseBoardMetadata(mutate(document => { (document.boards as unknown[]).push(boardDocument()); })), /strictly ordered/u);
    assert.throws(() => parseBoardMetadata(mutate(document => {
        (document.boards as Record<string, unknown>[])[0]!.ruleset_manifest_sha256 = sha('a');
    })), /unknown field ruleset_manifest_sha256/u);
    assert.throws(() => parseBoardMetadata(mutate(document => {
        (document.boards as Record<string, unknown>[])[0]!.metrics = ['fastest_success', 'original_score'];
    })), /canonical order/u);
    assert.throws(() => parseBoardMetadata(mutate(document => {
        (document.boards as Record<string, unknown>[])[0]!.missions = [
            { mission_id: 'M', display_name: 'A' }, { mission_id: 'M', display_name: 'B' },
        ];
    })), /unique missions/u);
    assert.throws(() => parseBoardMetadata(mutate(document => {
        (document.boards as Record<string, unknown>[])[0]!.simulation_policy = {
            kind: 'fixed', policy: { version: 1, preset: 'custom', difficulty: 'custom' },
        };
    })), /Standard or Original/u);
    assert.throws(() => parseBoardMetadata(mutate(document => {
        document.tick_duration = { numerator_micros: 100, denominator: 2 };
    })), /reduced canonical fraction/u);
});

test('leaderboard pages validate uploader, counts, order and competition ranks', () => {
    const page = parseBoardPage(leaderboardPage());
    assert.equal(page.entries.length, 3);
    assert.deepEqual(page.entries[0]?.uploader, {
        seat: 0, username: 'Robin', publicKey, publicKeyFingerprint: publicFingerprint,
    });
    assert.equal(page.entries[1]?.uploader, null);
    assert.equal(page.filter.playerPublicKey, null);

    assert.throws(() => parseBoardPage(leaderboardPage([leaderboardEntry(1, 1, 400, 1), leaderboardEntry(2, 2, 500, 2)])), /primary metric order/u);
    assert.throws(() => parseBoardPage(leaderboardPage([leaderboardEntry(1, 1, 500, 1), leaderboardEntry(2, 2, 500, 2)])), /tied entries/u);
    const guestUploader = leaderboardEntry(1, 1, 500, 1);
    guestUploader.uploader = { ...uploader, seat: 1 };
    assert.throws(() => parseBoardPage(leaderboardPage([guestUploader])), /host seat 0/u);
    const wrongFingerprint = leaderboardEntry(1, 1, 500, 1);
    wrongFingerprint.uploader = { ...uploader, public_key_fingerprint: '2'.repeat(32) };
    assert.throws(() => parseBoardPage(leaderboardPage([wrongFingerprint])), /canonical public-key fingerprint/u);
    const tooFewInstances = leaderboardEntry(1, 1, 500, 1);
    tooFewInstances.max_concurrent_players = 3;
    tooFewInstances.participant_instance_count = 2;
    assert.throws(() => parseBoardPage(leaderboardPage([tooFewInstances])), /participant_instance_count/u);
    const wrongMetric = leaderboardEntry(1, 1, 500, 1);
    wrongMetric.metric_value = { metric: 'fastest_success', active_simulation_ticks: 5 };
    assert.throws(() => parseBoardPage(leaderboardPage([wrongMetric])), /page metric/u);
    const legacyTick = leaderboardEntry(1, 1, 500, 1);
    legacyTick.metric_value = { metric: 'original_score', points: 1, tick_duration: { numerator_micros: 1, denominator: 1 } };
    assert.throws(() => parseBoardPage(leaderboardPage([legacyTick])), /unknown field tick_duration/u);
    const legacyRoster = leaderboardEntry(1, 1, 500, 1);
    legacyRoster.named_participants = [];
    assert.throws(() => parseBoardPage(leaderboardPage([legacyRoster])), /unknown field named_participants/u);
});

test('leaderboard cursors bind the canonical filter digest, snapshot and final row', async () => {
    const document = leaderboardPage();
    const entries = document.entries as Record<string, unknown>[];
    document.next_cursor = cursorFor(document, entries[2]!, 'next-token');
    const page = parseBoardPage(document);
    assert.equal(page.nextCursor, 'next-token');
    assert.equal(page.nextCursorDocument?.querySha256, await canonicalDocumentSha256(document.filter));

    const second = {
        ...leaderboardPage([leaderboardEntry(4, 4, 300, 4)]),
        previous_cursor: cursorFor(document, entries[2]!, 'next-token'),
    };
    assert.equal(parseBoardPage(second).previousCursor?.opaqueToken, 'next-token');
    assert.throws(() => parseBoardPage({ ...second, entries: [leaderboardEntry(5, 5, 300, 4)] }), /gap-free/u);
    const otherFilter = { ...second, filter: runFilter({ mission_id: 'Dem_Lin_MP' }) };
    assert.throws(() => parseBoardPage(otherFilter), /previous leaderboard cursor/u);
    const wrongLast = structuredClone(document);
    wrongLast.next_cursor = cursorFor(document, entries[1]!, 'next-token');
    assert.throws(() => parseBoardPage(wrongLast), /final page row/u);
    assert.throws(() => parseBoardPage({ ...leaderboardPage([]), next_cursor: cursorFor(document, entries[2]!, 'x') }), /empty leaderboard page/u);

    const withPlayer = leaderboardPage();
    withPlayer.filter = runFilter({ player_public_key: publicKey });
    assert.equal(parseBoardPage(withPlayer).filter.playerPublicKey, publicKey);
});

test('run detail exposes the verified V2 facts and binds the viewer to the recorded engine', () => {
    const run = parseRunDetail(runDetail());
    assert.equal(run.edition, 'demo');
    assert.equal(run.uploader?.username, 'Robin');
    assert.equal(run.recordedEngineVersion, '0123456789ab');
    assert.deepEqual(run.viewer, { availability: { status: 'available' }, contentRequirement: 'bundled_demo', runtimeBuild: '0123456789ab' });
    assert.deepEqual(run.simConfig.nested, { value: 3 });
    assert.deepEqual(run.achievements.map(item => [item.id, item.evaluation]), [['quiet-hands', 'earned'], ['unseen', 'unverifiable']]);

    const anonymous = runDetail();
    anonymous.uploader = null;
    assert.equal(parseRunDetail(anonymous).uploader, null);
    const unavailable = runDetail();
    unavailable.viewer = { availability: { status: 'unavailable', safe_reason: 'Runtime build retired.' }, content_requirement: 'user_local_retail', runtime_build: '0123456789ab' };
    assert.deepEqual(parseRunDetail(unavailable).viewer.availability, { status: 'unavailable', safeReason: 'Runtime build retired.' });

    const substitutedBuild = runDetail();
    (substitutedBuild.viewer as Record<string, unknown>).runtime_build = 'ffffffffffff';
    assert.throws(() => parseRunDetail(substitutedBuild), /recorded engine version/u);
    for (const legacy of ['build', 'content', 'rules_config_sha256', 'full_campaign_sessions', 'verification_proof']) {
        assert.throws(() => parseRunDetail({ ...runDetail(), [legacy]: null }), new RegExp(`unknown field ${legacy}`, 'u'));
    }
    assert.throws(() => parseRunDetail({ ...runDetail(), sim_config: [] }), /sim_config must be an object/u);
    const wrongMedia = runDetail();
    ((wrongMedia.replay as Record<string, unknown>).artifact as Record<string, unknown>).media_type = 'application/jsonl';
    assert.throws(() => parseRunDetail(wrongMedia), /media_type/u);
    const { viewer: _viewer, ...missingViewer } = runDetail();
    assert.throws(() => parseRunDetail(missingViewer), /missing required field viewer/u);
});

test('player history parses V2 run summaries and personal bests', () => {
    const page = parsePlayerRunHistoryPage(playerHistoryPage());
    assert.equal(page.player.username, '<img src=x onerror=alert(1)>');
    assert.equal(page.runs[0]?.run.boardId, 'demo-standard-normal');
    assert.equal(page.personalBests[0]?.filter.playerPublicKey, publicKey);
    assert.equal(page.nextCursor, 'authenticated-cursor');

    const substituted = structuredClone(playerHistoryPage());
    (substituted.runs as Record<string, unknown>[])[0]!.player_public_key = sha('2');
    assert.throws(() => parsePlayerRunHistoryPage(substituted), /different public key/u);
    const duplicateRuns = structuredClone(playerHistoryPage());
    (duplicateRuns.runs as unknown[]).push(structuredClone((duplicateRuns.runs as unknown[])[0]));
    assert.throws(() => parsePlayerRunHistoryPage(duplicateRuns), /duplicate run identities/u);
    const duplicateBests = structuredClone(playerHistoryPage());
    (duplicateBests.personal_bests as unknown[]).push(structuredClone((duplicateBests.personal_bests as unknown[])[0]));
    assert.throws(() => parsePlayerRunHistoryPage(duplicateBests), /duplicate personal-best filters/u);
    const unnamedBest = structuredClone(playerHistoryPage());
    (unnamedBest.personal_bests as Record<string, unknown>[])[0]!.filter = runFilter();
    assert.throws(() => parsePlayerRunHistoryPage(unnamedBest), /must name its player/u);
    const wrongMetric = structuredClone(playerHistoryPage());
    (wrongMetric.personal_bests as Record<string, unknown>[])[0]!.metric_value = { metric: 'fastest_success', active_simulation_ticks: 1 };
    assert.throws(() => parsePlayerRunHistoryPage(wrongMetric), /does not match its board filter/u);
    const legacyRun = structuredClone(playerHistoryPage());
    ((legacyRun.runs as Record<string, unknown>[])[0]!.run as Record<string, unknown>).ruleset_manifest_sha256 = sha('d');
    assert.throws(() => parsePlayerRunHistoryPage(legacyRun), /unknown field ruleset_manifest_sha256/u);
});

test('metrics format scores and active time with the published tick duration', () => {
    const tick = { numeratorMicros: 50_000, denominator: 1 };
    assert.equal(formatActiveTime(1200, tick), '1:00.0');
    assert.equal(formatMetricValue({ metric: 'fastest_success', activeSimulationTicks: 30 }, { numeratorMicros: 100_000, denominator: 3 }), '0:01.0');
    assert.match(formatMetricValue({ metric: 'original_score', points: 1234 }, tick), /1.?234/u);
});

test('public identity fingerprints are derived from the exact key and hostile names remain inert text', () => {
    const profile = parsePlayerProfile({
        schema_version: 1,
        username: '<img src=x onerror=alert(1)>',
        public_key: publicKey,
        public_key_fingerprint: publicFingerprint,
    });
    assert.equal(profile.username, '<img src=x onerror=alert(1)>');
    assert.equal(parsePlayerProfile({
        schema_version: 1,
        username: 'Protocol vector',
        public_key: 'ab'.repeat(32),
        public_key_fingerprint: '4386e85fa8fe41e53c9be90d18458bbd',
    }).publicKeyFingerprint, '4386e85fa8fe41e53c9be90d18458bbd');
    assert.throws(() => parsePlayerProfile({
        schema_version: 1,
        username: 'Robin‮exe',
        public_key: publicKey,
        public_key_fingerprint: publicFingerprint,
    }), /control characters/u);
});

const signedAt = 1_800_000_000_000;
const signed = (request: unknown, signature = '3'.repeat(128)) => ({ schema_version: 2, request, algorithm: 'ed25519', signature });

test('owner-operation parsers retain exact timestamped signed claims', () => {
    const rename = { schema_version: 2, public_key: publicKey, signed_at_unix_ms: signedAt, username: 'Robin' };
    assert.deepEqual(parseSignedUsernameUpdate(signed(rename)).request, rename);
    const deletion = { schema_version: 2, public_key: publicKey, signed_at_unix_ms: signedAt, target: { kind: 'run', run_id: 'run-1' } };
    assert.deepEqual(parseSignedDeletionRequest(signed(deletion)).request.target, { kind: 'run', run_id: 'run-1' });
    const status = { schema_version: 2, public_key: publicKey, signed_at_unix_ms: signedAt, submission_id: 'sub-1' };
    assert.equal(parseSignedSubmissionOwnerStatusRequest(signed(status)).request.submission_id, 'sub-1');

    // Removed challenge embeddings and V1 envelopes are rejected.
    assert.throws(() => parseUsernameUpdateClaim({ ...rename, username_challenge_nonce: '2'.repeat(64) }), /unknown field username_challenge_nonce/u);
    assert.throws(() => parseDeletionRequestClaim({ ...deletion, schema_version: 1 }), /schema_version must be 2/u);
    assert.throws(() => parseSignedDeletionRequest({ schema_version: 1, challenge: deletion, signature: '5'.repeat(128) }), /unknown field challenge|schema_version must be 2/u);
    assert.throws(() => parseSubmissionOwnerStatusClaim({ ...status, signed_at_unix_ms: 0 }), /positive/u);
    assert.throws(() => parseSignedUsernameUpdate({ ...signed(rename), algorithm: 'rsa' }), /algorithm/u);
    assert.throws(() => parseSignedUsernameUpdate(signed(rename, '0'.repeat(128))), /non-zero 128/u);
});

test('signing bytes are the NUL-terminated domain followed by the canonical claim', () => {
    const claim = { username: 'Robin', signed_at_unix_ms: signedAt, public_key: publicKey, schema_version: 2 };
    assert.equal(
        new TextDecoder().decode(signedRequestSigningBytes(SIGNED_REQUEST_DOMAINS.usernameUpdate, claim)),
        `robinhood/leaderboards/2/username-update\0{"public_key":"${publicKey}","schema_version":2,"signed_at_unix_ms":${signedAt},"username":"Robin"}`,
    );
    assert.equal(SIGNED_REQUEST_DOMAINS.submission, 'robinhood/leaderboards/3/submission\0');
    assert.equal(SIGNED_REQUEST_DOMAINS.deletionRequest, 'robinhood/leaderboards/2/deletion-request\0');
    assert.equal(SIGNED_REQUEST_DOMAINS.submissionOwnerStatus, 'robinhood/leaderboards/2/submission-owner-status\0');
});

test('owner status responses bind the submission, owner key and exact signed request digest', () => {
    const request = parseSignedSubmissionOwnerStatusRequest(signed({
        schema_version: 2, public_key: publicKey, signed_at_unix_ms: signedAt, submission_id: 'sub-1',
    }));
    const response = (state: unknown, overrides: Record<string, unknown> = {}) => ({
        schema_version: 2, submission_id: 'sub-1', public_key: publicKey,
        request_sha256: canonicalDocumentSha256Sync(request), state, ...overrides,
    });
    assert.deepEqual(parseSubmissionOwnerStatusResponse(response({ state: 'accepted', run_id: 'run-1' }), request).lifecycle,
        { state: 'accepted', runId: 'run-1' });
    assert.deepEqual(parseSubmissionOwnerStatusResponse(response({ state: 'rejected', code: 'state_hash_mismatch', safe_message: 'Desync.' }), request).lifecycle,
        { state: 'rejected', code: 'state_hash_mismatch', safeMessage: 'Desync.' });
    for (const mismatch of [
        { submission_id: 'sub-2' }, { public_key: sha('2') }, { request_sha256: sha('3') },
    ]) {
        assert.throws(() => parseSubmissionOwnerStatusResponse(response({ state: 'queued' }, mismatch), request), /does not answer/u);
    }
    assert.throws(() => parseSubmissionOwnerStatusResponse(response({ state: 'queued' }, { schema_version: 1 }), request), /schema_version must be 2/u);
    assert.throws(() => parseSubmissionOwnerStatusResponse(response({ state: 'failed', code: 'other', safe_message: 'x' }), request), /code/u);
    assert.throws(() => parseSubmissionOwnerStatusResponse(response({ state: 'queued', run_id: 'run-1' }), request), /unknown field run_id/u);
});
