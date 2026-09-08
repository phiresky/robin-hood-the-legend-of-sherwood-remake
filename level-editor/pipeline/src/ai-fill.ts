// AI texture completion of face tiles with gpt-image-2 (OpenAI images/edits).
//
//   node src/ai-fill.ts --tiles a.png,b.png [--map york --contexts "x0,y0,x1,y1;…"] [--out dir]
//       [--quality low|medium|high]
//
// With --map and --contexts (one map-pixel box per tile, the face's
// projected bbox) a crop of the original map around that box, with 40 %
// margin, goes along as a second reference image so the model sees the
// whole building, its neighbours and the painting style.
//
// Each input is an RGBA tile whose alpha-0 pixels are the holes (the
// --fill none output of volumes.ts, or a VOLUMES_DUMP face). The tile is
// scaled up (at most 4x) into a square canvas, the known pixels stay, the
// holes are transparent, and a mask PNG marks them: white where the
// original is to be preserved, transparent where the model should paint.
// The result is scaled back and only the hole pixels are copied into the
// tile, so known pixels are never touched. Responses are cached under
// work/ai-fill-cache/<hash>/.
//
// The call goes straight to OpenAI: OpenRouter's Images API accepts a
// `mask` field but drops it (a probe with a mask exposing one small square
// still had the whole canvas repainted), and its reference-image route
// cannot protect known pixels. Needs OPENAI_API_KEY in level-editor/.env.
import crypto from "node:crypto";
import { pathToFileURL } from "node:url";
import {
  cacheDirectory,
  contentKey,
  cachedArtifacts,
  type CacheOptions,
} from "./provider-cache.ts";
import fs from "node:fs/promises";
import path from "node:path";
import sharp from "sharp";
import { requireEnv, workDir } from "./env.ts";
import { loadProtoLevel, mapImageSource } from "./asset-writer.ts";

const MODEL = "gpt-image-2";
const CANVAS = 1024;
const MAX_UPSCALE = 4;

const CONTEXT_PROMPT = ` The second image is a crop of the original map around this very surface (the game's oblique view): the tile is an unwrapped face of a building in it. Use it to see which building and material this is, its colours, neighbours and how the painter handled similar surfaces, and paint the missing area consistently with it.`;

const PROMPT = `This is one texture tile of a hand-painted isometric medieval town map (oblique view from the south, soft daylight from the upper left, muted earthy palette, fine painterly detail at about one brush stroke per pixel). Repaint only the transparent areas so the surface continues seamlessly: same material, same colours, same texture scale, same light direction, same level of detail. Continue brick courses, timber framing, roof tiles or ground texture naturally. Do not add new objects, text, people or shadows. Keep every non-transparent pixel exactly as it is.`;

export interface Prepared {
  /** square RGBA canvas with holes transparent */
  canvas: Buffer;
  /** mask: white opaque = keep, transparent = paint */
  mask: Buffer;
  scale: number;
  ox: number;
  oy: number;
  w: number;
  h: number;
}

