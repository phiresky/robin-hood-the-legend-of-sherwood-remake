import { isDeepStrictEqual } from "node:util";
import { partMatrix, type Level3D } from "../../shared/src/level3d.ts";
import type { SoundSource } from "../../shared/src/level.ts";
import {
  validateAssetGameplay,
  type GameplayAssetDescriptor,
} from "../../shared/src/asset-gameplay.ts";
import { compileSoundSource } from "../../shared/src/compile-sound-source.ts";
import { gameToScene } from "../../shared/src/scene.ts";
import { applyAffineMatrix, sceneToGame } from "../../shared/src/geometry.ts";

/** Preserve explicit acoustic assets during one-time recovery and consume each
 * matching source record once. No source indices are stored in asset definitions. */
export function recoverAuthoredSounds(
  document: Level3D,
  descriptors: ReadonlyMap<string, GameplayAssetDescriptor>,
  sources: SoundSource[],
) {
  const used = new Set<number>();
  const results = [];
  for (const descriptor of descriptors.values()) {
    if (descriptor.asset_type !== "Sound region") continue;
    const gameplay = descriptor.gameplay;
    if (
      !gameplay ||
      gameplay.collision !== "none" ||
      gameplay.surfaces.length ||
      gameplay.doors.length ||
      gameplay.lifts?.length ||
      gameplay.interiors?.length ||
      !gameplay.sounds?.length ||
      !descriptor.parts.every((p) => p.scenery && p.gameplay_only) ||
      Object.keys(gameplay).some(
        (key) =>
          ![
            "version",
            "collision",
            "surfaces",
            "doors",
            "sounds",
            "lifts",
            "interiors",
            "draft",
          ].includes(key),
      )
    )
      throw new Error(`Sound-region asset has unsupported or missing gameplay: ${descriptor.id}`);
    validateAssetGameplay(gameplay, descriptor);
    const compiled = gameplay.sounds.map((sound) =>
      compileSoundSource(sound, (node, point) => {
        const frames = document.objects.filter((p) => p.node === `asset:${descriptor.id}:${node}`);
        if (frames.length !== 1)
          throw new Error(
            `Sound recovery needs exactly one placed instance of ${descriptor.id}:${node}`,
          );
        const matrix = partMatrix(document.camera, document, frames[0]!);
        return sceneToGame(
          document.camera,
          applyAffineMatrix(matrix, gameToScene(document.camera, ...point)),
        );
      }),
    );
    const sourceIndices = compiled.map((sound) => {
      const matches = sources.flatMap((source, index) =>
        !used.has(index) && isDeepStrictEqual(source, sound) ? [index] : [],
      );
      if (matches.length !== 1)
        throw new Error(
          `Authored sound in ${descriptor.id} must match exactly one unclaimed source (found ${matches.length})`,
        );
      const index = matches[0]!;
      used.add(index);
      return index;
    });
    results.push({ asset: descriptor.id, sounds: structuredClone(gameplay.sounds), sourceIndices });
  }
  return results;
}
