import type { Level3D } from "@rle/shared";
import type { GameplayAssetDescriptor } from "../../shared/src/asset-gameplay.ts";
import { compileAssetGameplay } from "../../shared/src/compile-asset-gameplay.ts";

export function staticGameplaySnapshot(document: Level3D): Level3D {
  const scene = structuredClone(document);
  delete scene.splines;
  for (const group of scene.groups) {
    delete group.states;
    delete group.patches;
  }
  for (const part of scene.objects) delete part.patches;
  return scene;
}

/** Authoring diagnostics only. A static probe must never authorize a full export. */
export function diagnoseGameplayCandidates(
  document: Level3D,
  candidates: ReadonlyMap<string, GameplayAssetDescriptor>,
  omissions: { omittedMovementTransitions?: number } = {},
) {
  const bounds =
    document.exportBounds ??
    (document.size
      ? ([0, 0, document.size[0], document.size[1]] as [number, number, number, number])
      : undefined);
  const probe = (scene: Level3D): { ready: boolean; error?: string; warnings?: string[] } => {
    try {
      if (!bounds) throw new Error("Map has no export bounds or size");
      const geometry = compileAssetGameplay(scene, candidates, bounds);
      return { ready: true, ...(geometry.warnings ? { warnings: geometry.warnings } : {}) };
    } catch (error) {
      return { ready: false, error: String(error) };
    }
  };
  const omittedMovementTransitions = omissions.omittedMovementTransitions ?? 0;
  const compilation = omittedMovementTransitions
    ? {
        ready: false,
        error: `Missing asset-local movement transition definitions (${omittedMovementTransitions})`,
      }
    : probe(document);
  // Preserve the currently visible placement geometry. Removing unsupported
  // behaviours is confined to this disposable diagnostic snapshot.
  const staticScene = staticGameplaySnapshot(document);
  return {
    compilation,
    staticGeometry: {
      scope: "current-visible-geometry-only",
      omittedMovementTransitions,
      ...probe(staticScene),
    },
  };
}
