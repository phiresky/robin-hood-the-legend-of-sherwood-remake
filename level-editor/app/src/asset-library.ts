import type { ProjectionAssetEntry } from "@rle/shared";

// Reviewed map refinement workflows are recorded in blender/README.md and
// blender/{leicester,lincoln,nottingham,sherwood}/. Extend this list when another
// source level goes through refinement; reconstruction alone does not qualify.
export const REFINED_LEVELS = ["Derby", "Leicester", "Lincoln", "Nottingham", "Sherwood"] as const;
export const REFINED_LEVELS_FILTER = "refined-levels";
const refinedLevels = new Set<string>(REFINED_LEVELS.map((name) => name.toLowerCase()));

export const ASSET_DRAG_TYPE = "application/x-rle-asset";

export function isGameplayHelper(entry: ProjectionAssetEntry): boolean {
  const parts = entry.editor?.parts;
  return !!parts?.length && parts.every((part) => part.gameplay_only === true);
}

/** Published metadata takes precedence; older catalogs remain browsable. */
export function assetType(entry: ProjectionAssetEntry): string {
  if (entry.editor_usage === "map-background") return "Background";
  if (entry.asset_type) return entry.asset_type;
  const name = entry.name.toLowerCase();
  if (/tree|bush|hedge|foliage|shrub/.test(name)) return "Vegetation";
  if (/barrel|crate|timber$|well|trough|bucket|cart|tub|chopping block|hay mound/.test(name))
    return "Prop";
  if (/bridge|\bgate\b|portcullis/.test(name)) return "Bridge & gate";
  if (/house|cottage|hall|keep|tower|turret|church|shed|shelter|lean-to|wing/.test(name))
    return "Building";
  if (/wall|fence|palisade|curtain|archway/.test(name)) return "Wall";
  if (/terrain|terrace|ground|cliff|rock|stone|bank|field boundary/.test(name)) return "Terrain";
  return "Building";
}

export function assetTags(entry: ProjectionAssetEntry): string[] {
  return [
    ...new Set([
      assetType(entry),
      entry.source_map,
      ...(entry.tags ?? []),
      ...(entry.state_variant ? [entry.state_variant] : []),
    ]),
  ];
}

export function filterAssets(
  entries: ProjectionAssetEntry[],
  search: string,
  type: string,
  source: string,
  showHelpers = false,
) {
  const terms = search.trim().toLowerCase().split(/\s+/).filter(Boolean);
  return entries.filter(
    (entry) =>
      (showHelpers || !isGameplayHelper(entry)) &&
      (!type || assetType(entry) === type) &&
      (!source ||
        (source === REFINED_LEVELS_FILTER
          ? refinedLevels.has(entry.source_map.toLowerCase())
          : entry.source_map === source)) &&
      terms.every((term) =>
        [entry.name, entry.id, ...assetTags(entry)].join(" ").toLowerCase().includes(term),
      ),
  );
}
