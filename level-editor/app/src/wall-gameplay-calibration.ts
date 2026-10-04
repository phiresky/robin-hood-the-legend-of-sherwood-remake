import * as THREE from "three";
import type { Level3D, ProjectionAssetDescriptor } from "@rle/shared";
import type { GameplayAssetDescriptor } from "../../shared/src/asset-gameplay.ts";
import { validateAssetGameplay } from "../../shared/src/asset-gameplay.ts";
import { matchesWallSource, wallSourceSettings } from "../../shared/src/wall-section-profile.ts";
import { wallSectionProfile } from "./spline-geometry.ts";

/** Derive export measurements from the same pinned asset meshes used by wall rendering. */
function* wallGameplayCalibrationSteps(
  document: Level3D,
  descriptors: ReadonlyMap<string, ProjectionAssetDescriptor>,
  sources: ReadonlyMap<string, THREE.Object3D>,
) {
  const assets = new Map(descriptors),
    warnings: string[] = [];
  const paths = (document.splines ?? []).filter((path) => path.kind === "wall");
  const ids = new Set(
    paths.flatMap((path) => [path.asset, path.cornerAsset]).filter((id) => id !== undefined),
  );
  for (const id of ids) {
    yield;
    const descriptor: GameplayAssetDescriptor | undefined = descriptors.get(id);
    if (
      !descriptor?.gameplay ||
      descriptor.states ||
      descriptor.gameplay.movementTransitions?.length
    )
      continue;
    try {
      const source = new THREE.Group();
      const nodes = new Map<string, THREE.Object3D>();
      for (const [key, node] of sources)
        if (key.startsWith(`asset:${id}:`)) {
          const clone = node.clone(true);
          source.add(clone);
          nodes.set(key.slice(`asset:${id}:`.length), clone);
        }
      if (!source.children.length) throw new Error("asset mesh is not loaded");
      source.updateWorldMatrix(true, true);
      const bounds = new THREE.Box3().setFromObject(source);
      const frames: Record<string, number[]> = {};
      for (const part of descriptor.parts) {
        const node = nodes.get(part.node);
        if (!node) throw new Error(`missing asset frame ${part.node}`);
        frames[part.node] = node.matrixWorld.toArray();
      }
      const gameplay = structuredClone(descriptor.gameplay);
      gameplay.spline = {
        modelSha256: document.assetSources?.find((s) => s.id === id)?.model_sha256,
        bounds: { min: bounds.min.toArray(), max: bounds.max.toArray() },
        frames,
        deformations: [],
      };
      for (const path of paths.filter((path) => path.asset === id)) {
        if (gameplay.spline.deformations!.some((c) => matchesWallSource(c, path))) continue;
        yield;
        try {
          source.rotation.z = (-(path.sourceAngle ?? 0) * Math.PI) / 180;
          source.updateWorldMatrix(true, true);
          const rotatedBounds = new THREE.Box3().setFromObject(source);
          gameplay.spline.deformations!.push({
            ...wallSourceSettings(path),
            bounds: { min: rotatedBounds.min.toArray(), max: rotatedBounds.max.toArray() },
            ...(!path.sourceStraight
              ? { profile: wallSectionProfile(source, rotatedBounds, path) }
              : {}),
          });
        } catch (error) {
          warnings.push(
            `Wall spline ${path.id}: source section calibration unavailable: ${String(error)}`,
          );
        }
      }
      validateAssetGameplay(gameplay, descriptor);
      const calibrated: GameplayAssetDescriptor = { ...descriptor, gameplay };
      assets.set(id, calibrated);
    } catch (error) {
      warnings.push(`Wall asset ${id}: mesh calibration unavailable: ${String(error)}`);
    }
  }
  return { assets, warnings };
}

export function prepareWallGameplayAssets(
  ...args: Parameters<typeof wallGameplayCalibrationSteps>
) {
  const steps = wallGameplayCalibrationSteps(...args);
  let step = steps.next();
  while (!step.done) step = steps.next();
  return step.value;
}

export async function prepareWallGameplayAssetsAsync(
  document: Level3D,
  descriptors: ReadonlyMap<string, ProjectionAssetDescriptor>,
  sources: ReadonlyMap<string, THREE.Object3D>,
  yieldFrame: () => Promise<void>,
) {
  const steps = wallGameplayCalibrationSteps(document, descriptors, sources);
  let step = steps.next();
  while (!step.done) {
    await yieldFrame();
    step = steps.next();
  }
  return step.value;
}
