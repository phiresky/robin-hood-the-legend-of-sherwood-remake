import assert from 'node:assert/strict';
import test from 'node:test';
import { loadReplayCheckpoints } from './replay-checkpoints.ts';

test('sidecar download waits for replay startup and installs after background validation', async () => {
    let started = false, downloaded = false, installed = false;
    const bytes = new Uint8Array([1]);
    await loadReplayCheckpoints(async <T>() => ({ replay: started ? { frame: 12 } : null }) as T,
        new AbortController().signal, {
            wait: async () => { assert.equal(downloaded, false); started = true; },
            download: async () => { assert(started); downloaded = true; return bytes; },
            validate: async data => { assert.equal(data, bytes); assert.equal(installed, false); },
            install: data => { assert.equal(data, bytes); installed = true; },
        });
    assert(installed);
});

test('retired playback cannot install an in-flight sidecar', async () => {
    const controller = new AbortController();
    await assert.rejects(loadReplayCheckpoints(async <T>() => ({ replay: { frame: 0 } }) as T,
        controller.signal, {
            download: async () => new Uint8Array([1]),
            validate: async () => { controller.abort(); },
            install: () => assert.fail('retired playback was changed'),
        }), { name: 'AbortError' });
});

test('missing optional sidecar leaves playback usable', async () => {
    await loadReplayCheckpoints(async <T>() => ({ replay: { frame: 0 } }) as T,
        new AbortController().signal, {
            download: async () => new Uint8Array(),
            validate: async () => assert.fail(), install: () => assert.fail(),
        });
});