export async function prepare(tilePng: Buffer): Promise<Prepared> {
  const { data, info } = await sharp(tilePng)
    .ensureAlpha()
    .raw()
    .toBuffer({ resolveWithObject: true });
  const w = info.width;
  const h = info.height;
  const scale = Math.min(
    MAX_UPSCALE,
    Math.floor((CANVAS / Math.max(w, h)) * 100) / 100,
  );
  const sw = Math.round(w * scale);
  const sh = Math.round(h * scale);
  const ox = Math.floor((CANVAS - sw) / 2);
  const oy = Math.floor((CANVAS - sh) / 2);
  // known pixels scaled with nearest neighbour so the paint grain stays crisp
  const scaled = await sharp(data, {
    raw: { width: w, height: h, channels: 4 },
  })
    .resize(sw, sh, { kernel: "nearest" })
    .raw()
    .toBuffer();
  const canvas = Buffer.alloc(CANVAS * CANVAS * 4);
  const mask = Buffer.alloc(CANVAS * CANVAS * 4);
  // outside the tile: keep (white mask) with a neutral mid-grey so the model does not paint there
  for (let i = 0; i < CANVAS * CANVAS; i++) {
    canvas[i * 4] = 128;
    canvas[i * 4 + 1] = 128;
    canvas[i * 4 + 2] = 128;
    canvas[i * 4 + 3] = 255;
    mask[i * 4] = 255;
    mask[i * 4 + 1] = 255;
    mask[i * 4 + 2] = 255;
    mask[i * 4 + 3] = 255;
  }
  for (let y = 0; y < sh; y++) {
    for (let x = 0; x < sw; x++) {
      const s = (y * sw + x) * 4;
      const d = ((oy + y) * CANVAS + ox + x) * 4;
      const known = scaled[s + 3]! > 0;
      canvas[d] = scaled[s]!;
      canvas[d + 1] = scaled[s + 1]!;
      canvas[d + 2] = scaled[s + 2]!;
      canvas[d + 3] = known ? 255 : 0;
      mask[d + 3] = known ? 255 : 0;
    }
  }
  return { canvas, mask, scale, ox, oy, w, h };
}

interface Result {
  png: Buffer;
  cost?: number;
  cached: boolean;
  seconds: number;
}

export interface ImageProviderOptions extends CacheOptions {
  request?: typeof fetch;
  apiKey?: () => string;
}
export async function callOpenAI(
  prep: Prepared,
  quality: string,
  fidelity: string,
  context: Buffer | null,
  options: ImageProviderOptions = {},
): Promise<Result> {
  const imagePng = await sharp(prep.canvas, {
    raw: { width: CANVAS, height: CANVAS, channels: 4 },
  })
    .png()
    .toBuffer();
  const maskPng = await sharp(prep.mask, {
    raw: { width: CANVAS, height: CANVAS, channels: 4 },
  })
    .png()
    .toBuffer();
  // gpt-image-2 rejects input_fidelity (gpt-image-1 takes it); send it only when asked for explicitly
  const params: Record<string, string> = {
    model: MODEL,
    prompt: PROMPT + (context ? CONTEXT_PROMPT : ""),
    size: `${CANVAS}x${CANVAS}`,
    quality,
    output_format: "png",
    n: "1",
  };
  if (fidelity !== "default") params.input_fidelity = fidelity;
  const hash = crypto
    .createHash("sha256")
    .update(imagePng)
    .update(maskPng)
    .update(context ?? Buffer.alloc(0))
    .update(JSON.stringify(params))
    .digest("hex")
    .slice(0, 24);
  const directory = await cacheDirectory(
    path.join(options.workDirectory ?? workDir, "ai-fill-cache"),
    contentKey([
      "openai/images/edits",
      imagePng,
      maskPng,
      context ?? Buffer.alloc(0),
      JSON.stringify(params),
    ]),
    hash,
  );
  let generated = false;
  return cachedArtifacts(
    directory,
    async (dir) => {
      const png = await fs.readFile(path.join(dir, "out.png"));
      const { info } = await sharp(png)
        .raw()
        .toBuffer({ resolveWithObject: true });
      if (info.width !== CANVAS || info.height !== CANVAS)
        throw new Error("invalid cached AI image dimensions");
      const meta = JSON.parse(
        await fs.readFile(path.join(dir, "meta.json"), "utf8"),
      );
      if (meta.model !== MODEL || !Number.isFinite(meta.seconds))
        throw new Error("invalid AI cache metadata");
      return {
        png,
        cost: meta.cost,
        cached: !generated,
        seconds: generated ? meta.seconds : 0,
      };
    },
    async (dir) => {
      const key = (options.apiKey ?? (() => requireEnv("OPENAI_API_KEY")))();
      await fs.writeFile(path.join(dir, "in.png"), imagePng);
      await fs.writeFile(path.join(dir, "mask.png"), maskPng);
      const form = new FormData();
      for (const [k, v] of Object.entries(params)) form.append(k, v);
      // several images: the first is the one edited (the mask applies to it), the rest are references
      form.append(
        "image[]",
        new Blob([new Uint8Array(imagePng)], { type: "image/png" }),
        "tile.png",
      );
      if (context) {
        await fs.writeFile(path.join(dir, "context.png"), context);
        form.append(
          "image[]",
          new Blob([new Uint8Array(context)], { type: "image/png" }),
          "context.png",
        );
      }
      form.append(
        "mask",
        new Blob([new Uint8Array(maskPng)], { type: "image/png" }),
        "mask.png",
      );
      const t0 = Date.now();
      const res = await (options.request ?? fetch)(
        "https://api.openai.com/v1/images/edits",
        {
          method: "POST",
          headers: { Authorization: `Bearer ${key}` },
          body: form,
        },
      );
      const text = await res.text();
      if (!res.ok)
        throw new Error(`openai ${res.status}: ${text.slice(0, 600)}`);
      const json = JSON.parse(text) as {
        data?: { b64_json?: string }[];
        usage?: {
          input_tokens?: number;
          output_tokens?: number;
          input_tokens_details?: {
            image_tokens?: number;
            text_tokens?: number;
          };
        };
      };
      const b64 = json.data?.[0]?.b64_json;
      if (!b64) throw new Error(`no image in response: ${text.slice(0, 400)}`);
      const png = await sharp(Buffer.from(b64, "base64")).png().toBuffer();
      const seconds = (Date.now() - t0) / 1000;
      // gpt-image pricing: $5/M text in, $8/M image in, $30/M image out (per token)
      const u = json.usage;
      const cost = u
        ? ((u.input_tokens_details?.text_tokens ?? 0) * 5 +
            (u.input_tokens_details?.image_tokens ?? 0) * 8 +
            (u.output_tokens ?? 0) * 30) /
          1e6
        : undefined;
      await fs.writeFile(path.join(dir, "out.png"), png);
      await fs.writeFile(
        path.join(dir, "meta.json"),
        JSON.stringify(
          {
            model: MODEL,
            quality,
            fidelity,
            usage: u,
            cost,
            seconds,
            requested_at: new Date().toISOString(),
          },
          null,
          1,
        ),
      );
      generated = true;
    },
    options,
  );
}

