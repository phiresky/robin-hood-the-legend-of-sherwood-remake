// fal.ai SAM 3 wrapper using the RLE endpoint (fal-ai/sam-3/image-rle).
// Input params follow the typed schema shipped with @fal-ai/client
// (Sam3ImageInput): image_url, prompt, point_prompts (label "0"/"1"),
// box_prompts (x_min..y_max), return_multiple_masks, max_masks,
// include_scores, include_boxes. Output: rle: string | string[] plus
// scores/boxes/metadata arrays.
//
// The rle strings' exact encoding (bare COCO counts vs JSON {size, counts})
// is not documented in the type — decodeRle() handles both and fails loudly
// otherwise; the first supervised run pins it down.
import crypto from "node:crypto";
import fs from "node:fs/promises";
import path from "node:path";
import { fal } from "@fal-ai/client";
import type { Sam3ImageRleInput, SAM3RLEOutput } from "@fal-ai/client/endpoints";
import { requireEnv, workDir } from "./env";

let configured = false;
function ensureConfigured() {
  if (!configured) {
    fal.config({ credentials: requireEnv("FAL_KEY") });
    configured = true;
  }
}

export interface SamMask {
  /** row-major 0/255 mask, dimensions `width` x `height` */
  data: Uint8Array;
  width: number;
  height: number;
  score: number | null;
  /** normalized [cx, cy, w, h] if returned */
  box: [number, number, number, number] | null;
}

export interface SamRequest {
  imagePng: Buffer;
  /** dimensions of imagePng — masks come back at this resolution */
  width: number;
  height: number;
  /** open-vocabulary concept prompt, e.g. "stone building with red roof" */
  prompt?: string;
  points?: { x: number; y: number; label: 0 | 1 }[];
  /** [x_min, y_min, x_max, y_max] */
  boxes?: [number, number, number, number][];
  maxMasks?: number;
}

/**
 * Decode one rle entry from the endpoint.
 *
 * Verified empirically (2026-08-20): the string is space-separated Kaggle-style
 * `start length` pairs, ROW-major, 1-indexed, at the resolution of the
 * submitted image (max index fit the crop area and the first start only lands
 * inside the returned bounding box under row-major ordering).
 */
export function decodeRle(
  rleStr: string,
  width: number,
  height: number,
): { data: Uint8Array; width: number; height: number } {
  const nums = rleStr.trim().split(/\s+/).map(Number);
  if (nums.length % 2 !== 0 || nums.some(Number.isNaN)) {
    throw new Error(
      `unexpected rle shape (${nums.length} numbers); first 120 chars: ${JSON.stringify(rleStr.slice(0, 120))}`,
    );
  }
  const data = new Uint8Array(width * height);
  for (let i = 0; i < nums.length; i += 2) {
    const start = nums[i]! - 1; // 1-indexed
    const len = nums[i + 1]!;
    if (start < 0 || start + len > data.length) {
      throw new Error(`rle run out of range: start ${start} len ${len} for ${width}x${height}`);
    }
    data.fill(255, start, start + len);
  }
  return { data, width, height };
}

/**
 * Every response is cached in work/sam-cache/<hash>.json keyed by the request
 * (image bytes + all prompt params), together with the input image, so
 * processing can be reworked later without re-hitting the API.
 */
async function cachedSubscribe(
  input: Sam3ImageRleInput,
  imagePng: Buffer,
): Promise<SAM3RLEOutput> {
  const cacheDir = path.join(workDir, "sam-cache");
  await fs.mkdir(cacheDir, { recursive: true });
  const { image_url: _, ...params } = input;
  const key = crypto
    .createHash("sha256")
    .update(imagePng)
    .update(JSON.stringify(params))
    .digest("hex")
    .slice(0, 24);
  const cachePath = path.join(cacheDir, `${key}.json`);
  try {
    const cached = JSON.parse(await fs.readFile(cachePath, "utf8"));
    console.log(`sam-cache hit: ${key}`);
    return cached.response as SAM3RLEOutput;
  } catch {
    // miss
  }

  const file = new Blob([new Uint8Array(imagePng)], { type: "image/png" });
  input.image_url = await fal.storage.upload(file as unknown as File);
  const result = await fal.subscribe("fal-ai/sam-3/image-rle", { input, logs: false });

  await fs.writeFile(path.join(cacheDir, `${key}.input.png`), imagePng);
  await fs.writeFile(
    cachePath,
    JSON.stringify(
      {
        endpoint: "fal-ai/sam-3/image-rle",
        requested_at: new Date().toISOString(),
        params,
        input_image: `${key}.input.png`,
        request_id: result.requestId,
        response: result.data,
      },
      null,
      1,
    ),
  );
  return result.data as SAM3RLEOutput;
}

export async function segment(req: SamRequest): Promise<SamMask[]> {
  ensureConfigured();

  const input: Sam3ImageRleInput = {
    image_url: "", // filled by cachedSubscribe on cache miss
    return_multiple_masks: true,
    max_masks: req.maxMasks ?? 8,
    include_scores: true,
    include_boxes: true,
  };
  if (req.prompt !== undefined) input.prompt = req.prompt;
  if (req.points?.length)
    input.point_prompts = req.points.map((p) => ({
      x: p.x,
      y: p.y,
      label: String(p.label) as "0" | "1",
    }));
  if (req.boxes?.length)
    input.box_prompts = req.boxes.map(([x_min, y_min, x_max, y_max]) => ({
      x_min,
      y_min,
      x_max,
      y_max,
    }));

  const data = await cachedSubscribe(input, req.imagePng);
  const rles = Array.isArray(data.rle) ? data.rle : [data.rle];
  if (rles.length === 0 || rles[0] === undefined) {
    throw new Error(`SAM 3 RLE returned no masks; keys: ${Object.keys(data).join(", ")}`);
  }

  return rles.map((rleStr, i) => ({
    ...decodeRle(rleStr, req.width, req.height),
    score: data.scores?.[i] ?? data.metadata?.[i]?.score ?? null,
    box:
      (data.boxes?.[i] as [number, number, number, number] | undefined) ??
      (data.metadata?.[i]?.box as [number, number, number, number] | undefined) ??
      null,
  }));
}
