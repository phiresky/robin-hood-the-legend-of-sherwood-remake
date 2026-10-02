import fs from "node:fs/promises";
import path from "node:path";
import { createHash } from "node:crypto";
import { pathToFileURL } from "node:url";
import {
  parseProjectionAssetIndex,
  parseProjectionAssetDescriptor,
} from "../../shared/src/validation.ts";
import {
  validateAssetGameplay,
  type GameplayAssetDescriptor,
} from "../../shared/src/asset-gameplay.ts";
import type { Vec3 } from "../../shared/src/scene.ts";
import { configureAssetGameplay, type GameplayEdit } from "./configure-surface-jumps.ts";

export interface TerrainAttachmentRule {
  kind: "mask" | "interior-door" | "passage-outside" | "passage-inside" | "projection-receiver";
  id: string;
  node: string;
  /** Reviewed asset-local anchor; reject changed ownership or geometry on reapplication. */
  anchor: Vec3;
  below: number;
  above: number;
}

/** Author finite vertical receivers from explicitly selected asset-local features. */
export function authorTerrainAttachments(
  asset: GameplayAssetDescriptor,
  rules: TerrainAttachmentRule[],
) {
  validateAssetGameplay(asset.gameplay, asset);
  const gameplay = structuredClone(asset.gameplay);
  if (!Array.isArray(rules) || !rules.length)
    throw new Error("Terrain attachments require explicit feature rules");
  const seen = new Set<string>();
  for (const rule of rules) {
    const key = `${rule.kind}/${rule.id}`;
    if (seen.has(key)) throw new Error(`Duplicate terrain attachment ${key}`);
    seen.add(key);
    if (
      !Array.isArray(rule.anchor) ||
      rule.anchor.length !== 3 ||
      !rule.anchor.every(Number.isFinite) ||
      ![rule.below, rule.above].every((v) => Number.isFinite(v) && v >= 0) ||
      rule.below + rule.above <= 0
    )
      throw new Error(`Invalid terrain attachment bounds ${key}`);
    const mask =
      rule.kind === "mask" ? (gameplay.masks?.filter((m) => m.id === rule.id) ?? []) : [];
    const doors =
      rule.kind === "interior-door"
        ? (gameplay.interiors?.flatMap((r) => r.doors).filter((d) => d.id === rule.id) ?? [])
        : rule.kind === "passage-outside" || rule.kind === "passage-inside"
          ? gameplay.doors.filter((d) => d.id === rule.id && d.type === 0)
          : [];
    const receivers =
      rule.kind === "projection-receiver"
        ? (gameplay.projectionReceivers?.filter((r) => r.id === rule.id) ?? [])
        : [];
    if (mask.length + doors.length + receivers.length !== 1)
      throw new Error(`Terrain attachment needs one local feature ${key}`);
    if ((mask[0] ?? doors[0] ?? receivers[0])!.node !== rule.node)
      throw new Error(`Terrain attachment owner changed ${key}`);
    const inside = rule.kind === "passage-inside";
    const anchor =
      mask[0]?.anchor ?? receivers[0]?.anchor ?? (inside ? doors[0]!.inside : doors[0]!.outside);
    if (anchor.some((v, i) => Math.abs(v - rule.anchor[i]!) > 1e-4))
      throw new Error(`Terrain attachment anchor changed ${key}`);
    if (inside ? doors[0]?.insideAnchor : doors[0]?.outsideAnchor)
      throw new Error(`Terrain attachment conflicts with door anchor ${key}`);
    const [x, y, z] = anchor;
    const segment: [Vec3, Vec3] = [
      [x, y, z - rule.below],
      [x, y, z + rule.above],
    ];
    const previous =
      mask[0]?.receiverSegment ??
      receivers[0]?.receiverSegment ??
      (inside ? doors[0]?.insideReceiverSegment : doors[0]?.outsideReceiverSegment);
    if (previous && JSON.stringify(previous) !== JSON.stringify(segment))
      throw new Error(`Terrain attachment already has different bounds ${key}`);
    if (mask[0]) mask[0].receiverSegment = segment;
    else if (receivers[0]) receivers[0].receiverSegment = segment;
    else if (inside) doors[0]!.insideReceiverSegment = segment;
    else doors[0]!.outsideReceiverSegment = segment;
  }
  validateAssetGameplay(gameplay, asset);
  return gameplay;
}

export async function stageTerrainAttachments(
  library: string,
  recipes: { asset: string; rules: TerrainAttachmentRule[] }[],
  output: string,
  apply = false,
) {
  const index = parseProjectionAssetIndex(
    JSON.parse(await fs.readFile(path.join(library, "3d-assets/index.json"), "utf8")),
  );
  const edits: GameplayEdit[] = [];
  for (const recipe of recipes) {
    const entry = index.find((e) => e.id === recipe.asset);
    if (!entry) throw new Error(`Unknown terrain attachment asset ${recipe.asset}`);
    const bytes = await fs.readFile(path.join(library, "3d-assets", entry.descriptor));
    const descriptor = parseProjectionAssetDescriptor(JSON.parse(bytes.toString("utf8")));
    edits.push({
      asset: recipe.asset,
      descriptorSha256: createHash("sha256").update(bytes).digest("hex"),
      gameplay: authorTerrainAttachments(descriptor, recipe.rules),
    });
  }
  return configureAssetGameplay(library, edits, output, apply);
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const [library, recipes, output, mode] = process.argv.slice(2);
  if (!library || !recipes || !output || (mode !== undefined && mode !== "--apply"))
    throw new Error(
      "Usage: author-terrain-attachments.ts library recipes.json new-backup-directory [--apply]",
    );
  console.log(
    JSON.stringify(
      await stageTerrainAttachments(
        library,
        JSON.parse(await fs.readFile(recipes, "utf8")),
        output,
        mode === "--apply",
      ),
    ),
  );
}