/** scale the result back and copy only the hole pixels into the tile */
export async function composite(
  tilePng: Buffer,
  prep: Prepared,
  outPng: Buffer,
): Promise<{ rgba: Buffer; changedKnown: number }> {
  const meta = await sharp(outPng).metadata();
  const sx = (meta.width ?? CANVAS) / CANVAS;
  const sy = (meta.height ?? CANVAS) / CANVAS;
  const out = await sharp(outPng)
    .extract({
      left: Math.round(prep.ox * sx),
      top: Math.round(prep.oy * sy),
      width: Math.round(prep.w * prep.scale * sx),
      height: Math.round(prep.h * prep.scale * sy),
    })
    .resize(prep.w, prep.h, { kernel: "lanczos3" })
    .removeAlpha()
    .raw()
    .toBuffer();
  const tile = await sharp(tilePng).ensureAlpha().raw().toBuffer();
  const rgba = Buffer.from(tile);
  let changedKnown = 0;
  for (let i = 0; i < prep.w * prep.h; i++) {
    if (tile[i * 4 + 3]! > 0) {
      // how much the model drifted on pixels it was told to keep (diagnostic only)
      const d =
        Math.abs(tile[i * 4]! - out[i * 3]!) +
        Math.abs(tile[i * 4 + 1]! - out[i * 3 + 1]!) +
        Math.abs(tile[i * 4 + 2]! - out[i * 3 + 2]!);
      if (d > 60) changedKnown++;
      continue;
    }
    rgba[i * 4] = out[i * 3]!;
    rgba[i * 4 + 1] = out[i * 3 + 1]!;
    rgba[i * 4 + 2] = out[i * 3 + 2]!;
    rgba[i * 4 + 3] = 255;
  }
  return { rgba, changedKnown };
}

