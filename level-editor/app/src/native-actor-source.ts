import type { Mask, MaskReference } from "../../shared/src/level.ts";
import type { NativeStatePresentationContract } from "../../shared/src/native-state-presentation.ts";
import { missionStateDataHash, type MissionStateSource } from "./mission-state-layer.ts";
import { actorMaskApplies, type ActorMaskSnapshot } from "./native-actor-pixels.ts";
import type { ActorCompositionAuthority } from "./native-actor-composition.ts";

export function initialActorMaskMembership(
  masks: readonly Mask[],
  patches: readonly { old_masks: MaskReference[]; new_masks: MaskReference[] }[],
) {
  const layers = new Map<number, number[]>(),
    active = masks.map(() => true);
  masks.forEach((mask, index) => {
    if (!layers.has(mask.layer)) layers.set(mask.layer, []);
    layers.get(mask.layer)!.push(index);
  });
  for (const patch of patches)
    for (const [references, value] of [
      [patch.old_masks, true],
      [patch.new_masks, false],
    ] as const)
      for (const reference of references) {
        const index = layers.get(reference.layer)?.[reference.index];
        if (index === undefined) throw new Error("Unresolved layer-local mask reference");
        active[index] = value;
      }
  return active;
}
export function queryActorMasks(
  masks: readonly Mask[],
  active: readonly boolean[],
  actor: { layer: number; mapPosition: [number, number] },
  box: [number, number, number, number],
  gridSize: [number, number],
): ActorMaskSnapshot[] {
  if (
    masks.length !== active.length ||
    active.some((value) => typeof value !== "boolean") ||
    !box.every(Number.isFinite) ||
    box[2] < box[0] ||
    box[3] < box[1] ||
    !gridSize.every((value) => Number.isInteger(value) && value > 0)
  )
    throw new Error("Invalid current mask query");
  const cells = (rect: readonly number[]) =>
    rect.map((value, index) =>
      Math.max(0, Math.min(gridSize[index % 2]! - 1, Math.trunc(value) >> 6)),
    );
  const bounds = masks.map((mask) => {
    const [x, y] = mask.box_top_left,
      [w, h] = mask.box_size;
    return [x, y, x + w, y + h];
  });
  const grids = bounds.map(cells),
    [x0, y0, x1, y1] = cells(box),
    seen = new Set<number>(),
    ordered: number[] = [];
  for (let y = y0!; y <= y1!; y++)
    for (let x = x0!; x <= x1!; x++)
      for (let i = 0; i < masks.length; i++) {
        const mask = masks[i]!,
          [a, b, c, d] = grids[i]!;
        if (
          mask.layer === actor.layer &&
          active[i] &&
          mask.mask_type & 1 &&
          x >= a! &&
          x <= c! &&
          y >= b! &&
          y <= d! &&
          !seen.has(i)
        ) {
          seen.add(i);
          ordered.push(i);
        }
      }
  return ordered
    .filter((index) => {
      const b = bounds[index]!;
      return (
        b[0]! <= box[2] &&
        b[2]! >= box[0] &&
        b[1]! <= box[3] &&
        b[3]! >= box[1] &&
        actorMaskApplies(masks[index]!, active[index]!, actor.layer, actor.mapPosition)
      );
    })
    .map((index) => ({ id: `mask:${index}`, mask: masks[index]!, active: active[index]! }));
}

/** Verified stream baseline for the bounded Crossroads 2 skirmish preview, before scripts. */
export async function bindInitialActorSource(
  source: MissionStateSource,
  contract: Pick<
    NativeStatePresentationContract,
    "mission" | "mission_data_sha256" | "level_data_sha256" | "background"
  >,
  epoch: number,
) {
  const missionHash = await missionStateDataHash(source.data),
    levelHash = await missionStateDataHash(source.level);
  if (
    source.name !== "S03_FoB_MP" ||
    source.camera.elevation_deg !== 35 ||
    contract.mission !== source.name ||
    missionHash !== contract.mission_data_sha256 ||
    levelHash !== contract.level_data_sha256 ||
    missionHash !== "edcd221c00c65203c0dbe397fd0b3f92b8f28ef87b0b09840386aef3b07555ea" ||
    levelHash !== "466e1e1e1501be6af09e793c9fb2d902e0b136bc9ea8cb6ff03f48645d34669e"
  )
    throw new Error("Actor source construction audit does not cover this source revision");
  const data = structuredClone(source.data),
    level = structuredClone(source.level);
  const array = (value: unknown, label: string): Record<string, unknown>[] => {
    if (
      !Array.isArray(value) ||
      value.some((row) => !row || typeof row !== "object" || Array.isArray(row))
    )
      throw new Error(`Invalid ${label}`);
    return value;
  };
  const same = (value: unknown, expected: string[]) =>
    JSON.stringify(value) === JSON.stringify(expected);
  if (
    !same((level as typeof level & { element_chunk_order?: unknown }).element_chunk_order, [
      "Animation",
      "Patch",
    ]) ||
    !same(data.element_chunk_order, ["Patch", "Element", "Bonus", "Scroll", "Mobile"]) ||
    !same(data.element_group_order, [
      "Animal",
      "BeamMe",
      "Civilian",
      "PcToRescue",
      "Soldier",
      "Target",
    ])
  )
    throw new Error("Unsupported actor construction stream order");
  const creationRanks = new Map<string, number>();
  let rank = 0;
  const add = (family: string, count: number) => {
    for (let index = 0; index < count; index++) creationRanks.set(`${family}:${index}`, rank++);
  };
  add("map-animation", level.animations.length);
  add("map-patch", level.patches.length);
  const missionPatches = array(data.mission_patches, "mission patches");
  add("mission-patch", missionPatches.length);
  const records = new Map<string, Record<string, unknown>>();
  for (const family of ["civilians", "pcs_to_rescue", "soldiers", "targets"]) {
    const rows = array(data[family], family),
      key = family === "targets" ? "mission-target" : family;
    add(key, rows.length);
    rows.forEach((row, index) => records.set(`${key}:${index}`, row));
  }
  const backgroundEffects = new Set<string>();
  level.animations.forEach((animation, index) => {
    if (animation.sprite.elevation === 0) backgroundEffects.add(`map-animation:${index}`);
  });
  const membership = initialActorMaskMembership(level.masks, [
    ...level.patches,
    ...missionPatches.map((patch) => ({
      old_masks: patch.old_masks as MaskReference[],
      new_masks: patch.new_masks as MaskReference[],
    })),
  ]);
  const width = contract.background.width,
    height = contract.background.height;
  if (width % 64 || height % 64) throw new Error("Unsupported source grid dimensions");
  // The mask grid retains four rows below the visible image.
  const gridSize: [number, number] = [width / 64, height / 64 + 4];
  const authority: ActorCompositionAuthority = {
    epoch,
    mission: source.name,
    creationRanks,
    backgroundEffects,
  };
  return {
    authority,
    records,
    masks: level.masks,
    membership,
    gridSize,
    missionHash,
    levelHash,
    scope: "explicit-editor-preview-from-pre-script-baseline" as const,
  };
}
