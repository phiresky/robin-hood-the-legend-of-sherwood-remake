import fs from "node:fs/promises";
import path from "node:path";
import { createHash } from "node:crypto";
import { pathToFileURL } from "node:url";
import {
  parseProjectionAssetIndex,
  parseProjectionAssetDescriptor,
} from "../../shared/src/validation.ts";
import {
  validateAssetGameplay,
  type AssetWalkableSurface,
  type AssetGameplay,
  type GameplayAssetDescriptor,
} from "../../shared/src/asset-gameplay.ts";

export interface SurfaceJumpEdit {
  asset: string;
  surface: string;
  /** Bind a reviewed edit to the descriptor that was inspected. */
  descriptorSha256: string;
  rules: NonNullable<AssetWalkableSurface["jump"]>;
}
export interface GameplayEdit {
  asset: string;
  descriptorSha256: string;
  /** Geometry-derived definitions are also bound to the model used for review. */
  modelSha256?: string;
  gameplay: AssetGameplay;
}
export interface SurfacePrecisionEdit {
  asset: string;
  surface: string;
  descriptorSha256: string;
  preserveMovementPrecision: boolean;
}
const hash = (bytes: string) => createHash("sha256").update(bytes).digest("hex");
const encode = (value: unknown) => JSON.stringify(value) + "\n";

