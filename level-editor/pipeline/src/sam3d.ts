// fal.ai SAM 3D Objects wrapper (fal-ai/sam-3/3d-objects).
//
// Input: one image plus one binary mask PNG per object (mask_urls). Output:
// per-object GLB mesh + gaussian splat and a local->camera pose (quaternion,
// translation, uniform scale); for multi-object requests also a combined,
// posed scene GLB/splat. $0.02 per request.
//
// Every response and all downloaded artifacts are cached under
// work/sam3d-cache/<hash>/ keyed by image + masks + params, so fitting and
// scene assembly can be reworked without re-hitting the API.
import crypto from "node:crypto";
import fs from "node:fs/promises";
import path from "node:path";
import { fal } from "@fal-ai/client";
import type {
  Sam33dObjectsInput,
  Sam33dObjectsOutput,
} from "@fal-ai/client/endpoints";
import type { ModelPose } from "@rle/shared";
import { requireEnv, workDir } from "./env";
import {
  cacheDirectory,
  contentKey,
  cachedArtifacts,
  validateGlb,
  type CacheOptions,
} from "./provider-cache";

const ENDPOINT = "fal-ai/sam-3/3d-objects";

let configured = false;
function ensureConfigured() {
  if (!configured) {
    fal.config({ credentials: requireEnv("FAL_KEY") });
    configured = true;
  }
}

export interface Sam3dRequest {
  imagePng: Buffer;
  /** one 8-bit PNG per object, white = object, same size as imagePng */
  maskPngs: Buffer[];
  seed?: number;
  /** bake a UV texture into the GLB instead of vertex colors */
  textured?: boolean;
}

export interface Sam3dObject {
  index: number;
  pose: ModelPose;
  /** local (unposed) GLB path in the cache dir */
  glb: string;
  /** gaussian splat PLY path in the cache dir, if returned per object */
  splat: string | null;
}

export interface Sam3dResult {
  requestId: string;
  cacheDir: string;
  objects: Sam3dObject[];
  /** combined posed scene files (multi-object requests only) */
  sceneGlb: string | null;
  sceneSplat: string | null;
}

interface FileRef {
  url: string;
  file_name?: string;
  content_type?: string;
}

async function download(url: string, dest: string) {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`download ${url}: HTTP ${res.status}`);
  await fs.writeFile(dest, Buffer.from(await res.arrayBuffer()));
}

function poseOf(meta: Sam33dObjectsOutput["metadata"][number]): ModelPose {
  // the typed schema declares nested arrays (batch dimension); accept both
  // [[x,y,z,w]] and [x,y,z,w]
  const flat = (v: unknown, n: number, what: string): number[] => {
    const arr =
      Array.isArray(v) && Array.isArray(v[0])
        ? (v as number[][])[0]
        : (v as number[]);
    if (
      !Array.isArray(arr) ||
      arr.length !== n ||
      arr.some((x) => typeof x !== "number" || !Number.isFinite(x))
    ) {
      throw new Error(
        `unexpected ${what} in SAM 3D metadata: ${JSON.stringify(v)}`,
      );
    }
    return arr;
  };
  return {
    rotation: flat(meta.rotation, 4, "rotation") as ModelPose["rotation"],
    translation: flat(
      meta.translation,
      3,
      "translation",
    ) as ModelPose["translation"],
    scale: flat(meta.scale, 3, "scale") as ModelPose["scale"],
    camera_pose: meta.camera_pose,
  };
}

