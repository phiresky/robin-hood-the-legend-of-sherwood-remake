// CLI wrapper around extract-core.
//
//   pnpm --filter pipeline extract -- --map Leicester --bbox 2900,400,600,500 \
//     --prompt "watermill building" --name "Leicester watermill" --tags building,water
import type { Bbox } from "./clip";
import { EXTRACT_DEFAULTS, runExtraction, slugify, type ExtractOptions } from "./extract-core";

function parseArgs(argv: string[]): ExtractOptions {
  const get = (flag: string): string | undefined => {
    const i = argv.indexOf(`--${flag}`);
    return i >= 0 ? argv[i + 1] : undefined;
  };
  const req = (flag: string): string => {
    const v = get(flag);
    if (!v) throw new Error(`missing --${flag}`);
    return v;
  };
  const bbox = req("bbox").split(",").map(Number);
  if (bbox.length !== 4 || bbox.some(Number.isNaN)) throw new Error("--bbox wants x,y,w,h");
  const name = req("name");
  return {
    map: req("map"),
    bbox: bbox as Bbox,
    prompt: req("prompt"),
    name,
    id: get("id") ?? slugify(name),
    tags: get("tags")?.split(",") ?? [],
    pad: Number(get("pad") ?? EXTRACT_DEFAULTS.pad),
    maxMasks: Number(get("max-masks") ?? EXTRACT_DEFAULTS.maxMasks),
    pick:
      get("pick") === undefined ? "best" : get("pick") === "all" ? "all" : Number(get("pick")),
    scaleClass: (get("scale-class") as ExtractOptions["scaleClass"]) ?? EXTRACT_DEFAULTS.scaleClass,
    variantGroup: get("variant-group"),
    minScore: Number(get("min-score") ?? EXTRACT_DEFAULTS.minScore),
    minArea: Number(get("min-area") ?? EXTRACT_DEFAULTS.minArea),
    dedupeIou: Number(get("dedupe-iou") ?? EXTRACT_DEFAULTS.dedupeIou),
  };
}

runExtraction(parseArgs(process.argv.slice(2))).catch((e) => {
  console.error(e);
  process.exit(1);
});
