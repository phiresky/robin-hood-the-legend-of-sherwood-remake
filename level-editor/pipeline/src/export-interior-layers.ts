// Export separate covered and revealed projection sources, retaining patch state metadata.
import fs from "node:fs/promises";
import path from "node:path";
import sharp, { type OverlayOptions } from "sharp";
import { findMapPng, loadProtoLevel } from "./asset-writer.ts";
import { fxTopLeft, loadFxSprite, loadKeyedFxPng } from "./fx.ts";
import { exportMissionPatchLayers } from "./mission-patch-layers.ts";

const [map, outputArg] = process.argv.slice(2);
if (!map || !outputArg) throw new Error("usage: node src/export-interior-layers.ts <Map> <fresh-output-dir>");
const output = path.resolve(outputArg);
const level = await loadProtoLevel(map);
const rawPath = await findMapPng("Day", map);
if (!rawPath) throw new Error(`missing Day map ${map}`);
const image = sharp(rawPath);
const { width, height } = await image.metadata();
if (!width || !height) throw new Error("missing map image dimensions");
await fs.mkdir(output, { recursive: false });
await image.png().toFile(path.join(output, "revealed.png"));
const overlays: OverlayOptions[] = [];
const patches = [];
for (let i = 0; i < level.patches.length; i++) {
  const patch = level.patches[i]!;
  const sprite = patch.element_fx.sprite;
  const id = `patch-${String(i).padStart(3, "0")}`;
  let graphic: { image: string; alpha: string; bbox: number[] } | null = null;
  const candidates: { source_node: string; alpha_overlap_pixels: number }[] = [];
  if (sprite.frame_profile_name !== "pixel_vert") {
    const fx = await loadFxSprite("Day", sprite.frame_profile_name, sprite.profile_name);
    if (!fx) throw new Error(`missing required patch sprite ${sprite.profile_name}`);
    const png = await loadKeyedFxPng(fx.framePath);
    const [left, top] = fxTopLeft(fx, sprite.position_x, sprite.position_y, sprite.elevation);
    const { data, info } = await sharp(png).ensureAlpha().raw().toBuffer({ resolveWithObject: true });
    await fs.writeFile(path.join(output, `${id}.png`), png);
    await sharp(png).ensureAlpha().extractChannel(3).png().toFile(path.join(output, `${id}-alpha.png`));
    graphic = { image: `${id}.png`, alpha: `${id}-alpha.png`, bbox: [left, top, info.width, info.height] };
    if (!patch.integrate_in_background) overlays.push({ input: png, left, top });
    for (const [n, obstacle] of level.sight_obstacles.entries()) {
      const xs = obstacle.points.map(p => p.x);
      const ys = obstacle.points.flatMap(p => [p.y - p.z_bottom, p.y - p.z_top]);
      const x0 = Math.max(0, Math.floor(Math.min(...xs) - left));
      const x1 = Math.min(info.width, Math.ceil(Math.max(...xs) - left));
      const y0 = Math.max(0, Math.floor(Math.min(...ys) - top));
      const y1 = Math.min(info.height, Math.ceil(Math.max(...ys) - top));
      let pixels = 0;
      for (let y = y0; y < y1; y++) for (let x = x0; x < x1; x++)
        if (data[(y * info.width + x) * 4 + 3]! > 127) pixels++;
      if (pixels) candidates.push({ source_node: `building-${String(n).padStart(3, "0")}`, alpha_overlap_pixels: pixels });
    }
  }
  patches.push({ id, name: sprite.profile_name, graphic,
    // Raw records retain mask references, triggers, doors and pathfinder state without guessing mesh removal.
    state: patch,
    sight_before: patch.old_sight_obstacles.map(n => `building-${String(n).padStart(3, "0")}`),
    sight_after: patch.new_sight_obstacles.map(n => `building-${String(n).padStart(3, "0")}`),
    coverage_candidates: candidates,
  });
}
await sharp(rawPath).composite(overlays).png().toFile(path.join(output, "covered.png"));
const missionPatches = await exportMissionPatchLayers(map, output, level.patches.length);
await fs.writeFile(path.join(output, "layers.json"), JSON.stringify({
  version: 1, map, size: [width, height], elevation_degrees: 35,
  sources: { exterior: "covered.png", interior: "revealed.png" },
  projection: "pixel_x = scene_x; pixel_y = -scene_y*sin(elevation) - scene_z*cos(elevation)",
  coverage_candidates_are: "Projected obstacle bounding-box intersections with opaque cover pixels; review candidates before assigning receiver or occluder roles.",
  patches,
  mission_patches: missionPatches,
  mission_patch_note: "Mission patches have independent initial, transition and applied states; select a mission and state before compositing or projecting them. They are not building interior covers.",
}, null, 2) + "\n");
console.log(JSON.stringify({ output, patches: patches.length, graphics: overlays.length, mission_patches: missionPatches.length }));