export async function reconstruct3d(
  req: Sam3dRequest,
  options: CacheOptions = {},
): Promise<Sam3dResult> {
  if (!req.maskPngs.length)
    throw new Error("SAM 3D requires at least one mask");
  const params = {
    seed: req.seed ?? 42,
    export_textured_glb: req.textured ?? true,
    mask_count: req.maskPngs.length,
  };
  const hash = crypto.createHash("sha256").update(req.imagePng);
  for (const m of req.maskPngs) hash.update(m);
  hash.update(JSON.stringify(params));
  const key = hash.digest("hex").slice(0, 24);
  const directory = await cacheDirectory(
    path.join(options.workDirectory ?? workDir, "sam3d-cache"),
    contentKey([
      ENDPOINT,
      req.imagePng,
      ...req.maskPngs,
      JSON.stringify(params),
    ]),
    key,
  );
  const assemble = async (
    cacheDir: string,
    response: Sam33dObjectsOutput,
    requestId: string,
    fetchInto: (
      ref: FileRef | undefined,
      name: string,
    ) => Promise<string | null>,
  ): Promise<Sam3dResult> => {
    const perObjectGlbs =
      (response.individual_glbs as FileRef[] | undefined) ?? [];
    const perObjectSplats =
      (response.individual_splats as FileRef[] | undefined) ?? [];
    const multi = perObjectGlbs.length > 0;
    if (!multi && req.maskPngs.length > 1) {
      throw new Error(
        `SAM 3D returned no individual_glbs for a ${req.maskPngs.length}-mask request`,
      );
    }
    if (response.metadata.length !== req.maskPngs.length) {
      throw new Error(
        `SAM 3D metadata count ${response.metadata.length} != mask count ${req.maskPngs.length}`,
      );
    }

    const objects: Sam3dObject[] = [];
    for (let i = 0; i < req.maskPngs.length; i++) {
      const glbRef = multi
        ? perObjectGlbs[i]
        : (response.model_glb as FileRef | undefined);
      const glb = await fetchInto(glbRef, `object-${i}.glb`);
      if (!glb) throw new Error(`SAM 3D returned no GLB for object ${i}`);
      const splatRef = multi
        ? perObjectSplats[i]
        : (response.gaussian_splat as FileRef);
      const splat = await fetchInto(splatRef, `object-${i}.ply`);
      objects.push({
        index: i,
        pose: poseOf(response.metadata[i]!),
        glb,
        splat,
      });
    }

    return {
      requestId,
      cacheDir,
      objects,
      sceneGlb: multi
        ? await fetchInto(
            response.model_glb as FileRef | undefined,
            "scene.glb",
          )
        : null,
      sceneSplat: multi
        ? await fetchInto(response.gaussian_splat as FileRef, "scene.ply")
        : null,
    };
  };
  return cachedArtifacts(
    directory,
    async (cacheDir) => {
      const cached = JSON.parse(
        await fs.readFile(path.join(cacheDir, "response.json"), "utf8"),
      );
      if (
        cached.endpoint !== ENDPOINT ||
        typeof cached.request_id !== "string" ||
        !Array.isArray(cached.response?.metadata)
      ) {
        throw new Error("invalid SAM 3D response metadata");
      }
      return assemble(
        cacheDir,
        cached.response,
        cached.request_id,
        async (ref, name) => {
          if (!ref?.url) return null;
          const file = path.join(cacheDir, name);
          if (name.endsWith(".glb")) await validateGlb(file);
          else {
            const bytes = await fs.readFile(file);
            if (
              !bytes.toString("ascii", 0, 4).startsWith("ply") ||
              !bytes.includes(Buffer.from("end_header"))
            )
              throw new Error(`invalid PLY: ${file}`);
          }
          return file;
        },
      );
    },
    async (cacheDir) => {
      ensureConfigured();
      await fs.writeFile(path.join(cacheDir, "input.png"), req.imagePng);
      for (let i = 0; i < req.maskPngs.length; i++) {
        await fs.writeFile(
          path.join(cacheDir, `mask-${i}.png`),
          req.maskPngs[i]!,
        );
      }
      const describeError = (e: unknown): string => {
        const err = e as { status?: number; body?: unknown; message?: string };
        const body = err.body === undefined ? "" : JSON.stringify(err.body);
        return `fal ${err.status ?? "?"} ${err.message ?? String(e)} ${body}`.trim();
      };
      // uploads and the request itself are retried on rate limits / transient
      // server errors; anything else fails the asset
      const withRetry = async <T>(
        what: string,
        fn: () => Promise<T>,
      ): Promise<T> => {
        let delay = 5000;
        for (let attempt = 1; ; attempt++) {
          try {
            return await fn();
          } catch (e) {
            const status = (e as { status?: number }).status;
            const retryable =
              status === 429 || (status !== undefined && status >= 500);
            if (!retryable || attempt >= 6)
              throw new Error(`${what}: ${describeError(e)}`);
            console.warn(
              `${what}: ${describeError(e)} — retry ${attempt} in ${delay / 1000}s`,
            );
            await new Promise((r) => setTimeout(r, delay));
            delay = Math.min(delay * 2, 60000);
          }
        }
      };
      const upload = (buf: Buffer) =>
        withRetry("upload", () =>
          fal.storage.upload(
            new Blob([new Uint8Array(buf)], {
              type: "image/png",
            }) as unknown as File,
          ),
        );
      const input: Sam33dObjectsInput = {
        image_url: await upload(req.imagePng),
        mask_urls: [],
        seed: params.seed,
        export_textured_glb: params.export_textured_glb,
      };
      for (const m of req.maskPngs) input.mask_urls!.push(await upload(m));
      console.log(`SAM 3D: ${req.maskPngs.length} mask(s) -> ${ENDPOINT}`);
      // Submission is not idempotent: only uploads retry automatically. An
      // ambiguous provider failure remains in the cache for explicit recovery.
      const result = await fal.subscribe(ENDPOINT, { input, logs: false });
      const response = result.data;
      const requestId = result.requestId;
      await fs.writeFile(
        path.join(cacheDir, "response.json"),
        JSON.stringify(
          {
            endpoint: ENDPOINT,
            requested_at: new Date().toISOString(),
            params,
            request_id: requestId,
            response,
          },
          null,
          1,
        ),
      );
      await assemble(cacheDir, response, requestId, async (ref, name) => {
        if (!ref?.url) return null;
        const file = path.join(cacheDir, name);
        await download(ref.url, file);
        return file;
      });
    },
    options,
  );
}
