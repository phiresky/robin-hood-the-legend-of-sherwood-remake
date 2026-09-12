// Which reading of the SAM 3D pose keeps buildings upright? Fits every
// reconstructed asset of a map (from the cached GLB + stored pose, no API
// calls) under all 24 interpretations and reports, per interpretation, the
// mean/median pre-snap tilt and how often it is the most upright one.
//
//   node src/pose-diag.ts --map york
import path from "node:path";
import sharp from "sharp";
import { readAssetDescriptor, readLibraryIndex } from "./library.ts";
import { libraryDir } from "./env.ts";
import { loadProtoLevel } from "./asset-writer.ts";
import { fitMapCamera } from "./map-camera.ts";
import { loadGlb } from "./mesh.ts";
import { describe, fitAll } from "./reconstruct.ts";

async function main() {
  const argv = process.argv.slice(2);
  const i = argv.indexOf("--map");
  const map = i >= 0 ? argv[i + 1] : undefined;
  if (!map) throw new Error("usage: --map <name>");
  const level = await loadProtoLevel(map);
  const fit = fitMapCamera(level);
  const cam = { kind: fit.kind, elevation_deg: fit.elevation_deg };
  const index = await readLibraryIndex(libraryDir, true);
  const stats = new Map<string, { tilts: number[]; ious: number[]; wins: number }>();
  let n = 0;
  for (const e of index) {
    if (e.source_map.toLowerCase() !== map.toLowerCase()) continue;
    const dir = path.join(libraryDir, e.id);
    const desc = await readAssetDescriptor(path.join(dir, "asset.json"));
    if (!desc.model?.pose_l2c) continue;
    const mesh = await loadGlb(path.join(dir, desc.model.glb));
    const mask = new Uint8Array(
      await sharp(path.join(dir, desc.images.mask)).extractChannel(0).raw().toBuffer(),
    );
    const cands = fitAll(
      e.id,
      mesh,
      { pose: desc.model.pose_l2c },
      cam,
      desc.source.bbox,
      mask,
    );
    const best = cands.reduce((a, b) => (b.tilt_deg < a.tilt_deg ? b : a));
    for (const c of cands) {
      const key = describe(c);
      const st = stats.get(key) ?? { tilts: [], ious: [], wins: 0 };
      st.tilts.push(c.tilt_deg);
      st.ious.push(c.iou);
      if (c === best) st.wins++;
      stats.set(key, st);
    }
    n++;
  }
  const rows = [...stats.entries()].map(([key, st]) => {
    const sorted = [...st.tilts].sort((a, b) => a - b);
    return {
      key,
      mean: st.tilts.reduce((a, b) => a + b, 0) / st.tilts.length,
      median: sorted[sorted.length >> 1]!,
      under20: st.tilts.filter((t) => t < 20).length,
      wins: st.wins,
      iou: st.ious.reduce((a, b) => a + b, 0) / st.ious.length,
    };
  });
  rows.sort((a, b) => a.mean - b.mean);
  console.log(`${n} assets; per interpretation: mean tilt / median / <20° count / most-upright wins / mean IoU`);
  for (const r of rows) {
    console.log(
      `${r.key.padEnd(26)} ${r.mean.toFixed(1).padStart(6)}° ${r.median.toFixed(0).padStart(4)}° ${String(r.under20).padStart(4)} ${String(r.wins).padStart(4)}  ${r.iou.toFixed(3)}`,
    );
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
