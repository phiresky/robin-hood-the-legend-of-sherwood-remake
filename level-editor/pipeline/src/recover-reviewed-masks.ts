import type { Level3D } from "../../shared/src/level3d.ts";
import { partMatrix } from "../../shared/src/level3d.ts";
import type { ProtoLevel, Point } from "../../shared/src/level.ts";
import { gameToScene, type Vec3 } from "../../shared/src/scene.ts";
import { applyAffineMatrix, gltfToScene, sceneToGame } from "../../shared/src/geometry.ts";
import { heightPlane, planeHeight } from "../../shared/src/gameplay-plane.ts";
import {
  reviewedMaskStateBindings,
  type RecoveredMaskTransition,
} from "./reviewed-mask-state-bindings.ts";
import { loadSceneModel } from "./scene-assets.ts";
import { maskRecoveryMesh, maskRecoveryTextures } from "./mask-recovery-mesh.ts";
import { recoverOcclusionMask } from "./recover-occlusion-mask.ts";
import { distanceToPolygon, recoverEndpointElevation } from "./recovery-elevation.ts";

export interface ReviewedMaskRecipe {
  asset: string;
  model_sha256: string;
  entries: {
    source: number;
    id: string;
    node: string;
    anchor: Vec3;
    receiverSegment?: [Vec3, Vec3];
    characterHeights?: number[];
    projectileHeights?: number[];
  }[];
}

/** One-time migration of reviewed masks, requiring complete local state ownership. */
export async function recoverReviewedMasks(
  library: string,
  document: Level3D,
  proto: Pick<ProtoLevel, "masks" | "sight_obstacles" | "motion_data"> & {
    patches: Pick<ProtoLevel["patches"][number], "old_masks" | "new_masks">[];
  },
  recipes: ReviewedMaskRecipe[],
  transitions: readonly RecoveredMaskTransition[] = [],
) {
  const bindings = reviewedMaskStateBindings(proto.masks, proto.patches, recipes, transitions);
  const seen = new Set<number>();
  const recovered = [];
  for (const recipe of recipes) {
    const reference = document.assetSources?.find((a) => a.id === recipe.asset);
    if (!reference || reference.model_sha256 !== recipe.model_sha256)
      throw new Error(`Reviewed mask model changed: ${recipe.asset}`);
    const prefix = `asset:${recipe.asset}:`;
    const parts = document.objects.filter((p) => p.node.startsWith(prefix));
    const frames = new Set<string>();
    for (const part of parts) {
      if (frames.has(part.node))
        throw new Error(`Reviewed mask recovery needs one placement of ${recipe.asset}`);
      frames.add(part.node);
    }
    const model = await loadSceneModel(library, {
      ...reference,
      role: "objects",
      resources: reference.resources ?? [],
    });
    const textures = await maskRecoveryTextures(model);
    const bounds = recipe.entries.map((entry) => {
      const mask = proto.masks[entry.source];
      if (!mask) throw new Error(`Missing reviewed mask: ${entry.source}`);
      const [left, top] = mask.box_top_left;
      return { left, top, right: left + mask.box_size[0], bottom: top + mask.box_size[1] };
    });
    const surfaces = parts.flatMap((part) =>
      maskRecoveryMesh(
        model,
        part.node.slice(prefix.length),
        (point) =>
          sceneToGame(
            document.camera,
            applyAffineMatrix(partMatrix(document.camera, document, part), gltfToScene(point)),
          ),
        textures,
        bounds,
      ),
    );
    for (const entry of recipe.entries) {
      if (!Number.isInteger(entry.source) || entry.source < 0 || seen.has(entry.source))
        throw new Error("Invalid or duplicate reviewed mask index");
      seen.add(entry.source);
      const mask = proto.masks[entry.source];
      const part = parts.find((p) => p.node === prefix + entry.node);
      if (!mask || !part) throw new Error(`Missing reviewed mask or frame: ${entry.source}`);
      const projected: Point = [entry.anchor[0], entry.anchor[1] - entry.anchor[2]];
      if (
        !proto.motion_data.layers[mask.layer]?.some(
          (area) => distanceToPolygon(projected, area.polygon.points) === 0,
        )
      )
        throw new Error(`Mask ${entry.source} anchor is outside its source receiving layer`);
      const candidates = proto.sight_obstacles
        .filter((o) => Array.isArray(o.projection_area) && o.projection_area[1] === mask.layer)
        .map((o) => {
          const points = o.points.map((p): Vec3 => [p.x, p.y - p.z_top, p.z_top]);
          return {
            distance: distanceToPolygon(
              projected,
              points.map(([x, y]): Point => [x, y]),
            ),
            height: planeHeight(heightPlane(points.slice(0, 3)), projected),
            maximumHeight: Math.max(...o.points.map((p) => Math.max(p.z_top, p.z_bottom))),
          };
        });
      const height = recoverEndpointElevation(candidates, mask.layer === 0);
      if (Math.abs(height - entry.anchor[2]) > 0.001)
        throw new Error(`Mask ${entry.source} anchor has incorrect receiving elevation`);
      const matrix = partMatrix(document.camera, document, part);
      const localize = (point: Vec3): Vec3 => {
        const offset = gameToScene(document.camera, ...point).map((v, i) => v - matrix[12 + i]!);
        return sceneToGame(
          document.camera,
          [0, 1, 2].map(
            (i) =>
              matrix[i * 4]! * offset[0]! +
              matrix[i * 4 + 1]! * offset[1]! +
              matrix[i * 4 + 2]! * offset[2]!,
          ) as Vec3,
        );
      };
      const obstacles = new Map(
        mask.obstacle_indices.map((index) => {
          const owners = parts.filter((p) => p.source.obstacle === index);
          if (owners.length !== 1) throw new Error(`Mask obstacle ${index} needs one local owner`);
          return [index, owners[0]!.node.slice(prefix.length)] as const;
        }),
      );
      recovered.push({
        asset: recipe.asset,
        source: entry.source,
        ...(bindings.has(entry.source) ? { state: bindings.get(entry.source)! } : {}),
        definition: recoverOcclusionMask(mask, { ...entry, surfaces, obstacles, localize }),
      });
    }
  }
  return recovered;
}
