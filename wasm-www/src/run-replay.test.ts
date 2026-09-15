import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import test from 'node:test';
import { fetchRunReplay, parseRunLaunch, runFromQuery, RANKED_REPLAY_MEDIA_TYPE } from './run-replay.ts';

const build = '0123456789ab';
const replay = new TextEncoder().encode(`rhrec-${build}-AAAA`);

function runDocument(overrides: Record<string, unknown> = {}): Record<string, unknown> {
    return {
        schema_version: 2,
        run_id: 'run-1',
        recorded_engine_version: build,
        replay: {
            artifact: {
                sha256: createHash('sha256').update(replay).digest('hex'),
                byte_length: replay.byteLength,
                media_type: RANKED_REPLAY_MEDIA_TYPE,
            },
            replay_schema_version: 44,
        },
        viewer: { availability: { status: 'available' }, content_requirement: 'bundled_demo', runtime_build: build },
        ...overrides,
    };
}

function api(document: unknown, replayBytes: Uint8Array = replay, replayType = RANKED_REPLAY_MEDIA_TYPE) {
    const urls: string[] = [];
    const fetchImpl = (async (input: RequestInfo | URL) => {
        const url = String(input);
        urls.push(url);
        if (url.endsWith('/replay')) {
            return new Response(replayBytes.slice(), { headers: { 'content-type': replayType } });
        }
        return new Response(JSON.stringify(document), { headers: { 'content-type': 'application/json' } });
    }) as typeof fetch;
    return { fetchImpl, urls };
}

test('run query ids are bounded opaque text', () => {
    assert.equal(runFromQuery(new URLSearchParams('run=run-1')), 'run-1');
    assert.equal(runFromQuery(new URLSearchParams('')), null);
    assert.throws(() => runFromQuery(new URLSearchParams('run=%20bad')), /valid run id/u);
});

test('a run launches its recorded engine build with the exact published replay', async () => {
    const { fetchImpl, urls } = api(runDocument());
    const result = await fetchRunReplay('run-1', 'https://robinhood.example/api/v1/', fetchImpl, new AbortController().signal);
    assert.deepEqual(result, { runId: 'run-1', runtimeBuild: build, content: `rhrec-${build}-AAAA` });
    assert.deepEqual(urls, ['https://robinhood.example/api/v1/runs/run-1', 'https://robinhood.example/api/v1/runs/run-1/replay']);
});

test('run playback refuses unavailable, Full, mismatched and tampered runs', async () => {
    const signal = new AbortController().signal;
    assert.throws(() => parseRunLaunch(runDocument({ run_id: 'other' }), 'run-1'), /different or unsupported/u);
    assert.throws(() => parseRunLaunch(runDocument({ viewer: {
        availability: { status: 'unavailable', safe_reason: 'Build retired.' }, content_requirement: 'bundled_demo', runtime_build: build,
    } }), 'run-1'), /Build retired/u);
    assert.throws(() => parseRunLaunch(runDocument({ viewer: {
        availability: { status: 'available' }, content_requirement: 'user_local_retail', runtime_build: build,
    } }), 'run-1'), /local Full installation/u);
    assert.throws(() => parseRunLaunch(runDocument({ recorded_engine_version: 'ffffffffffff' }), 'run-1'), /published engine build/u);
    assert.throws(() => parseRunLaunch(runDocument({ viewer: {
        availability: { status: 'available' }, content_requirement: 'bundled_demo', runtime_build: '../latest',
    }, recorded_engine_version: '../latest' }), 'run-1'), /published engine build/u);

    const tampered = replay.slice();
    tampered[tampered.length - 1] = 0x42;
    await assert.rejects(fetchRunReplay('run-1', '/api/v1', api(runDocument(), tampered).fetchImpl, signal), /published identity/u);
    await assert.rejects(fetchRunReplay('run-1', '/api/v1', api(runDocument(), replay, 'application/jsonl').fetchImpl, signal), /did not return/u);
    const otherBuild = new TextEncoder().encode('rhrec-ffffffffffff-AAAA');
    const otherDocument = runDocument();
    ((otherDocument.replay as Record<string, unknown>).artifact as Record<string, unknown>).sha256 = createHash('sha256').update(otherBuild).digest('hex');
    await assert.rejects(fetchRunReplay('run-1', '/api/v1', api(otherDocument, otherBuild).fetchImpl, signal), /not recorded by engine build/u);
});
