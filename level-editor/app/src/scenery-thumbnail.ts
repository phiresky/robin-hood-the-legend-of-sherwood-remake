import type { GameplayAssetDescriptor } from "../../shared/src/asset-gameplay.ts";
import { safeLibraryPath } from "@rle/shared";
import { validateSceneryManifest } from "./scenery-manifest.ts";

/** Static palette artwork for an effect-only asset; never enters map bake geometry. */
export async function loadSceneryThumbnail(
  descriptor: GameplayAssetDescriptor,
  read: (name: string) => Promise<Uint8Array>,
): Promise<{ png: Uint8Array; legacy: boolean } | null> {
  if (!descriptor.parts.length || !descriptor.parts.every((part) => part.gameplay_only))
    return null;
  const animations = descriptor.gameplay?.animations ?? [];
  const animation = animations.find((entry) => entry.active) ?? animations[0];
  if (!animation?.resourceDirectory) return null;
  const directory = animation.resourceDirectory;
  if (!safeLibraryPath(directory)) throw new Error("Invalid scenery thumbnail resource directory");
  const files: Record<string, Uint8Array> = {};
  for (const pin of descriptor.resources ?? []) {
    if (!pin.path.startsWith(`${directory}/`)) continue;
    if (!safeLibraryPath(pin.path)) throw new Error(`Invalid scenery resource path: ${pin.path}`);
    const bytes = await read(pin.path);
    const hash = [...new Uint8Array(await crypto.subtle.digest("SHA-256", new Uint8Array(bytes)))]
      .map((byte) => byte.toString(16).padStart(2, "0"))
      .join("");
    if (hash !== pin.sha256) throw new Error(`Scenery resource changed: ${pin.path}`);
    files[pin.path.slice(directory.length + 1)] = bytes;
  }
  if (!files["manifest.json"]) throw new Error("Missing pinned scenery thumbnail manifest");
  const data: unknown = JSON.parse(new TextDecoder().decode(files["manifest.json"]));
  const manifest = await validateSceneryManifest(data, files);
  const profile = manifest.profiles.find((profile) => profile.name === animation.profile);
  if (
    !profile ||
    profile.center_x !== animation.center[0] ||
    profile.center_y !== animation.center[1]
  )
    throw new Error(`Missing profile or changed sprite center: ${animation.profile}`);
  return {
    png: files[profile.preview]!,
    legacy: (data as { pixel_format: string }).pixel_format === "legacy_color_keys",
  };
}
