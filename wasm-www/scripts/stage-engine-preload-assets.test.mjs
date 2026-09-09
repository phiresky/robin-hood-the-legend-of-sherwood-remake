import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdir, mkdtemp, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import { stageEnginePreloadAssets } from './stage-engine-preload-assets.mjs';

async function fixture() {
    const root = await mkdtemp(resolve(tmpdir(), 'engine-preload-'));
    const core = resolve(root, 'core');
    const output = resolve(root, 'output');
    await mkdir(resolve(core, 'Data/Interface/Fonts'), { recursive: true });
    await mkdir(resolve(core, 'Data/Interface/UI'), { recursive: true });
    await mkdir(output);
    await writeFile(resolve(core, 'Data/AudioDurations.json'), '{}');
    await writeFile(resolve(core, 'Data/Interface/Fonts/arial.ttf'), 'font');
    await writeFile(resolve(core, 'Data/Interface/UI/z.png'), 'z');
    await writeFile(resolve(core, 'Data/Interface/UI/a.png'), 'a');
    await writeFile(resolve(core, 'Data/Interface/UI/ignored.txt'), 'ignored');
    return { root, core, output };
}

test('engine preload closure is sorted and excludes unrelated overlay files', async t => {
    const value = await fixture();
    t.after(() => rm(value.root, { recursive: true, force: true }));
    const manifest = await stageEnginePreloadAssets(value.core, value.output);
    assert.deepEqual(manifest.map(entry => entry.path), [
        'Data/AudioDurations.json',
        'Data/Interface/Fonts/arial.ttf',
        'Data/Interface/UI/a.png',
        'Data/Interface/UI/z.png',
    ]);
    assert.equal(await readFile(resolve(value.output, 'Data/Interface/UI/a.png'), 'utf8'), 'a');
});

test('engine preload staging rejects required symlinks', async t => {
    const value = await fixture();
    t.after(() => rm(value.root, { recursive: true, force: true }));
    await rm(resolve(value.core, 'Data/Interface/Fonts/arial.ttf'));
    await symlink(resolve(value.core, 'Data/Interface/UI/a.png'), resolve(value.core, 'Data/Interface/Fonts/arial.ttf'));
    await assert.rejects(stageEnginePreloadAssets(value.core, value.output), /not a regular file/u);
});

test('missing authoritative timings reject browser staging', async t => {
    const value = await fixture();
    t.after(() => rm(value.root, { recursive: true, force: true }));
    await rm(resolve(value.core, 'Data/AudioDurations.json'));
    await assert.rejects(stageEnginePreloadAssets(value.core, value.output), /AudioDurations/u);
});
