import type { AssetSceneryAnimation, CompiledAssetGeometry } from "./asset-gameplay.ts";
import type { Vec3 } from "./scene.ts";
import type { Point } from "./level.ts";

/** Transform the billboard's anchor, keeping its raster center in camera coordinates. */
export function compileSceneryAnimation(
  animation: AssetSceneryAnimation,
  transform: (node: string, point: Vec3) => Vec3,
  warn?: (message: string) => void,
): NonNullable<CompiledAssetGeometry["animations"]>[number] {
  const quantize = (value: number, minimum: number, maximum: number) => {
    const result = Math.round(value);
    if (!Number.isFinite(result) || result < minimum || result > maximum)
      throw new Error(`Scenery animation ${animation.id}: coordinate outside runtime range`);
    return result;
  };
  const signed = (value: number) => quantize(value, -32768, 32767);
  const [x, y, z] = transform(animation.node, animation.anchor);
  const elevation = quantize(z, 0, 65535);
  const projected = animation.displayPolyline.map((point): Point => {
    const [px, py, pz] = transform(animation.node, point);
    return [signed(px), signed(py - pz)];
  });
  // Quantization may collapse consecutive vertices; zero-length segments cannot
  // bracket an actor and carry no additional boundary shape.
  const displayPolyline = projected.filter(
    (point, index) =>
      index === 0 || point[0] !== projected[index - 1]![0] || point[1] !== projected[index - 1]![1],
  );
  // The runtime brackets actors between consecutive left-to-right vertices.
  // Placement can reverse that order without changing the boundary itself.
  if (
    displayPolyline.length > 1 &&
    displayPolyline[0]![0] > displayPolyline.at(-1)![0] &&
    displayPolyline.every(
      (point, index) => index === 0 || point[0] <= displayPolyline[index - 1]![0],
    )
  )
    displayPolyline.reverse();
  if (
    elevation > 0 &&
    displayPolyline.some((point, index) => index > 0 && point[0] < displayPolyline[index - 1]![0])
  )
    warn?.(
      "drawing boundary folds back after placement; actor ordering may be incorrect. Adjust the asset boundary for this orientation",
    );
  else if (
    elevation > 0 &&
    displayPolyline.some((point, index) => index > 0 && point[0] === displayPolyline[index - 1]![0])
  )
    warn?.(
      "drawing boundary has a vertical segment after placement; actor ordering on that column may be incorrect. Adjust the asset boundary for this orientation",
    );
  return {
    sprite: {
      // Runtime resolution appends the extension after choosing the ambience directory.
      frame_profile_name: animation.file.replace(/\.rhs$/i, ""),
      profile_name: animation.profile,
      position_x: signed(x - animation.center[0]),
      position_y: signed(y - z - animation.center[1]),
      elevation,
    },
    blit_type: Number(animation.shadow),
    active: animation.active,
    force_display: animation.forceDisplay,
    display_polyline: displayPolyline,
  };
}
