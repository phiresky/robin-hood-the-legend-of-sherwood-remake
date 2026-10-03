import type { AssetSceneryAnimation, CompiledAssetGeometry } from "./asset-gameplay.ts";
import type { Vec3 } from "./scene.ts";

/** Transform the billboard's anchor, keeping its raster center in camera coordinates. */
export function compileSceneryAnimation(
  animation: AssetSceneryAnimation,
  transform: (node: string, point: Vec3) => Vec3,
): NonNullable<CompiledAssetGeometry["animations"]>[number] {
  const quantize = (value: number, minimum: number, maximum: number) => {
    const result = Math.round(value);
    if (!Number.isFinite(result) || result < minimum || result > maximum)
      throw new Error(`Scenery animation ${animation.id}: coordinate outside runtime range`);
    return result;
  };
  const signed = (value: number) => quantize(value, -32768, 32767);
  const [x, y, z] = transform(animation.node, animation.anchor);
  return {
    sprite: {
      frame_profile_name: animation.file,
      profile_name: animation.profile,
      position_x: signed(x - animation.center[0]),
      position_y: signed(y - z - animation.center[1]),
      elevation: quantize(z, 0, 65535),
    },
    blit_type: Number(animation.shadow),
    active: animation.active,
    force_display: animation.forceDisplay,
    display_polyline: animation.displayPolyline.map((point) => {
      const [px, py, pz] = transform(animation.node, point);
      return [signed(px), signed(py - pz)];
    }),
  };
}
