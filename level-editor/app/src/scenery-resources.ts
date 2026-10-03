import { safeLibraryPath, type ProjectionAssetDescriptor } from "@rle/shared";
import type { GameplayAssetDescriptor } from "../../shared/src/asset-gameplay.ts";
import type { CompiledMap } from "./map-compile.ts";
import { validateSceneryManifest } from "./scenery-manifest.ts";

export type SceneryResources = Record<string, Uint8Array>;

async function sha256(bytes: Uint8Array): Promise<string> {
  return [...new Uint8Array(await crypto.subtle.digest("SHA-256", new Uint8Array(bytes)))]
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
}

/** Resource inputs are pinned library files, never runtime level data. */
export async function collectSceneryResources(
  compiled: CompiledMap,
  assets: ReadonlyMap<string, ProjectionAssetDescriptor>,
  read: (path: string) => Promise<Uint8Array>,
  progress: (completed: number, total: number) => void = () => {},
): Promise<SceneryResources> {
  const animations = compiled.descriptor.asset_geometry?.animations ?? [];
  const files: SceneryResources = {};
  const banks = new Map<string, { directory: string; pins: Map<string, string> }>();
  const failed = new Set<string>();
  const sources = compiled.scenerySources.map(({ assetId, animationId }) => {
    const asset: GameplayAssetDescriptor | undefined = assets.get(assetId);
    const animation = asset?.gameplay?.animations?.find((entry) => entry.id === animationId);
    if (!asset || !animation)
      throw new Error(`Compiled scenery source is unavailable: ${assetId}/${animationId}`);
    const bank = animation.file.replace(/\.rhs$/i, "");
    const directory = animation.resourceDirectory;
    const pins = new Map(
      (asset.resources ?? [])
        .filter((pin) => directory && pin.path.startsWith(`${directory}/`))
        .map((pin) => [pin.path.slice(directory!.length + 1), pin.sha256]),
    );
    const identity = directory
      ? JSON.stringify([...pins].sort(([a], [b]) => a.localeCompare(b, "en")))
      : "shared";
    return { animation, bank, directory, pins, identity };
  });
  if (sources.length !== animations.length)
    throw new Error("Compiled scenery bindings do not match emitted animations");
  const variants = new Map<string, Set<string>>();
  const reserved = new Set(sources.map((source) => source.bank.toLowerCase()));
  for (const source of sources) {
    const key = source.bank.toLowerCase();
    if (!variants.has(key)) variants.set(key, new Set());
    variants.get(key)!.add(JSON.stringify([source.bank, source.identity]));
  }
  const aliases = new Map<string, string>();
  for (const source of sources) {
    if (!source.directory || variants.get(source.bank.toLowerCase())!.size < 2) continue;
    const key = JSON.stringify([source.bank, source.identity]);
    let alias = aliases.get(key);
    if (!alias) {
      const base = `editor-fx-${await sha256(new TextEncoder().encode(key))}`;
      alias = base;
      for (let suffix = 1; reserved.has(alias.toLowerCase()); suffix++) alias = `${base}-${suffix}`;
      aliases.set(key, alias);
      reserved.add(alias.toLowerCase());
    }
    source.bank = alias;
  }
  const warn = (bank: string, reason: string) => {
    failed.add(bank);
    compiled.warnings.push(`Scenery bank ${bank} omitted: ${reason}`);
  };
  for (const { bank, directory, pins } of sources) {
    if (!directory) continue;
    if (!banks.has(bank)) banks.set(bank, { directory, pins });
  }
  let completed = 0;
  const total = [...banks.values()].reduce((sum, bank) => sum + bank.pins.size, 0);
  progress(completed, total);
  for (const [bank, { directory, pins }] of banks) {
    if (failed.has(bank)) continue;
    const pending: SceneryResources = {};
    let reportingProgress = false;
    try {
      if (
        !safeLibraryPath(directory) ||
        !/^[a-zA-Z0-9_-]+$/.test(bank) ||
        !pins.has("manifest.json")
      )
        throw new Error("missing pinned manifest or invalid resource path");
      for (const [relative, expected] of pins) {
        if (!safeLibraryPath(relative)) throw new Error(`invalid frame path ${relative}`);
        const bytes = await read(`${directory}/${relative}`);
        const hash = await sha256(bytes);
        if (hash !== expected) throw new Error(`resource changed: ${relative}`);
        pending[relative] = bytes;
        reportingProgress = true;
        progress(++completed, total);
        reportingProgress = false;
      }
      const manifest = await validateSceneryManifest(
        JSON.parse(new TextDecoder().decode(pending["manifest.json"])),
        pending,
        () => {
          reportingProgress = true;
          progress(completed, total);
          reportingProgress = false;
        },
      );
      for (const { animation, bank: sourceBank } of sources) {
        if (sourceBank !== bank) continue;
        const profile = manifest.profiles.find((profile) => profile.name === animation.profile);
        if (
          !profile ||
          profile.center_x !== animation.center[0] ||
          profile.center_y !== animation.center[1]
        )
          throw new Error(`missing profile or changed sprite center: ${animation.profile}`);
      }
      for (const [relative, bytes] of Object.entries(pending))
        files[`Data/Animations/Day/${bank}.rhs.d/${relative}`] = bytes;
    } catch (error) {
      if (reportingProgress) throw error;
      warn(bank, error instanceof Error ? error.message : String(error));
    }
  }
  for (const [index, source] of sources.entries())
    animations[index]!.sprite.frame_profile_name = source.bank;
  if (failed.size && compiled.descriptor.asset_geometry) {
    compiled.descriptor.asset_geometry.animations = animations.filter(
      (_, index) => !failed.has(sources[index]!.bank),
    );
    compiled.scenerySources = compiled.scenerySources.filter(
      (_, index) => !failed.has(sources[index]!.bank),
    );
  }
  return files;
}
