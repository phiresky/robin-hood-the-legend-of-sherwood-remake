import { gameplayFrameModel } from "./gameplay-frame-model.ts";
import type { Vec3 } from "../../shared/src/scene.ts";
import type { Level3DObject } from "../../shared/src/level3d.ts";
import {
  validateAssetGameplay,
  type AssetSceneryAnimation,
  type GameplayAssetDescriptor,
} from "../../shared/src/asset-gameplay.ts";

/** Author independent sprite effects in a reusable local frame, without baked artwork. */
export async function authorSceneryAnimationAsset(
  animations: Omit<AssetSceneryAnimation, "node">[],
  options: {
    id: string;
    name: string;
    map: string;
    origin: Vec3;
    resources?: GameplayAssetDescriptor["resources"];
  },
) {
  if (!/^[a-z0-9][a-z0-9-]*$/.test(options.id) || !options.name.trim())
    throw new Error("Scenery asset needs a stable ID and name");
  if (options.origin.length !== 3 || !options.origin.every(Number.isFinite))
    throw new Error("Scenery asset origin must be finite");
  if (!animations.length) throw new Error("Scenery asset needs at least one animation");
  const node = "scenery-animation";
  const descriptor: GameplayAssetDescriptor = {
    version: 1,
    kind: "projection-mapped-asset",
    id: options.id,
    name: options.name,
    source_map: options.map,
    asset_type: "Animated scenery",
    tags: ["animation", "gameplay"],
    model: "model.glb",
    model_scene: "default",
    resources: structuredClone(options.resources ?? []),
    parts: [{ node, name: options.name, scenery: true, gameplay_only: true }],
    gameplay: {
      version: 1,
      collision: "none",
      surfaces: [],
      doors: [],
      animations: animations.map((animation) => ({ ...structuredClone(animation), node })),
    },
  };
  validateAssetGameplay(descriptor.gameplay, descriptor);
  const placement: Level3DObject = {
    id: options.id,
    name: options.name,
    kind: "scenery",
    node: `asset:${options.id}:${node}`,
    source: { map: options.map },
    transform: { dx: options.origin[0], dy: options.origin[1], dz: options.origin[2], rot_deg: 0 },
  };
  return { descriptor, model: await gameplayFrameModel(options.id, node), placement };
}
