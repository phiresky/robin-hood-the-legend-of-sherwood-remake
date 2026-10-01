import fs from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { createHash } from "node:crypto";
import { NodeIO } from "@gltf-transform/core";
import { ALL_EXTENSIONS } from "@gltf-transform/extensions";
import { mat4, vec3 } from "gl-matrix";
import earcut, { flatten } from "earcut";
import ClipperLib from "clipper-lib";
import polygonClipping, { type MultiPolygon, type Polygon } from "polygon-clipping";
import { fixedPolygonBoolean } from "../../shared/src/fixed-polygon-boolean.ts";
import {
  heightPlane,
  planeHeight,
  clipHeight,
  type HeightPlane,
} from "../../shared/src/gameplay-plane.ts";
import { simplifyMotionRing } from "../../shared/src/motion-quantization.ts";
import type { Vec3 } from "../../shared/src/scene.ts";
import type { Point } from "../../shared/src/level.ts";
import type { AssetGameplay } from "../../shared/src/asset-gameplay.ts";

type Triangle = [Vec3, Vec3, Vec3];
type Policy = { material: number; opaque: boolean; walkwayHeight: number };
const area = (ring: Point[]) =>
  Math.abs(
    ring.reduce((s, a, i) => {
      const b = ring[(i + 1) % ring.length]!;
      return s + a[0] * b[1] - a[1] * b[0];
    }, 0),
  ) / 2;
const clean = (polygon: Polygon): Point[][] =>
  polygon.map((r) =>
    simplifyMotionRing(
      ClipperLib.Clipper.CleanPolygon(
        r.map(([x, y]) => ({ X: Math.round(x * 1024), Y: Math.round(y * 1024) })),
        0.02 * 1024,
      ).map(({ X, Y }): Point => [X / 1024, Y / 1024]),
    ),
  );

/** Dissolve coplanar mesh faces into compact grounded solids and a deliberately selected walkway. */
export function authorWallWalkway(
  triangles: Triangle[],
  node: string,
  policy: Policy,
  progress: (message: string) => void = () => {},
): AssetGameplay {
  if (!triangles.length) throw new Error("Wall model has no triangles");
  if (
    !Number.isFinite(policy.walkwayHeight) ||
    !Number.isInteger(policy.material) ||
    policy.material < 0 ||
    policy.material > 9 ||
    typeof policy.opaque !== "boolean"
  )
    throw new Error("Wall walkway needs a finite selected height and valid physical policy");
  const min: Vec3 = [Infinity, Infinity, Infinity],
    max: Vec3 = [-Infinity, -Infinity, -Infinity];
  for (const triangle of triangles)
    for (const p of triangle)
      for (let i = 0; i < 3; i++) {
        min[i] = Math.min(min[i]!, p[i]!);
        max[i] = Math.max(max[i]!, p[i]!);
      }
  const cleanCaps = (polygon: Polygon) =>
    clean(polygon).map((r) =>
      r.map(
        (p) =>
          p.map((n, i) =>
            Math.abs(n - min[i]!) < 0.02 ? min[i]! : Math.abs(n - max[i]!) < 0.02 ? max[i]! : n,
          ) as Point,
      ),
    );
  const groups = new Map<string, { plane: HeightPlane; polygons: MultiPolygon }>();
  for (const triangle of triangles) {
    if (
      area(triangle.map((p) => [p[0], p[1]])) < 1e-4 ||
      triangle.every((p) => p[2] <= min[2] + 1e-4)
    )
      continue;
    const plane = heightPlane(triangle);
    // Near-vertical texture faces have negligible plan coverage and unstable cap planes.
    if (Math.hypot(plane[0], plane[1]) > 100) continue;
    const canonical: HeightPlane =
      Math.hypot(plane[0], plane[1]) < 0.001
        ? [0, 0, Math.round((triangle.reduce((s, p) => s + p[2], 0) / 3) * 100) / 100]
        : (plane.map(
            (n, i) => Math.round(n * (i === 2 ? 100 : 1e4)) / (i === 2 ? 100 : 1e4),
          ) as HeightPlane);
    const key = canonical.join(",");
    let group = groups.get(key);
    if (!group) {
      group = { plane: canonical, polygons: [] };
      groups.set(key, group);
    }
    group.polygons.push([triangle.map((p) => [p[0], p[1]])]);
  }
  progress(`Dissolving ${groups.size} cap planes`);
  const regions = [...groups.values()].map((g, i) => {
    progress(`Plane ${i + 1}/${groups.size}: ${g.polygons.length} faces`);
    return {
      plane: g.plane,
      polygons: polygonClipping.union(g.polygons[0]!, ...g.polygons.slice(1)),
    };
  });
  progress("Clipping the selected walkway");
  const candidates = regions.filter(
    (r) =>
      Math.hypot(r.plane[0], r.plane[1]) < 1e-5 &&
      Math.abs(r.plane[2] - policy.walkwayHeight) < 0.05,
  );
  if (!candidates.length)
    throw new Error(`No horizontal walkway near scene height ${policy.walkwayHeight}`);
  const deck = candidates.reduce((a, b) =>
    a.polygons.reduce((s, p) => s + area(p[0]!), 0) >
    b.polygons.reduce((s, p) => s + area(p[0]!), 0)
      ? a
      : b,
  );
  const bounds: Point[] = [
    [min[0] - 1, min[1] - 1],
    [max[0] + 1, min[1] - 1],
    [max[0] + 1, max[1] + 1],
    [min[0] - 1, max[1] + 1],
  ];
  let walkable = deck.polygons;
  for (const region of regions) {
    const above: HeightPlane = [
      region.plane[0] - deck.plane[0],
      region.plane[1] - deck.plane[1],
      region.plane[2] - deck.plane[2] - 0.001,
    ];
    if (bounds.every((p) => planeHeight(above, p) <= 0)) continue;
    const clip = clipHeight(bounds, above);
    if (clip.length < 3 || area(clip) < 1e-5) continue;
    const blocks = fixedPolygonBoolean("intersection", region.polygons, [[clip]], 1024);
    if (blocks.length) walkable = fixedPolygonBoolean("difference", walkable, [blocks], 1024);
  }
  if (!walkable.length) throw new Error("Selected walkway is entirely covered by higher geometry");
  const sine = Math.sin((35 * Math.PI) / 180),
    cosine = Math.cos((35 * Math.PI) / 180);
  const gameplay: AssetGameplay = {
    version: 1,
    collision: "none",
    doors: [],
    surfaces: [],
    volumes: [],
    spline: {
      bounds: { min, max },
      frames: { [node]: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1] },
    },
    draft: {
      issues: [
        "Spline solids follow the model's upper surfaces and extend to its base; undercut openings require authored volumes.",
      ],
    },
  };
  for (const region of regions)
    for (const raw of region.polygons) {
      const cleaned = cleanCaps(raw);
      if (!cleaned[0] || cleaned[0].length < 3 || area(cleaned[0]) <= 1e-4) continue;
      const rings = [
        cleaned[0],
        ...cleaned.slice(1).filter((r) => r.length >= 3 && area(r) > 1e-4),
      ];
      // Holes require separate prisms. Simple polygons can retain their dissolved outline.
      const flat = flatten(rings),
        indices = earcut(flat.vertices, flat.holes, 2);
      const polygons: Point[][] =
        rings.length === 1
          ? [rings[0]!]
          : Array.from({ length: indices.length / 3 }, (_, i) =>
              indices
                .slice(i * 3, i * 3 + 3)
                .map((j) => [flat.vertices[j * 2]!, flat.vertices[j * 2 + 1]!]),
            );
      for (const polygon of polygons) {
        const points = polygon.map(([x, y]) => ({
          x,
          y: -y * sine,
          z_bottom: min[2] * cosine,
          z_top: Math.max(min[2], planeHeight(region.plane, [x, y])) * cosine,
        }));
        if (points.every((p) => p.z_top - p.z_bottom < 1e-5)) continue;
        gameplay.volumes!.push({
          id: `solid-${gameplay.volumes!.length}`,
          node,
          shape: {
            points,
            solid: true,
            opaque: policy.opaque,
            mouse: true,
            show_shadow_polygon: false,
            default_material: policy.material,
          },
        });
      }
    }
  for (const polygon of walkable) {
    const rings = cleanCaps(polygon).filter((r, i) => i === 0 || (r.length >= 3 && area(r) > 1e-4));
    if (rings[0]!.length < 3 || area(rings[0]!) < 1) continue;
    gameplay.surfaces.push({
      id: `walkway-${gameplay.surfaces.length}`,
      node,
      polygon: rings[0]!.map(([x, y]) => [x, -y * sine]),
      holes: rings.slice(1).map((r) => r.map(([x, y]) => [x, -y * sine])),
      height: deck.plane[2] * cosine,
      preserveMovementPrecision: true,
      projectionMaterials: { defaultMaterial: policy.material, regions: [] },
    });
  }
  if (!gameplay.surfaces.length) throw new Error("Walkway has no usable area");
  return gameplay;
}