/** Install reviewed gameplay without changing models or scene placements. */
export async function configureAssetGameplay(
  library: string,
  edits: (SurfaceJumpEdit | SurfacePrecisionEdit | GameplayEdit)[],
  output: string,
  apply = false,
) {
  library = path.resolve(library);
  output = path.resolve(output);
  if (output === library || output.startsWith(library + path.sep))
    throw new Error("Backup must be outside the library");
  const unique = new Set(
    edits.map((edit) => `${edit.asset}/${"surface" in edit ? edit.surface : "$gameplay"}`),
  );
  if (!edits.length || unique.size !== edits.length)
    throw new Error("Surface edits must be nonempty and unique");
  const indexFile = "3d-assets/index.json";
  const indexBefore = await fs.readFile(path.join(library, indexFile), "utf8");
  const index = parseProjectionAssetIndex(JSON.parse(indexBefore));
  const changes: { file: string; before: string; after: string }[] = [];
  const reviewedModels = new Map<string, string>();
  const pins = new Map<string, { before: string; after: string }>();
  const published: string[] = [];
  for (const asset of new Set(edits.map((edit) => edit.asset))) {
    const entry = index.find((entry) => entry.id === asset);
    if (!entry) throw new Error(`Unknown asset ${asset}`);
    const file = "3d-assets/" + entry.descriptor;
    const before = await fs.readFile(path.join(library, file), "utf8");
    const digest = hash(before);
    const selected = edits.filter((edit) => edit.asset === asset);
    if (
      entry.descriptor_sha256 !== digest ||
      selected.some((edit) => edit.descriptorSha256 !== digest)
    )
      throw new Error(`Stale reviewed descriptor: ${asset}`);
    const raw = JSON.parse(before);
    const descriptor: GameplayAssetDescriptor = parseProjectionAssetDescriptor(raw);
    const replacements = selected.filter((edit) => "gameplay" in edit);
    if (replacements.length && selected.length !== 1)
      throw new Error(`Conflicting gameplay edits: ${asset}`);
    const gameplay = structuredClone(replacements[0]?.gameplay ?? descriptor.gameplay);
    if (!gameplay) throw new Error(`Missing gameplay: ${asset}`);
    const reviewedModel = replacements[0]?.modelSha256;
    if (reviewedModel !== undefined) {
      if (!/^[0-9a-f]{64}$/.test(reviewedModel))
        throw new Error(`Invalid reviewed model digest: ${asset}`);
      const modelPath = path.join(library, "3d-assets", entry.model);
      const model = await fs.readFile(modelPath);
      if (createHash("sha256").update(model).digest("hex") !== reviewedModel)
        throw new Error(`Stale reviewed model: ${asset}`);
      reviewedModels.set(modelPath, reviewedModel);
    }
    if (gameplay.spline?.modelSha256) {
      const model = await fs.readFile(path.join(library, "3d-assets", entry.model));
      if (createHash("sha256").update(model).digest("hex") !== gameplay.spline.modelSha256)
        throw new Error(`Stale spline model calibration: ${asset}`);
    }
    for (const edit of selected) {
      if (!("surface" in edit)) continue;
      const surface = gameplay.surfaces.find((surface) => surface.id === edit.surface);
      if (!surface) throw new Error(`Missing surface: ${asset}/${edit.surface}`);
      if ("preserveMovementPrecision" in edit) {
        if (typeof edit.preserveMovementPrecision !== "boolean")
          throw new Error(`Invalid surface precision edit: ${asset}/${edit.surface}`);
        surface.preserveMovementPrecision = edit.preserveMovementPrecision;
      } else surface.jump = structuredClone(edit.rules);
    }
    validateAssetGameplay(gameplay, descriptor);
    const next = { ...raw, gameplay };
    const after = encode(next);
    changes.push({ file, before, after });
    pins.set(file, { before: digest, after: hash(after) });
    entry.editor = parseProjectionAssetDescriptor(next);
    entry.descriptor_sha256 = hash(after);
    published.push(asset);
  }
  const scenes: string[] = [];
  for (const name of await fs.readdir(path.join(library, "scenes"))) {
    if (!name.endsWith(".rhlos-map.json")) continue;
    const file = "scenes/" + name;
    const before = await fs.readFile(path.join(library, file), "utf8");
    const document = JSON.parse(before) as {
      assetSources?: { descriptor?: string; descriptor_sha256?: string }[];
      sceneAssets?: { descriptor?: string; descriptor_sha256?: string }[];
    };
    let changed = false;
    for (const ref of [...(document.assetSources ?? []), ...(document.sceneAssets ?? [])]) {
      const pin = ref.descriptor && pins.get(ref.descriptor);
      if (!pin) continue;
      if (ref.descriptor_sha256 !== pin.before)
        throw new Error(`Stale scene descriptor: ${file}/${ref.descriptor}`);
      ref.descriptor_sha256 = pin.after;
      changed = true;
    }
    if (changed) {
      let after = before;
      for (const pin of pins.values())
        after = after.replaceAll(JSON.stringify(pin.before), JSON.stringify(pin.after));
      if (JSON.stringify(JSON.parse(after)) !== JSON.stringify(document))
        throw new Error(`Descriptor hash occurs outside scene references: ${file}`);
      changes.push({ file, before, after });
      scenes.push(name);
    }
  }
  const nextIndex = { ...JSON.parse(indexBefore), assets: index };
  parseProjectionAssetIndex(nextIndex);
  changes.push({ file: indexFile, before: indexBefore, after: encode(nextIndex) });
  await fs.mkdir(output, { recursive: false });
  for (const change of changes)
    for (const [folder, bytes] of [
      ["before", change.before],
      ["after", change.after],
    ] as const) {
      const target = path.join(output, folder, change.file);
      await fs.mkdir(path.dirname(target), { recursive: true });
      await fs.writeFile(target, bytes, { flag: "wx" });
    }
  const report = {
    published,
    surfaces: edits.filter((edit) => "surface" in edit).length,
    scenes,
    applied: false,
  };
  await fs.writeFile(path.join(output, "report.json"), encode(report));
  if (apply) {
    for (const [modelPath, digest] of reviewedModels)
      if (
        createHash("sha256")
          .update(await fs.readFile(modelPath))
          .digest("hex") !== digest
      )
        throw new Error(`Reviewed model changed during publication: ${modelPath}`);
    for (const change of changes)
      if ((await fs.readFile(path.join(library, change.file), "utf8")) !== change.before)
        throw new Error(`Library changed: ${change.file}`);
    const installed: typeof changes = [];
    const temporaries = new Set<string>();
    try {
      for (const change of changes) {
        const target = path.join(library, change.file);
        await fs.writeFile(target + ".surface-jumps.tmp", change.after, { flag: "wx" });
        temporaries.add(target + ".surface-jumps.tmp");
        await fs.rename(target + ".surface-jumps.tmp", target);
        temporaries.delete(target + ".surface-jumps.tmp");
        installed.push(change);
      }
    } catch (error) {
      for (const change of installed.reverse())
        await fs.writeFile(path.join(library, change.file), change.before);
      for (const temporary of temporaries) await fs.unlink(temporary);
      throw error;
    }
    report.applied = true;
    await fs.writeFile(path.join(output, "report.json"), encode(report));
  }
  return report;
}

export const configureSurfaceJumps = configureAssetGameplay;

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const [library, configuration, output, mode] = process.argv.slice(2);
  if (!library || !configuration || !output || (mode !== undefined && mode !== "--apply"))
    throw new Error(
      "Usage: configure-surface-jumps.ts library edits.json new-backup-directory [--apply]",
    );
  const edits = JSON.parse(await fs.readFile(configuration, "utf8")) as (
    | SurfaceJumpEdit
    | SurfacePrecisionEdit
    | GameplayEdit
  )[];
  console.log(
    JSON.stringify(await configureSurfaceJumps(library, edits, output, mode === "--apply")),
  );
}
