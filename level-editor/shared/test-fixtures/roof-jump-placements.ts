import { groupCentroid, groupParts, type Level3D } from "../src/level3d.ts";

/** Move the complete two-house assembly, then add an independent copy. */
export function roofJumpPlacement(source: Level3D, rotation: number, height: number): Level3D {
  const document = structuredClone(source);
  document.groups = [];
  document.objects = [];
  const angle = (rotation * Math.PI) / 180;
  const sine = Math.sin((source.camera.elevation_deg * Math.PI) / 180);
  for (const copy of [0, 1]) {
    for (const group of source.groups) {
      const parts = groupParts(source, group.id);
      const [px, py] = groupCentroid(parts);
      const x = px + group.transform.dx - 600;
      const y = (py + group.transform.dy - 900) / sine;
      const id = `${copy}/${group.id}`;
      document.groups.push({
        ...structuredClone(group),
        id,
        transform: {
          dx: 600 + 700 * copy + x * Math.cos(angle) - y * Math.sin(angle) - px,
          dy: 900 + 200 * copy + (x * Math.sin(angle) + y * Math.cos(angle)) * sine - py,
          dz: group.transform.dz + height,
          rot_deg: group.transform.rot_deg + rotation,
        },
      });
      document.objects.push(
        ...parts.map((part) => ({ ...structuredClone(part), id: `${copy}/${part.id}`, group: id })),
      );
    }
  }
  return document;
}