async function main() {
  const argv = process.argv.slice(2);
  const get = (flag: string): string | undefined => {
    const i = argv.indexOf(`--${flag}`);
    return i >= 0 ? argv[i + 1] : undefined;
  };
  const tiles = (get("tiles") ?? "").split(",").filter(Boolean);
  if (tiles.length === 0)
    throw new Error(
      "usage: --tiles a.png,b.png [--out dir] [--quality low|medium|high] [--fidelity low|high] [--probe-square]",
    );
  const outDir = get("out") ?? path.join(workDir, "ai-fill");
  const quality = get("quality") ?? "medium";
  const fidelity = get("fidelity") ?? "default";
  const probeSquare = argv.includes("--probe-square");
  // optional map context per tile
  const contexts = (get("contexts") ?? "")
    .split(";")
    .filter(Boolean)
    .map((b) => b.split(",").map(Number) as [number, number, number, number]);
  let mapImg: sharp.Sharp | null = null;
  let mapSize: [number, number] = [0, 0];
  if (get("map") && contexts.length) {
    const level = await loadProtoLevel(get("map")!);
    const src = await mapImageSource(get("map")!, "Day", true, level);
    if (!src) throw new Error(`no Day map for ${get("map")}`);
    const png = await sharp(src).png().toBuffer();
    const meta = await sharp(png).metadata();
    mapImg = sharp(png);
    mapSize = [meta.width!, meta.height!];
  }
  const contextCrop = async (k: number): Promise<Buffer | null> => {
    const box = contexts[k];
    if (!box || !mapImg) return null;
    const [x0, y0, x1, y1] = box;
    const m = 0.4 * Math.max(x1 - x0, y1 - y0);
    const left = Math.max(0, Math.floor(x0 - m));
    const top = Math.max(0, Math.floor(y0 - m));
    const width = Math.min(mapSize[0] - left, Math.ceil(x1 - x0 + 2 * m));
    const height = Math.min(mapSize[1] - top, Math.ceil(y1 - y0 + 2 * m));
    const side = Math.max(width, height);
    const sc = Math.min(1, CANVAS / side);
    return mapImg
      .clone()
      .extract({ left, top, width, height })
      .resize(Math.round(width * sc), Math.round(height * sc))
      .png()
      .toBuffer();
  };
  await fs.mkdir(outDir, { recursive: true });
  let total = 0;
  for (const [k, file] of tiles.entries()) {
    const tilePng = await fs.readFile(file);
    const prep = await prepare(tilePng);
    const context = await contextCrop(k);
    if (probeSquare) {
      // keep everything except a 220 px square in the lower middle of the canvas
      for (let y = 0; y < CANVAS; y++) {
        for (let x = 0; x < CANVAS; x++) {
          const i = (y * CANVAS + x) * 4;
          const inSquare = x >= 400 && x < 620 && y >= 620 && y < 840;
          prep.mask[i + 3] = inSquare ? 0 : 255;
        }
      }
    }
    const stem =
      path.basename(file, path.extname(file)) +
      (probeSquare ? "-square" : "") +
      (context ? "-ctx" : "");
    try {
      const r = await callOpenAI(prep, quality, fidelity, context, {
        offline: argv.includes("--offline") ? true : undefined,
      });
      const { rgba, changedKnown } = await composite(tilePng, prep, r.png);
      await sharp(rgba, { raw: { width: prep.w, height: prep.h, channels: 4 } })
        .png()
        .toFile(path.join(outDir, `${stem}-ai.png`));
      await fs.writeFile(path.join(outDir, `${stem}-ai-raw.png`), r.png);
      total += r.cost ?? 0;
      console.log(
        `${stem}: ${prep.w}x${prep.h} x${prep.scale} ${r.cached ? "cached" : `${r.seconds.toFixed(0)} s`} cost $${(r.cost ?? 0).toFixed(3)} kept-pixel drift ${((changedKnown / (prep.w * prep.h)) * 100).toFixed(1)}%`,
      );
    } catch (e) {
      console.error(`${stem}: ${(e as Error).message}`);
    }
  }
  console.log(`total cost $${total.toFixed(3)}`);
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  main().catch((e) => {
    console.error(e);
    process.exitCode = 1;
  });
}