async function readModel(file: string): Promise<Triangle[]> {
  const document = await new NodeIO().registerExtensions(ALL_EXTENSIONS).read(file);
  const scene = document.getRoot().getDefaultScene() ?? document.getRoot().listScenes()[0];
  if (!scene) throw new Error("Missing model scene");
  const triangles: Triangle[] = [];
  const wrapper = scene.listChildren().find((n) => n.getName() === "map");
  const inverse = wrapper ? mat4.invert(mat4.create(), wrapper.getWorldMatrix()) : mat4.create();
  if (!inverse) throw new Error("Invalid model wrapper transform");
  scene.traverse((node) => {
    const mesh = node.getMesh();
    if (!mesh) return;
    const matrix = mat4.multiply(mat4.create(), inverse, node.getWorldMatrix());
    for (const primitive of mesh.listPrimitives()) {
      if (primitive.getMode() !== 4) continue;
      const positions = primitive.getAttribute("POSITION");
      if (!positions) throw new Error("Missing mesh positions");
      const indices = primitive.getIndices(),
        count = indices?.getCount() ?? positions.getCount();
      for (let i = 0; i < count; i += 3) {
        const points: Vec3[] = [];
        for (let k = 0; k < 3; k++) {
          const p = positions.getElement(indices ? indices.getScalar(i + k) : i + k, []);
          const world = vec3.transformMat4(vec3.create(), [p[0]!, p[1]!, p[2]!], matrix);
          points.push([world[0], world[1], world[2]]);
        }
        triangles.push([points[0]!, points[1]!, points[2]!]);
      }
    }
  });
  return triangles;
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const [file, node, json] = process.argv.slice(2);
  if (!file || !node || !json)
    throw new Error("Usage: author-wall-walkway.ts model.glb node policy-json");
  const result = authorWallWalkway(
    await readModel(file),
    node,
    JSON.parse(json) as Policy,
    process.env.WALL_AUTHOR_TRACE ? (message) => console.error(message) : undefined,
  );
  result.spline!.modelSha256 = createHash("sha256")
    .update(await fs.readFile(file))
    .digest("hex");
  console.log(JSON.stringify(result));
}
