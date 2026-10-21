import * as THREE from 'three';
import { decode } from 'fast-png';
import { gameToScene, partMatrix, sceneToGame, type Level3D } from '@rle/shared';
import type { GameplayAssetDescriptor } from '../../shared/src/asset-gameplay.ts';
import type { ProtoLevel } from '../../shared/src/level.ts';
import type { NativeStatePresentationContract } from '../../shared/src/native-state-presentation.ts';
import { compileSceneryAnimation } from '../../shared/src/compile-scenery-animation.ts';
import { missionStateDataHash } from './mission-state-layer.ts';
import { decodeSpritePixels } from './entity-projection.ts';
import { decodeNativeResource } from './native-state-presentation.ts';
import { loadSceneryBank } from './scenery-thumbnail.ts';
import type { ScenerySourceClockBinding } from './scenery-source-clocks.ts';

function stable(value: any): any {
  return Array.isArray(value) ? value.map(stable) : value && typeof value === 'object'
    ? Object.fromEntries(Object.keys(value).sort().map(key => [key, stable(value[key])])) : value;
}
const key = (value: unknown) => JSON.stringify(stable(value));

/** Exact placed source records; names are never used to infer correspondence. */
export function placedScenerySourceCandidates(document: Level3D, descriptors: ReadonlyMap<string, GameplayAssetDescriptor>) {
  const hidden = new Set(document.groups.filter(group => group.hidden).map(group => group.id));
  const candidates = [];
  for (const part of document.objects) {
    const match = /^asset:([^:]+):(.+)$/.exec(part.node);
    if (!match || (part.group && hidden.has(part.group))) continue;
    const descriptor = descriptors.get(match[1]!);
    if (!descriptor) throw new Error('Missing scenery source descriptor: ' + match[1]);
    if (part.hidden && !document.objects.some(other => other.group === part.group && part.group && !other.hidden && other.node.startsWith(`asset:${match[1]}:`))) continue;
    for (const animation of descriptor.gameplay?.animations ?? []) {
      if (animation.node !== match[2] || !animation.active) continue;
      const pin = document.assetSources?.find(ref => ref.id === descriptor.id);
      if (!pin?.descriptor_sha256) throw new Error('Unpinned scenery source descriptor');
      const matrix = new THREE.Matrix4().fromArray(partMatrix(document.camera, document, part));
      const compiled = compileSceneryAnimation(animation, (_node, point) => {
        const placed = new THREE.Vector3(...gameToScene(document.camera, ...point)).applyMatrix4(matrix);
        return sceneToGame(document.camera, [placed.x, placed.y, placed.z]);
      });
      candidates.push({ effectId: `${part.id}/${animation.id}`, descriptor, animation, descriptorSha256: pin.descriptor_sha256,
        compiled, identity: key([pin.descriptor_sha256, compiled]) });
    }
  }
  return candidates;
}

/** Match complete placed records and decoded artwork before admitting a source clock. */
export async function resolveScenerySourceCorrespondence(
  document: Level3D,
  descriptors: ReadonlyMap<string, GameplayAssetDescriptor>,
  native: NativeStatePresentationContract,
  level: ProtoLevel,
  read: (path: string) => Promise<Uint8Array>,
) {
  const bindings: ScenerySourceClockBinding[] = [];
  const identities = new Map<string, string>();
  const owned = new Set<number>();
  for (const candidate of placedScenerySourceCandidates(document, descriptors)) {
    const matching = native.elements.filter(element => element.source.kind === 'map-animation' &&
      element.loop && key(level.animations?.[element.source.index]) === key(candidate.compiled));
    if (!matching.length) continue; // A separately placed palette instance is not the mission owner.
    if (matching.length !== 1) throw new Error('Ambiguous placed scenery source animation');
    const element = matching[0]!;
    if (owned.has(element.source.index)) throw new Error('Duplicate placed scenery source owner');
    if (await missionStateDataHash(level.animations![element.source.index]) !== element.source.sha256)
      throw new Error('Scenery source animation pin changed');
    const bank = await loadSceneryBank(candidate.descriptor, candidate.animation, read);
    if (bank.profile.rows[0]!.frames.length !== element.frames.length)
      throw new Error('Scenery source frame count differs');
    for (const [index, frame] of bank.profile.rows[0]!.frames.entries()) {
      const expected = element.frames[index]!;
      if (frame.delay !== expected.delay || frame.offsetX !== expected.offset[0] || frame.offsetY !== expected.offset[1])
        throw new Error('Scenery source frame timing or offset differs');
      const image = decode(bank.files[frame.path]!, { checkCrc: true });
      if (image.depth !== 8 || ![3, 4].includes(image.channels) || image.width !== expected.width || image.height !== expected.height)
        throw new Error('Scenery source frame dimensions differ');
      const pixels = new Uint8ClampedArray(image.width * image.height * 4);
      for (let p = 0; p < image.width * image.height; p++) {
        for (let c = 0; c < 3; c++) pixels[p * 4 + c] = image.data[p * image.channels + c]!;
        pixels[p * 4 + 3] = image.channels === 4 ? image.data[p * 4 + 3]! : 255;
      }
      decodeSpritePixels(pixels, bank.legacy);
      const source = await decodeNativeResource(expected, resource => read(resource.path));
      if (pixels.some((value, i) => value !== source.data[i])) throw new Error('Scenery source frame artwork differs');
    }
    owned.add(element.source.index);
    identities.set(candidate.effectId, candidate.identity);
    bindings.push({ effectId: candidate.effectId, descriptorSha256: candidate.descriptorSha256,
      sourceIndex: element.source.index, sourceSha256: element.source.sha256 });
  }
  return { bindings, identities };
}
