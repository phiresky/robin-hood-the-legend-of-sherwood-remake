import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdir, mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import { stageCloudflareHeaders } from './stage-cloudflare-headers.mjs';

test('header staging selects exactly one checked-in origin policy', async t => {
    const root = await mkdtemp(resolve(tmpdir(), 'robin-cf-headers-'));
    t.after(() => rm(root, { recursive: true, force: true }));
    for (const kind of ['public', 'runtime', 'datadir', 'signer']) {
        const output = resolve(root, kind);
        await mkdir(output);
        await stageCloudflareHeaders(kind, output);
        const text = await readFile(resolve(output, '_headers'), 'utf8');
        assert.match(text, new RegExp(`X-Robinhood-Static-Origin: ${kind}-v1`, 'u'));
    }
    await assert.rejects(stageCloudflareHeaders('unknown', root), /exactly public/u);
});
