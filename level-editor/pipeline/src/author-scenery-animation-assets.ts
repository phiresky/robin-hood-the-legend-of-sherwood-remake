import fs from "node:fs/promises";
import path from "node:path";
import { createHash } from "node:crypto";
import { parseArgs } from "node:util";
import { pathToFileURL } from "node:url";
import { safeLibraryPath } from "../../shared/src/projection-assets.ts";
import type { ExternalAssetSource } from "../../shared/src/projection-assets.ts";
import type { Level3DObject } from "../../shared/src/level3d.ts";
import { validateSceneryManifest } from "../../app/src/scenery-manifest.ts";
import { authorSceneryAnimationAsset } from "./author-scenery-animation-asset.ts";

type AuthorOptions = Parameters<typeof authorSceneryAnimationAsset>[1];
export interface SceneryAssetRecipe {
  version: 1;
  entries: (AuthorOptions & {
    animations: Parameters<typeof authorSceneryAnimationAsset>[0];
  })[];
}
const hash = (bytes: Uint8Array | string) => createHash("sha256").update(bytes).digest("hex");

/** Validate every definition and resource before creating a fresh output library. */
export async function writeSceneryAnimationAssets(
  recipe: SceneryAssetRecipe,
  readResource: (name: string) => Promise<Uint8Array>,
  output: string,
) {
  if (recipe?.version !== 1 || !Array.isArray(recipe.entries) || !recipe.entries.length)
    throw new Error("Scenery recipe requires version 1 and nonempty entries");
  const files = new Map<string, Uint8Array>();
  const assetSources: ExternalAssetSource[] = [];
  const objects: Level3DObject[] = [];
  const ids = new Set<string>();
  const add = (name: string, bytes: Uint8Array) => {
    if (files.has(name) && hash(files.get(name)!) !== hash(bytes))
      throw new Error(`Conflicting output file: ${name}`);
    files.set(name, bytes);
  };
  const json = (name: string, value: unknown) =>
    add(name, new TextEncoder().encode(JSON.stringify(value, null, 2) + "\n"));
  for (const entry of recipe.entries) {
    if (!entry || typeof entry.map !== "string" || !entry.map.trim() || ids.has(entry.id))
      throw new Error("Scenery recipe has an invalid or duplicate asset");
    const asset = await authorSceneryAnimationAsset(entry.animations, entry);
    ids.add(entry.id);
    const resources = new Map<string, Uint8Array>();
    for (const pin of asset.descriptor.resources ?? []) {
      if (!safeLibraryPath(pin.path) || !/^[a-f0-9]{64}$/.test(pin.sha256))
        throw new Error(`Invalid scenery resource pin: ${pin.path}`);
      const bytes = files.get(pin.path) ?? (await readResource(pin.path));
      if (hash(bytes) !== pin.sha256) throw new Error(`Scenery resource changed: ${pin.path}`);
      resources.set(pin.path, bytes);
      add(pin.path, bytes);
    }
    const banks = new Map<string, Awaited<ReturnType<typeof validateSceneryManifest>>>();
    for (const animation of asset.descriptor.gameplay!.animations!) {
      const directory = animation.resourceDirectory;
      if (!directory) continue;
      let manifest = banks.get(directory);
      if (!manifest) {
        const manifestBytes = resources.get(`${directory}/manifest.json`);
        if (!manifestBytes) throw new Error(`Missing pinned manifest: ${directory}`);
        const frames = Object.fromEntries(
          [...resources]
            .filter(([name]) => name.startsWith(`${directory}/`))
            .map(([name, bytes]) => [name.slice(directory.length + 1), bytes]),
        );
        manifest = await validateSceneryManifest(
          JSON.parse(new TextDecoder().decode(manifestBytes)),
          frames,
        );
        banks.set(directory, manifest);
      }
      const profile = manifest.profiles.find((profile) => profile.name === animation.profile);
      if (
        !profile ||
        profile.center_x !== animation.center[0] ||
        profile.center_y !== animation.center[1]
      )
        throw new Error(`Missing profile or changed sprite center: ${entry.id}/${animation.id}`);
    }
    const root = `3d-assets/${entry.id}`;
    json(`${root}/asset.json`, asset.descriptor);
    add(`${root}/model.glb`, asset.model);
    add(`${root}/lossy.glb`, asset.model);
    json(`${root}/lossy.glb.receipt.json`, {
      source: hash(asset.model),
      output: hash(asset.model),
      producer: "author-scenery-animation-assets",
      mode: "identity-gameplay-frame",
      version: 1,
    });
    assetSources.push({
      id: entry.id,
      descriptor: `${root}/asset.json`,
      model: `${root}/model.glb`,
      descriptor_sha256: hash(files.get(`${root}/asset.json`)!),
      model_sha256: hash(asset.model),
      model_scene: "default",
      resources: structuredClone(asset.descriptor.resources ?? []),
    });
    objects.push(asset.placement);
  }
  const manifest = { assetSources, objects };
  json("scenery-animation-assets.json", manifest);
  // Refuse existing output directories rather than modifying another library revision.
  await fs.mkdir(output);
  for (const [name, bytes] of files) {
    const target = path.join(output, name);
    await fs.mkdir(path.dirname(target), { recursive: true });
    await fs.writeFile(target, bytes, { flag: "wx" });
  }
  return manifest;
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const { values } = parseArgs({
    options: {
      recipe: { type: "string" },
      library: { type: "string" },
      out: { type: "string" },
    },
  });
  if (!values.recipe || !values.out)
    throw new Error(
      "Usage: --recipe <local-effects.json> --out <new-library-directory> [--library <pinned-resource-root>]",
    );
  const recipe: SceneryAssetRecipe = JSON.parse(await fs.readFile(values.recipe, "utf8"));
  const result = await writeSceneryAnimationAssets(
    recipe,
    async (name) => {
      if (!values.library) throw new Error("Pinned sprite resources require --library");
      return fs.readFile(path.join(values.library, name));
    },
    values.out,
  );
  console.log(JSON.stringify({ assets: result.assetSources.length, out: values.out }));
}
