import type { AssetSceneryAnimation } from "../../shared/src/asset-gameplay.ts";
import type { ElementFx, Point } from "../../shared/src/level.ts";
import type { Vec3 } from "../../shared/src/scene.ts";

/** One-time authoring: an explicit frame and reviewed height replace screen-only placement. */
export function recoverSceneryAnimation(
  source: ElementFx,
  profile: { name: string; center_x: number; center_y: number },
  owner: { id: string; node: string; anchor: Vec3; resourceDirectory?: string },
  localize: (point: Vec3) => Vec3,
): AssetSceneryAnimation {
  const center: Point = [profile.center_x, profile.center_y];
  const sprite = source.sprite;
  const [x, y, z] = owner.anchor;
  if (
    profile.name !== sprite.profile_name ||
    !center.every(Number.isFinite) ||
    !owner.id.trim() ||
    !owner.node.trim() ||
    owner.anchor.length !== 3 ||
    !owner.anchor.every(Number.isFinite) ||
    z < 0 ||
    Math.abs(x - center[0] - sprite.position_x) > 1e-4 ||
    Math.abs(y - z - center[1] - sprite.position_y) > 1e-4
  )
    throw new Error(
      "Scenery recovery requires a matching profile and reviewed anchor preserving screen placement",
    );
  const point = (world: Vec3) => {
    const local = localize(world);
    if (local.length !== 3 || !local.every(Number.isFinite))
      throw new Error("Invalid scenery owner transform");
    return local;
  };
  return {
    id: owner.id,
    node: owner.node,
    anchor: point(owner.anchor),
    file: sprite.frame_profile_name,
    profile: sprite.profile_name,
    center,
    active: source.active,
    forceDisplay: source.force_display,
    shadow: source.blit_type !== 0,
    displayPolyline: source.display_polyline.map(([px, py]) => point([px, py + z, z])),
    ...(owner.resourceDirectory ? { resourceDirectory: owner.resourceDirectory } : {}),
  };
}
