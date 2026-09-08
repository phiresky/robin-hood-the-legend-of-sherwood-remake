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
import type {
  Sam3ImageRleInput,
  SAM3RLEOutput,
} from "@fal-ai/client/endpoints";
import { requireEnv, workDir } from "./env";
import {
  contentKey,
  cachedArtifacts,
  isMissing,
  type CacheOptions,
} from "./provider-cache";

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
  if (
    !Number.isSafeInteger(width) ||
    !Number.isSafeInteger(height) ||
    width <= 0 ||
    height <= 0
  )
    throw new Error("invalid mask dimensions");
  const nums = rleStr.trim() ? rleStr.trim().split(/\s+/).map(Number) : [];
  if (nums.length % 2 !== 0 || nums.some((n) => !Number.isSafeInteger(n))) {
    throw new Error(
      `unexpected rle shape (${nums.length} numbers); first 120 chars: ${JSON.stringify(rleStr.slice(0, 120))}`,
    );
  }
  const data = new Uint8Array(width * height);
  for (let i = 0; i < nums.length; i += 2) {
    const start = nums[i]! - 1; // 1-indexed
    const len = nums[i + 1]!;
    if (start < 0 || len < 0 || start + len > data.length) {
      throw new Error(
        `rle run out of range: start ${start} len ${len} for ${width}x${height}`,
      );
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
  req: SamRequest,
  options: CacheOptions,
): Promise<SAM3RLEOutput> {
  const { imagePng } = req;
  const cacheDir = path.join(options.workDirectory ?? workDir, "sam-cache");
  const { image_url: _, ...params } = input;
  const key = crypto
    .createHash("sha256")
    .update(imagePng)
    .update(JSON.stringify(params))
    .digest("hex")
    .slice(0, 24);
  const validate = (cached: unknown): SAM3RLEOutput => {
    const record = cached as { endpoint?: unknown; response?: SAM3RLEOutput };
    if (record.endpoint !== "fal-ai/sam-3/image-rle" || !record.response)
      throw new Error("invalid SAM cache response");
    const data = record.response;
    const rles = Array.isArray(data.rle) ? data.rle : [data.rle];
    if (rles.some((r) => typeof r !== "string"))
      throw new Error("invalid SAM RLE response");
    for (const rle of rles) decodeRle(rle, req.width, req.height);
    return data;
  };
  // Reuse the old flat cache without turning a corrupt paid response into a miss.
  let legacy: string | undefined;
  try {
    legacy = await fs.readFile(path.join(cacheDir, `${key}.json`), "utf8");
  } catch (error) {
    if (!isMissing(error)) throw error;
  }
  if (legacy !== undefined) return validate(JSON.parse(legacy));
  return cachedArtifacts(
    path.join(
      cacheDir,
      contentKey(["fal-ai/sam-3/image-rle", imagePng, JSON.stringify(params)]),
    ),
    async (dir) => {
      return validate(
        JSON.parse(await fs.readFile(path.join(dir, "response.json"), "utf8")),
      );
    },
    async (dir) => {
      ensureConfigured();
      const image_url = await fal.storage.upload(
        new Blob([new Uint8Array(imagePng)], {
          type: "image/png",
        }) as unknown as File,
      );
      let response: SAM3RLEOutput;
      let requestId: string | undefined;
      try {
        const result = await fal.subscribe("fal-ai/sam-3/image-rle", {
          input: { ...input, image_url },
          logs: false,
        });
        response = result.data;
        requestId = result.requestId;
      } catch (error) {
        const err = error as { status?: number; body?: { detail?: unknown } };
        const detail =
          typeof err.body?.detail === "string"
            ? err.body.detail
            : JSON.stringify(err.body?.detail ?? "");
        if (err.status !== 422 || !detail.includes("No masks generated"))
          throw error;
        response = { rle: [] } as unknown as SAM3RLEOutput;
      }
      await fs.writeFile(path.join(dir, "input.png"), imagePng);
      await fs.writeFile(
        path.join(dir, "response.json"),
        JSON.stringify({
          endpoint: "fal-ai/sam-3/image-rle",
          params,
          request_id: requestId,
          response,
        }),
      );
    },
    options,
  );
}

export async function segment(
  req: SamRequest,
  options: CacheOptions = {},
): Promise<SamMask[]> {
  if (
    !Number.isSafeInteger(req.width) ||
    !Number.isSafeInteger(req.height) ||
    req.width <= 0 ||
    req.height <= 0
  )
    throw new Error("invalid SAM image dimensions");

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

  const data = await cachedSubscribe(input, req, options);
  const rles = (Array.isArray(data.rle) ? data.rle : [data.rle]).filter(
    (r): r is string => r !== undefined,
  );

  return rles.map((rleStr, i) => ({
    ...decodeRle(rleStr, req.width, req.height),
    score: data.scores?.[i] ?? data.metadata?.[i]?.score ?? null,
    box:
      (data.boxes?.[i] as [number, number, number, number] | undefined) ??
      (data.metadata?.[i]?.box as
        [number, number, number, number] | undefined) ??
      null,
  }));
}
