import test from 'node:test';
import assert from 'node:assert/strict';
import { encode } from 'fast-png';
import { sceneryThumbnailFixture } from '../tests/scenery-thumbnail-fixture.ts';
import { resolveScenerySourceCorrespondence } from './scenery-source-correspondence.ts';
import { missionStateDataHash } from './mission-state-layer.ts';
import type { Level3D } from '@rle/shared';
import type { ProtoLevel } from '../../shared/src/level.ts';
import type { NativeStatePresentationContract } from '../../shared/src/native-state-presentation.ts';
const hash = async (bytes: Uint8Array) => [...new Uint8Array(await crypto.subtle.digest('SHA-256', bytes))].map(n => n.toString(16).padStart(2, '0')).join('');
async function fixture() {
  const bank = await sceneryThumbnailFixture(true, true);
  const animation = { sprite: { frame_profile_name: 'fire', profile_name: 'burning', position_x: -1, position_y: -1, elevation: 0 }, blit_type: 0, active: true, force_display: false, display_polyline: [] };
  const level = { animations: [animation] } as unknown as ProtoLevel;
  const frames = [];
  for (const [index, values] of [[0, [0, 0, 0, 0, 255, 0, 0, 255]], [1, Array(8).fill(0)]] as const) {
    const bytes = encode({ width: 2, height: 1, channels: 4, data: new Uint8Array(values) });
    const path = `source/${index}.png`; bank.files.set(path, bytes);
    frames.push({ path, sha256: await hash(bytes), width: 2, height: 1, delay: index ? 4 : 2, offset: index ? [3, -2] : [0, 0] });
  }
  const native = { elements: [{ id: 'source0', source: { kind: 'map-animation', index: 0, sha256: await missionStateDataHash(animation) }, active: true, loop: true, frames }] } as unknown as NativeStatePresentationContract;
  const document: Level3D = { version: 1, map: 'Authored', size: null, camera: { kind: 'oblique-orthographic', elevation_deg: 35 }, sceneAssets: [], groups: [], assetSources: [{ id: 'fire', model: 'model.glb', model_sha256: 'a'.repeat(64), descriptor: 'asset.json', descriptor_sha256: 'b'.repeat(64) }], objects: [{ id: 'placed', kind: 'scenery', node: 'asset:fire:scenery-effect', source: { map: 'Authored' }, transform: { dx: 0, dy: 0, dz: 0, rot_deg: 0 } }] };
  const descriptors = new Map([['fire', bank.descriptor]]);
  const run = () => resolveScenerySourceCorrespondence(document, descriptors, native, level, bank.read);
  return { bank, native, document, level, run };
}
test('placed source identity and decoded RGBA admit re-encoded legacy artwork', async () => {
  const f = await fixture(); const result = await f.run();
  assert.deepEqual(result.bindings, [{ effectId: 'placed/fire', descriptorSha256: 'b'.repeat(64), sourceIndex: 0, sourceSha256: f.native.elements[0]!.source.sha256 }]);
  assert.equal(result.identities.size, 1);
});
test('a separately placed palette duplicate does not acquire the mission clock by name', async () => {
  const f = await fixture(); f.document.objects[0]!.transform.dx = 1;
  assert.deepEqual((await f.run()).bindings, []);
});
test('duplicate source owners, changed source pins and changed frame metadata are rejected', async () => {
  const duplicate = await fixture(); duplicate.document.objects.push({ ...structuredClone(duplicate.document.objects[0]!), id: 'duplicate' });
  await assert.rejects(duplicate.run(), /Duplicate placed/);
  const pin = await fixture(); pin.native.elements[0]!.source.sha256 = 'c'.repeat(64);
  await assert.rejects(pin.run(), /pin changed/);
  const offset = await fixture(); offset.native.elements[0]!.frames[0]!.offset[0]++;
  await assert.rejects(offset.run(), /timing or offset/);
});
test('correct source identity cannot admit different decoded artwork even with a valid replacement PNG hash', async () => {
  const f = await fixture(); const frame = f.native.elements[0]!.frames[0]!;
  const bytes = encode({ width: 2, height: 1, channels: 4, data: new Uint8Array([0, 0, 0, 0, 254, 0, 0, 255]) });
  f.bank.files.set(frame.path, bytes); frame.sha256 = await hash(bytes);
  await assert.rejects(f.run(), /artwork differs/);
});

