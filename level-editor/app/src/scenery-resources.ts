import { safeLibraryPath, type ProjectionAssetDescriptor } from "@rle/shared";
import type { GameplayAssetDescriptor } from "../../shared/src/asset-gameplay.ts";
import type { CompiledMap } from "./map-compile.ts";
import { validateSceneryManifest } from "./scenery-manifest.ts";

export type SceneryResources = Record<string, Uint8Array>;

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
    return { asset, animation };
  });
  const warn = (bank: string, reason: string) => {
    failed.add(bank);
    compiled.warnings.push(`Scenery bank ${bank} omitted: ${reason}`);
  };
  for (const { asset, animation } of sources) {
    const bank = animation.file.replace(/\.rhs$/i, "");
    if (!animations.some((placed) => placed.sprite.frame_profile_name === bank)) continue;
    const directory = animation.resourceDirectory;
    if (!directory) continue;
    const pins = new Map(
      (asset.resources ?? [])
        .filter((pin) => pin.path.startsWith(`${directory}/`))
        .map((pin) => [pin.path.slice(directory.length + 1), pin.sha256]),
    );
    const previous = banks.get(bank);
    if (
      previous &&
      (previous.pins.size !== pins.size ||
        [...pins].some(([path, hash]) => previous.pins.get(path) !== hash))
    )
      warn(bank, "placed assets disagree on the pinned sprite resources.");
    else if (!previous) banks.set(bank, { directory, pins });
    for (const other of banks.keys())
      if (other !== bank && other.toLowerCase() === bank.toLowerCase()) {
        warn(bank, "sprite bank name differs only by case from another placed bank.");
        warn(other, "sprite bank name differs only by case from another placed bank.");
      }
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
        const hash = [
          ...new Uint8Array(await crypto.subtle.digest("SHA-256", new Uint8Array(bytes))),
        ]
          .map((byte) => byte.toString(16).padStart(2, "0"))
          .join("");
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
      for (const { animation } of sources) {
        if (animation.file.replace(/\.rhs$/i, "") !== bank) continue;
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
  if (failed.size && compiled.descriptor.asset_geometry)
    compiled.descriptor.asset_geometry.animations = animations.filter(
      (animation) => !failed.has(animation.sprite.frame_profile_name),
    );
  return files;
}
