import type { Level3D, GameTransform } from "./level3d.ts";
/** Appearance authority is the existing placed roof control, not another patch cursor. */
export interface SourceCoverVisibility {
  native_element_id: string;
  physical_target_id: string;
  source: { kind: "map-patch"; index: number; sha256: string };
  visible_when: "applied";
  placement: {
    group_id: string;
    transform: GameTransform;
    asset_id: string;
    appearance_id: string;
    patch_id: string;
    model_sha256: string;
    descriptor_sha256: string;
  };
}
export function validateSourceCoverVisibility(
  rows: unknown,
): asserts rows is SourceCoverVisibility[] {
  if (!Array.isArray(rows)) throw new Error("Invalid source cover visibility list");
  const ids = new Set<string>();
  const hash = (v: unknown) => typeof v === "string" && /^[a-f0-9]{64}$/.test(v);
  for (const r of rows) {
    if (
      !r ||
      !r.source ||
      r.source.kind !== "map-patch" ||
      !Number.isSafeInteger(r.source.index) ||
      r.source.index < 0 ||
      !hash(r.source.sha256) ||
      r.visible_when !== "applied" ||
      !r.placement ||
      ![
        r.native_element_id,
        r.physical_target_id,
        r.placement.group_id,
        r.placement.asset_id,
        r.placement.appearance_id,
        r.placement.patch_id,
      ].every((v) => typeof v === "string" && v.length > 0) ||
      !r.placement.transform ||
      !["dx", "dy", "dz", "rot_deg"].every((k) => Number.isFinite(r.placement.transform[k])) ||
      !hash(r.placement.model_sha256) ||
      !hash(r.placement.descriptor_sha256) ||
      r.placement.patch_id !== `patch-${String(r.source.index).padStart(3, "0")}` ||
      ids.has(r.native_element_id) ||
      ids.has("physical:" + r.physical_target_id)
    )
      throw new Error("Invalid or duplicate source cover visibility");
    ids.add(r.native_element_id);
    ids.add("physical:" + r.physical_target_id);
  }
}
export function verifySourceCoverPlacements(
  rows: readonly Pick<SourceCoverVisibility, "placement">[],
  document: Level3D,
) {
  for (const r of rows) {
    const group = document.groups.find((g) => g.id === r.placement.group_id);
    const asset = document.assetSources?.find((a) => a.id === r.placement.asset_id);
    if (
      !group ||
      ["dx", "dy", "dz", "rot_deg"].some(
        (k) =>
          group.transform[k as keyof GameTransform] !==
          r.placement.transform[k as keyof GameTransform],
      ) ||
      group.patches?.[r.placement.asset_id]?.[r.placement.appearance_id] !== r.placement.patch_id ||
      !asset ||
      asset.model_sha256 !== r.placement.model_sha256 ||
      asset.descriptor_sha256 !== r.placement.descriptor_sha256 ||
      !document.objects.some((o) => o.group === group.id && o.node.split(":")[1] === asset.id)
    )
      throw new Error("Source cover placement differs from the displayed roof");
  }
}
