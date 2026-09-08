// Alternative image-to-3D backends on fal.ai, for comparison against SAM 3D
// Objects (sam3d.ts). All take a single RGBA cutout (object on transparent
// background) and return one textured GLB in a canonical Y-up frame with no
// pose; placement is fitted afterwards (reconstruct.ts, identity pose + yaw
// search). Responses and GLBs are cached under work/<backend>-cache/<hash>/.
import crypto from "node:crypto";
import fs from "node:fs/promises";
import path from "node:path";
import { fal } from "@fal-ai/client";
import { requireEnv, workDir } from "./env.ts";
import {
  cacheDirectory,
  contentKey,
  cachedArtifacts,
  validateGlb,
  type CacheOptions,
} from "./provider-cache.ts";

export type Backend = "sam3d" | "trellis2" | "tripo" | "hunyuan";
export const ALT_BACKENDS: Exclude<Backend, "sam3d">[] = [
  "trellis2",
  "tripo",
  "hunyuan",
];

interface BackendSpec {
  endpoint: string;
  /** approximate price per generation in USD (fal, Sept 2026) */
  price: number;
  input: (imageUrl: string, seed: number) => Record<string, unknown>;
  /** pick the GLB file reference out of the response */
  glb: (data: Record<string, unknown>) => { url: string } | undefined;
}

const SPECS: Record<Exclude<Backend, "sam3d">, BackendSpec> = {
  trellis2: {
    endpoint: "fal-ai/trellis-2",
    price: 0.3,
    input: (image_url, seed) => ({
      image_url,
      seed,
      resolution: "1024",
      decimation_target: 100000,
      texture_size: "2048",
    }),
    glb: (d) => d.model_glb as { url: string } | undefined,
  },
  tripo: {
    endpoint: "tripo3d/h3.1/image-to-3d",
    price: 0.3,
    input: (image_url, seed) => ({
      image_url,
      texture: true,
      pbr: false,
      texture_quality: "standard",
      geometry_quality: "standard",
      texture_alignment: "original_image",
      // rotate the model to match the input view; our yaw search verifies it
      orientation: "align_image",
      model_seed: seed,
      texture_seed: seed,
    }),
    glb: (d) => {
      const urls = d.model_urls as { glb?: { url: string } } | undefined;
      return urls?.glb ?? (d.model_mesh as { url: string } | undefined);
    },
  },
  hunyuan: {
    endpoint: "fal-ai/hunyuan-3d/v3.1/pro/image-to-3d",
    price: 0.375,
    input: (input_image_url) => ({
      input_image_url,
      generate_type: "Normal",
      enable_pbr: false,
      face_count: 100000,
    }),
    glb: (d) => d.model_glb as { url: string } | undefined,
  },
};

export function backendInfo(b: Exclude<Backend, "sam3d">): {
  endpoint: string;
  price: number;
} {
  return { endpoint: SPECS[b].endpoint, price: SPECS[b].price };
}

let configured = false;
function ensureConfigured() {
  if (!configured) {
    fal.config({ credentials: requireEnv("FAL_KEY") });
    configured = true;
  }
}

export interface BackendResult {
  backend: Exclude<Backend, "sam3d">;
  endpoint: string;
  requestId: string;
  cacheDir: string;
  /** local GLB path in the cache dir */
  glb: string;
  /** wall-clock seconds of the request (0 on cache hit) */
  seconds: number;
}

export async function reconstructWith(
  backend: Exclude<Backend, "sam3d">,
  cutoutPng: Buffer,
  seed: number,
  options: CacheOptions = {},
): Promise<BackendResult> {
  const spec = SPECS[backend];
  const params = spec.input("<image>", seed);
  const key = crypto
    .createHash("sha256")
    .update(cutoutPng)
    .update(JSON.stringify({ backend, params }))
    .digest("hex")
    .slice(0, 24);
  const directory = await cacheDirectory(
    path.join(options.workDirectory ?? workDir, `${backend}-cache`),
    contentKey([spec.endpoint, cutoutPng, JSON.stringify(params)]),
    key,
  );
  let seconds = 0;
  return cachedArtifacts(
    directory,
    async (cacheDir) => {
      const cached = JSON.parse(
        await fs.readFile(path.join(cacheDir, "response.json"), "utf8"),
      );
      if (
        cached.endpoint !== spec.endpoint ||
        typeof cached.request_id !== "string" ||
        !spec.glb(cached.response)?.url
      ) {
        throw new Error(`invalid ${backend} response metadata`);
      }
      const glb = path.join(cacheDir, "model.glb");
      await validateGlb(glb);
      return {
        backend,
        endpoint: spec.endpoint,
        requestId: cached.request_id,
        cacheDir,
        glb,
        seconds,
      };
    },
    async (cacheDir) => {
      ensureConfigured();
      await fs.writeFile(path.join(cacheDir, "input.png"), cutoutPng);
      const imageUrl = await fal.storage.upload(
        new Blob([new Uint8Array(cutoutPng)], {
          type: "image/png",
        }) as unknown as File,
      );
      const t0 = Date.now();
      const result = await fal.subscribe(spec.endpoint, {
        input: spec.input(imageUrl, seed),
        logs: false,
      });
      seconds = (Date.now() - t0) / 1000;
      const data = result.data as Record<string, unknown>;
      await fs.writeFile(
        path.join(cacheDir, "response.json"),
        JSON.stringify({
          endpoint: spec.endpoint,
          requested_at: new Date().toISOString(),
          seconds: (Date.now() - t0) / 1000,
          params,
          request_id: result.requestId,
          response: data,
        }),
      );
      const ref = spec.glb(data);
      if (!ref?.url) throw new Error(`${spec.endpoint}: no GLB in response`);
      const res = await fetch(ref.url);
      if (!res.ok) throw new Error(`download ${ref.url}: HTTP ${res.status}`);
      await fs.writeFile(
        path.join(cacheDir, "model.glb"),
        Buffer.from(await res.arrayBuffer()),
      );
    },
    options,
  );
}