import { SceneryLayer } from './scenery-layer.ts';
function attachLayer(f: Awaited<ReturnType<typeof fixture>>) {
  const warnings: string[] = [];
  const layer = new SceneryLayer(() => {}, text => warnings.push(text));
  const internal = layer as any;
  const directory = (prefix = ''): any => ({
    getDirectoryHandle: async (name: string) => directory(prefix + name + '/'),
    getFileHandle: async (name: string) => ({ getFile: async () => new File([await f.bank.read(prefix + name)], name) }),
  });
  internal.document = f.document;
  internal.library = directory();
  internal.descriptors = new Map([['fire', f.bank.descriptor]]);
  internal.effects.set('placed/fire', { key: 'fixture' });
  internal.resources.set('fixture', Promise.resolve({}));
  return { layer, internal, warnings };
}
test('production layer admits the binding and hides it after a placed source identity changes', async () => {
  const f = await fixture(), { layer, internal, warnings } = attachLayer(f);
  assert.equal(await layer.bindNativeSourceClocks(f.native, f.level), true);
  layer.sourceClocks.sample([{ id: 'source0', source: f.native.elements[0]!.source, tick: 0, frame: 0, active: true, playing: false, epoch: 1, generation: 1, key: 'test' }]);
  const frames = [{ geometry: {}, texture: {} }, { geometry: {}, texture: {} }];
  const effect = { start: 0, descriptorSha256: 'b'.repeat(64), frames: { frames }, mesh: { name: 'effect', visible: true, material: {}, geometry: {} } };
  internal.effects.set('placed/fire', effect);
  layer.update(999999);
  assert.equal(effect.mesh.material.map, frames[0]!.texture);
  f.document.objects[0]!.transform.dx = 10;
  layer.update(999999);
  assert.equal(effect.mesh.visible, false);
  assert.match(warnings[0]!, /placement or descriptor changed/);
  layer.clearSourceClocks();
  assert.equal(layer.sourceClocks.frame('placed/fire', 'b'.repeat(64), 2), undefined);
});
test('clearing a mission during asynchronous descriptor admission cannot install a retired binding', async () => {
  const f = await fixture(), { layer, internal } = attachLayer(f);
  let release!: () => void;
  internal.descriptorLoad = new Promise<void>(resolve => { release = resolve; });
  const pending = layer.bindNativeSourceClocks(f.native, f.level);
  layer.clearSourceClocks(); release();
  assert.equal(await pending, false);
  assert.equal(layer.sourceClocks.frame('placed/fire', 'b'.repeat(64), 2), undefined);
});
test('an unrelated missing scenery preview does not prevent a verified source owner from binding', async () => {
  const f = await fixture(), { layer, internal } = attachLayer(f);
  const unrelated = Promise.reject(new Error('Unrelated optional preview missing'));
  void unrelated.catch(() => {});
  internal.resources.set('unrelated', unrelated);
  assert.equal(await layer.bindNativeSourceClocks(f.native, f.level), true);
});
test('retired missions cannot bind after a matching resource finishes loading', async () => {
  const f = await fixture(), { layer, internal } = attachLayer(f);
  let waiting = false, release!: () => void;
  internal.resources.set('fixture', { then(resolve: (value: unknown) => void) { waiting = true; release = () => resolve({}); } });
  const pending = layer.bindNativeSourceClocks(f.native, f.level);
  while (!waiting) await new Promise(resolve => setImmediate(resolve));
  layer.clearSourceClocks(); release();
  assert.equal(await pending, false);
  assert.equal(layer.sourceClocks.frame('placed/fire', 'b'.repeat(64), 2), undefined);
});
test('placement mismatch during resource admission rejects without leaving a partial binding', async () => {
  const f = await fixture(), { layer, internal } = attachLayer(f);
  let waiting = false, release!: () => void;
  internal.resources.set('fixture', { then(resolve: (value: unknown) => void) { waiting = true; release = () => resolve({}); } });
  const pending = layer.bindNativeSourceClocks(f.native, f.level);
  while (!waiting) await new Promise(resolve => setImmediate(resolve));
  f.document.objects[0]!.transform.dx = 12; release();
  await assert.rejects(pending, /placement changed during/);
  assert.equal(layer.sourceClocks.frame('placed/fire', 'b'.repeat(64), 2), undefined);
});
