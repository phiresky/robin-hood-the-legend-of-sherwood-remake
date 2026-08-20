// Run a batch of extraction proposals from a JSON file.
//
//   pnpm --filter pipeline exec tsx src/sweep.ts proposals/leicester.json
//
// Proposal file: { "map": "...", "proposals": [{ bbox, prompt, name, id?,
// tags?, pick?, scale_class?, variant_group?, max_masks?, min_score?, pad? }] }
// Unset fields fall back to EXTRACT_DEFAULTS. Failures don't abort the batch.
import fs from "node:fs/promises";
import type { Bbox } from "./clip";
import {
  EXTRACT_DEFAULTS,
  runExtraction,
  slugify,
  type ExtractOptions,
  type ExtractSummary,
} from "./extract-core";

interface Proposal {
  dedupe_iou?: number;
  bbox: Bbox;
  prompt?: string;
  /** fallback prompts tried in order when the previous one finds no masks */
  alt_prompts?: string[];
  /** box prompts in world coords [x, y, w, h] */
  boxes?: Bbox[];
  /** point prompts in world coords; label 1 = foreground, 0 = background */
  points?: { x: number; y: number; label: 0 | 1 }[];
  /** composite roof-closing patch sprites onto the map first */
  apply_patches?: boolean;
  /** "crop" = rectangular swatch without segmentation */
  mode?: "sam" | "crop";
  name: string;
  id?: string;
  tags?: string[];
  pick?: number | "best" | "all";
  scale_class?: ExtractOptions["scaleClass"];
  variant_group?: string;
  max_masks?: number;
  min_score?: number;
  min_area?: number;
  pad?: number;
}

interface SweepFile {
  map: string;
  proposals: Proposal[];
}

async function main() {
  const file = process.argv[2];
  if (!file) throw new Error("usage: tsx src/sweep.ts <proposals.json>");
  const sweep: SweepFile = JSON.parse(await fs.readFile(file, "utf8"));

  const results: { id: string; written: number; skipped: number; error?: string }[] = [];
  for (const p of sweep.proposals) {
    const id = p.id ?? slugify(p.name);
    console.log(`\n=== ${id}: "${p.prompt}" @ ${p.bbox.join(",")} ===`);
    try {
      let summary: ExtractSummary | null = null;
      for (const prompt of [p.prompt, ...(p.alt_prompts ?? [])]) {
        summary = await runExtraction({
          map: sweep.map,
          bbox: p.bbox,
          prompt,
          boxes: p.boxes,
          points: p.points,
          applyPatches: p.apply_patches,
          mode: p.mode,
          name: p.name,
          id,
          tags: p.tags ?? [],
          pad: p.pad ?? EXTRACT_DEFAULTS.pad,
          maxMasks: p.max_masks ?? EXTRACT_DEFAULTS.maxMasks,
          pick: p.pick ?? EXTRACT_DEFAULTS.pick,
          scaleClass: p.scale_class ?? EXTRACT_DEFAULTS.scaleClass,
          variantGroup: p.variant_group,
          minScore: p.min_score ?? EXTRACT_DEFAULTS.minScore,
          minArea: p.min_area ?? EXTRACT_DEFAULTS.minArea,
          dedupeIou: p.dedupe_iou ?? EXTRACT_DEFAULTS.dedupeIou,
        });
        if (summary.written.length > 0) break;
        console.log(`no assets from "${prompt}", trying next prompt if any`);
      }
      results.push({ id, written: summary!.written.length, skipped: summary!.skipped.length });
    } catch (e) {
      console.error(`FAILED: ${e}`);
      results.push({ id, written: 0, skipped: 0, error: String(e) });
    }
  }

  console.log("\n=== sweep summary ===");
  for (const r of results) {
    console.log(
      `${r.id}: ${r.error ? `ERROR ${r.error}` : `${r.written} written, ${r.skipped} skipped`}`,
    );
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
