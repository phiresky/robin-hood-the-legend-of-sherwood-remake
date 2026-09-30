import { terrainTexture, terrainMaterialTexture } from "./terrain-texture.ts";
import * as THREE from "three";
import { excludedCornerAssetIds } from "./spline-corners.ts";
import { terrainSplineCurve, gameToScene, type LevelSpline, type MapCamera } from "@rle/shared";
import { sampleSpline, splineMaterialWeightsAt } from "../../shared/src/spline-sampling.ts";
import { roadGeometry } from "./road-geometry.ts";
import type { Level3D } from "@rle/shared";

export const splineCurve = terrainSplineCurve;

export function riverGeometry(path: LevelSpline, camera: MapCamera, document?: Level3D) {
  if (document && path.kind === "road") return roadGeometry(path, camera, document);
  const samples = sampleSpline(path, camera);
  const count = samples.length - 1;
  const positions: number[] = [],
    uvs: number[] = [],
    indices: number[] = [];
  const columns = 1;
  for (let i = 0; i <= count; i++) {
    const sample = samples[i]!,
      p = sample.position,
      tangent = sample.tangent;
    const normal = new THREE.Vector3(-tangent.y, tangent.x, 0)
      .normalize()
      .multiplyScalar(sample.width / 2);
    for (let column = 0; column <= columns; column++) {
      const sign = (column / columns) * 2 - 1;
      const x = p.x + sign * normal.x,
        y = p.y + sign * normal.y;
      positions.push(x, y, p.z + 0.8);
      uvs.push(column / columns, sample.distance / path.repeatLength);
    }
    if (i < count) {
      for (let column = 0; column < columns; column++) {
        const a = i * (columns + 1) + column,
          b = a + columns + 1;
        indices.push(a, a + 1, b, a + 1, b + 1, b);
      }
    }
  }
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", new THREE.Float32BufferAttribute(positions, 3));
  geometry.setAttribute("uv", new THREE.Float32BufferAttribute(uvs, 2));
  geometry.setIndex(indices);
  geometry.computeVertexNormals();
  return geometry;
}

/** Synthesized surface art with feathered ribbon edges. */
export function defaultRiverTexture(road = false) {
  return terrainTexture(road ? "dirt" : "water", true);
}

export function riverMesh(
  path: LevelSpline,
  camera: MapCamera,
  document?: Level3D,
  preview = false,
) {
  const texture = path.texture
    ? new THREE.TextureLoader().load(path.texture)
    : path.pointMaterials
      ? blendedSplineTexture(path, camera, document, preview)
      : defaultRiverTexture(path.kind === "road");
  if (!path.texture && !path.pointMaterials) texture.repeat.y = path.repeatLength / 1024;
  texture.wrapS = THREE.ClampToEdgeWrapping;
  texture.wrapT =
    path.pointMaterials && !path.texture ? THREE.ClampToEdgeWrapping : THREE.RepeatWrapping;
  texture.colorSpace = THREE.SRGBColorSpace;
  texture.magFilter = THREE.LinearFilter;
  texture.minFilter = THREE.LinearMipmapLinearFilter;
  texture.generateMipmaps = true;
  const material = new THREE.MeshBasicMaterial({
    map: texture,
    side: THREE.DoubleSide,
    transparent: true,
    depthWrite: false,
    polygonOffset: true,
    polygonOffsetFactor: -2,
    polygonOffsetUnits: -2,
  });
  const mesh = new THREE.Mesh(riverGeometry(path, camera, document), material);
  mesh.renderOrder = 1;
  mesh.userData.noSunShadow = true;
  return mesh;
}

/** Bake the longitudinal material blend into a regular texture, also usable by depth export. */
export function blendedSplineTexture(
  path: LevelSpline,
  camera: MapCamera,
  document?: Level3D,
  preview = false,
) {
  const curve = splineCurve(path, camera),
    length = curve.getLength();
  // Live dragging preserves the current material blend with a bounded sampling cost.
  // Committed surfaces and exports always use the full-resolution bake.
  const width = preview ? 64 : 128,
    height = Math.max(2, Math.min(preview ? 512 : 4096, Math.ceil(length / 2)));
  const data = new Uint8Array(width * height * 4);
  const textures = new Map<string, THREE.DataTexture>();
  const ids =
    path.pointMaterials ??
    path.points.map(() => (path.kind === "river" ? "water_still" : "path_dirt"));
  for (const id of [
    ...ids,
    ...(path.pointMaterialMixes ?? []).flatMap((mix) => Object.keys(mix ?? {})),
  ])
    if (!textures.has(id))
      textures.set(id, terrainMaterialTexture(id, document?.customMaterials, true));
  const singleStoneCenters = new Map<string, number[]>();
  const arcLengths = curve.getLengths();
  const sections = path.closed ? path.points.length : path.points.length - 1;
  for (const id of textures.keys())
    if (id.endsWith("_single")) {
      const runs: [number, number][] = [];
      for (let i = 0; i < path.points.length; i++) {
        const mix = path.pointMaterialMixes?.[i];
        const present = mix ? (mix[id] ?? 0) > 0 : ids[i] === id;
        if (!present) continue;
        const previous = runs.at(-1);
        if (previous && previous[1] === i - 1) previous[1] = i;
        else runs.push([i, i]);
      }
      singleStoneCenters.set(
        id,
        runs.map(([start, end]) => {
          const parameter = (Math.max(0, start - 1) + Math.min(sections, end + 1)) / (2 * sections);
          return arcLengths[Math.round(parameter * (arcLengths.length - 1))]!;
        }),
      );
    }
  try {
    for (let row = 0; row < height; row++) {
      const distance = (row / (height - 1)) * length;
      const weights = splineMaterialWeightsAt(path, curve.getUtoTmapping(row / (height - 1), 0));
      // Material weights, source rows, and isolated stone locations are constant across a row.
      const sources = Object.entries(weights)
        .filter(([, weight]) => weight > 0)
        .map(([id, weight]) => {
          const tile = textures.get(id)!.image;
          const centers = singleStoneCenters.get(id);
          const center = centers?.length
            ? centers.reduce((a, b) => (Math.abs(a - distance) < Math.abs(b - distance) ? a : b))
            : undefined;
          const rowInTile =
            center === undefined
              ? Math.floor(distance) % tile.height
              : Math.max(
                  0,
                  Math.min(tile.height - 1, Math.floor(distance - center + tile.height / 2)),
                );
          return {
            pixels: tile.data as Uint8Array,
            width: tile.width,
            offset: rowInTile * tile.width * 4,
            weight,
          };
        });
      for (let column = 0; column < width; column++) {
        const target = (row * width + column) * 4;
        let red = 0,
          green = 0,
          blue = 0,
          alpha = 0;
        for (const source of sources) {
          const offset = source.offset + Math.floor((column / width) * source.width) * 4;
          red += source.pixels[offset]! * source.weight;
          green += source.pixels[offset + 1]! * source.weight;
          blue += source.pixels[offset + 2]! * source.weight;
          alpha += source.pixels[offset + 3]! * source.weight;
        }
        data[target] = Math.round(red);
        data[target + 1] = Math.round(green);
        data[target + 2] = Math.round(blue);
        data[target + 3] = Math.round(alpha);
      }
    }
  } finally {
    for (const texture of textures.values()) texture.dispose();
  }
  const texture = new THREE.DataTexture(data, width, height);
  texture.colorSpace = THREE.SRGBColorSpace;
  texture.repeat.y = path.repeatLength / Math.max(length, 1);
  texture.wrapT = THREE.ClampToEdgeWrapping;
  texture.needsUpdate = true;
  return texture;
}

type Vertex = Record<string, number[]>;
function interpolate(a: Vertex, b: Vertex, t: number): Vertex {
  return Object.fromEntries(
    Object.keys(a).map((key) => [key, a[key]!.map((v, i) => v + (b[key]![i]! - v) * t)]),
  );
}
function clip(polygon: Vertex[], axis: number, boundary: number, above: boolean) {
  const result: Vertex[] = [];
  for (let i = 0; i < polygon.length; i++) {
    const a = polygon[i]!,
      b = polygon[(i + 1) % polygon.length]!;
    const av = a.position![axis]!,
      bv = b.position![axis]!;
    const insideA = above ? av >= boundary : av <= boundary;
    const insideB = above ? bv >= boundary : bv <= boundary;
    if (insideA) result.push(a);
    if (insideA !== insideB) result.push(interpolate(a, b, (boundary - av) / (bv - av)));
  }
  return result;
}

export interface WallSectionProfile {
  start: number;
  end: number;
  sections: { center: number; width: number }[];
}

/** Measure cross-sections, excluding the source's longitudinal bend from thickness. */
export function wallSectionProfile(
  source: THREE.Object3D,
  bounds: THREE.Box3,
  path: LevelSpline,
): WallSectionProfile {
  const axis = path.axis === "y" ? 1 : 0,
    cross = 1 - axis;
  const full = bounds.max.getComponent(axis) - bounds.min.getComponent(axis);
  const start = bounds.min.getComponent(axis) + full * (path.sourceStart ?? 0);
  const end = bounds.min.getComponent(axis) + full * (path.sourceEnd ?? 1);
  const count = 64;
  const spans = Array.from({ length: count + 1 }, () => ({ min: Infinity, max: -Infinity }));
  source.traverse((node) => {
    if (!(node instanceof THREE.Mesh)) return;
    const geometry = node.geometry,
      positions = geometry.getAttribute("position");
    const vertices = Array.from({ length: positions.count }, (_, index) =>
      new THREE.Vector3().fromBufferAttribute(positions, index).applyMatrix4(node.matrixWorld),
    );
    const indices = geometry.index;
    for (let i = 0; i < (indices?.count ?? positions.count); i += 3) {
      const triangle = [0, 1, 2].map((k) => vertices[indices ? indices.getX(i + k) : i + k]!);
      const low = Math.min(...triangle.map((p) => p.getComponent(axis)));
      const high = Math.max(...triangle.map((p) => p.getComponent(axis)));
      const first = Math.max(0, Math.ceil(((low - start) / (end - start)) * count));
      const last = Math.min(count, Math.floor(((high - start) / (end - start)) * count));
      for (let station = first; station <= last; station++) {
        const coordinate = start + ((end - start) * station) / count;
        const span = spans[station]!;
        for (let edge = 0; edge < 3; edge++) {
          const a = triangle[edge]!,
            b = triangle[(edge + 1) % 3]!;
          const av = a.getComponent(axis),
            bv = b.getComponent(axis);
          if (Math.abs(av - coordinate) < 1e-6) {
            span.min = Math.min(span.min, a.getComponent(cross));
            span.max = Math.max(span.max, a.getComponent(cross));
          }
          if ((av < coordinate && bv > coordinate) || (av > coordinate && bv < coordinate)) {
            const value =
              a.getComponent(cross) +
              ((b.getComponent(cross) - a.getComponent(cross)) * (coordinate - av)) / (bv - av);
            span.min = Math.min(span.min, value);
            span.max = Math.max(span.max, value);
          }
        }
      }
    }
  });
  const valid = spans
    .map((span, index) => ({ ...span, index }))
    .filter((span) => span.max - span.min > 0.001);
  if (!valid.length) throw new Error("Wall source has no measurable cross-section");
  const sections = spans.map((span, index) => {
    if (span.max - span.min > 0.001)
      return { center: (span.min + span.max) / 2, width: span.max - span.min };
    // A tapered end can reduce to a single vertex. Use the adjacent section
    // at that endpoint so repetitions join with the requested thickness.
    if (index !== 0 && index !== count)
      throw new Error("Wall source has a gap; trim to a continuous section");
    const adjacent = index === 0 ? valid[0]! : valid.at(-1)!;
    return { center: (adjacent.min + adjacent.max) / 2, width: adjacent.max - adjacent.min };
  });
  return { start, end, sections };
}

/** Subdivide longitudinally before bending; UVs interpolate across every cut. */
export function wallGeometry(
  source: THREE.BufferGeometry,
  matrix: THREE.Matrix4,
  path: LevelSpline,
  camera: MapCamera,
  bounds: THREE.Box3,
  repeat: number,
  profile?: WallSectionProfile,
) {
  const axis = path.axis === "y" ? 1 : 0,
    cross = 1 - axis;
  const full = bounds.max.getComponent(axis) - bounds.min.getComponent(axis);
  const start = bounds.min.getComponent(axis) + full * (path.sourceStart ?? 0);
  const end = bounds.min.getComponent(axis) + full * (path.sourceEnd ?? 1);
  if (end - start <= 0.001) throw new Error("Wall source has no length along the selected axis");
  const curve = splineCurve(path, camera),
    length = curve.getLength();
  const sourceWidth = bounds.max.getComponent(cross) - bounds.min.getComponent(cross);
  if (sourceWidth <= 0.001) throw new Error("Wall source has no thickness");
  const center = (bounds.max.getComponent(cross) + bounds.min.getComponent(cross)) / 2;
  const attributes = Object.entries(source.attributes).filter(
    ([key]) => key !== "normal" && key !== "tangent",
  );
  const output: Record<string, number[]> = Object.fromEntries(attributes.map(([key]) => [key, []]));
  const read = (index: number): Vertex => {
    const vertex: Vertex = {};
    for (const [key, attribute] of attributes) {
      vertex[key] = Array.from({ length: attribute.itemSize }, (_, k) =>
        attribute.getComponent(index, k),
      );
    }
    const p = new THREE.Vector3(...(vertex.position! as [number, number, number])).applyMatrix4(
      matrix,
    );
    vertex.position = p.toArray();
    return vertex;
  };
  const push = (vertex: Vertex) => {
    const p = vertex.position!,
      along = (p[axis]! - start) / (end - start);
    const distance = (repeat + along) * path.repeatLength;
    const t = Math.min(1, Math.max(0, distance / length));
    const point = curve.getPointAt(t),
      tangent = curve.getTangentAt(t);
    let sectionCenter = center,
      sectionWidth = sourceWidth;
    if (profile) {
      const sample = Math.min(1, Math.max(0, along)) * (profile.sections.length - 1);
      const first = Math.floor(sample),
        fraction = sample - first;
      const a = profile.sections[first]!,
        b = profile.sections[Math.min(first + 1, profile.sections.length - 1)]!;
      sectionCenter = a.center + (b.center - a.center) * fraction;
      sectionWidth = a.width + (b.width - a.width) * fraction;
    }
    const lateral =
      (((p[cross]! - sectionCenter) * path.width) / sectionWidth) *
      (axis === 1 ? -1 : 1) *
      (path.flipCrossSection ? -1 : 1);
    const normal = new THREE.Vector3(-tangent.y, tangent.x, 0).normalize();
    output.position!.push(
      point.x + normal.x * lateral,
      point.y + normal.y * lateral,
      point.z + p[2]! - bounds.min.z,
    );
    for (const [key] of attributes) if (key !== "position") output[key]!.push(...vertex[key]!);
  };
  const count = source.index?.count ?? source.getAttribute("position").count;
  const bands = 12;
  const lastFraction = Math.min(1, length / path.repeatLength - repeat);
  for (let i = 0; i < count; i += 3) {
    const triangle = [0, 1, 2].map((k) => read(source.index ? source.index.getX(i + k) : i + k));
    const low = Math.min(...triangle.map((v) => v.position![axis]!));
    const high = Math.max(...triangle.map((v) => v.position![axis]!));
    for (let band = 0; band < bands; band++) {
      const a = start + ((end - start) * band) / bands;
      const b = start + (end - start) * Math.min((band + 1) / bands, lastFraction);
      if (b <= a || low > b || high < a) continue;
      const polygon = clip(clip(triangle, axis, a, true), axis, b, false);
      for (let j = 1; j + 1 < polygon.length; j++) {
        push(polygon[0]!);
        push(polygon[path.flipCrossSection ? j + 1 : j]!);
        push(polygon[path.flipCrossSection ? j : j + 1]!);
      }
    }
  }
  const geometry = new THREE.BufferGeometry();
  for (const [key, attribute] of attributes)
    geometry.setAttribute(key, new THREE.Float32BufferAttribute(output[key]!, attribute.itemSize));
  geometry.computeVertexNormals();
  geometry.computeBoundingSphere();
  return geometry;
}

/** Turns are measured in the ground plane, including the seam of closed walls. */
export function wallCorners(path: LevelSpline, camera: MapCamera) {
  if (!path.cornerAsset || excludedCornerAssetIds.has(path.cornerAsset)) return [];
  const points = path.points.map((p) => new THREE.Vector3(...gameToScene(camera, ...p)));
  return points.flatMap((p, i) => {
    if ((!path.closed && (i === 0 || i === points.length - 1)) || path.cornerDisabled?.includes(i))
      return [];
    const incoming = p.clone().sub(points[(i + points.length - 1) % points.length]!);
    incoming.z = 0;
    incoming.normalize();
    const outgoing = points[(i + 1) % points.length]!.clone().sub(p);
    outgoing.z = 0;
    outgoing.normalize();
    const angle = THREE.MathUtils.radToDeg(incoming.angleTo(outgoing));
    if (angle < (path.cornerMinAngle ?? 35)) return [];
    const direction = incoming.add(outgoing).normalize();
    return [
      {
        index: i,
        position: p,
        rotation:
          Math.atan2(direction.y, direction.x) + THREE.MathUtils.degToRad(path.cornerRotation ?? 0),
      },
    ];
  });
}

function towerWall(
  path: LevelSpline,
  camera: MapCamera,
  sources: Map<string, THREE.Object3D>,
): THREE.Group {
  const corners = wallCorners(path, camera),
    result = new THREE.Group();
  if (!corners.length) return wallMesh({ ...path, cornerAsset: undefined }, camera, sources);
  const tower = new THREE.Group();
  for (const [key, node] of sources)
    if (key.startsWith("asset:" + path.cornerAsset + ":")) tower.add(node.clone(true));
  if (!tower.children.length) throw new Error("Missing corner tower source: " + path.cornerAsset);
  tower.updateWorldMatrix(true, true);
  const bounds = new THREE.Box3().setFromObject(tower),
    center = bounds.getCenter(new THREE.Vector3());
  const anchor = new THREE.Vector3(center.x, center.y, bounds.min.z);
  try {
    // Each tower terminates adjoining spans, avoiding a rounded curtain bulge
    // underneath a sharp corner. Gentle intermediate controls stay curved.
    const breaks = corners.map((c) => c.index);
    const runs: number[][] = [];
    if (path.closed)
      for (let j = 0; j < breaks.length; j++) {
        const run = [breaks[j]!],
          end = breaks[(j + 1) % breaks.length]!;
        let i = (breaks[j]! + 1) % path.points.length;
        while (i !== end) {
          run.push(i);
          i = (i + 1) % path.points.length;
        }
        run.push(end);
        runs.push(run);
      }
    else {
      const stops = [0, ...breaks, path.points.length - 1];
      for (let j = 0; j < stops.length - 1; j++)
        runs.push(Array.from({ length: stops[j + 1]! - stops[j]! + 1 }, (_, k) => stops[j]! + k));
    }
    for (const run of runs)
      result.add(
        wallMesh(
          {
            ...path,
            cornerAsset: undefined,
            closed: false,
            points: run.map((i) => path.points[i]!),
          },
          camera,
          sources,
        ),
      );
    for (const corner of corners) {
      const instance = tower.clone(true);
      instance.traverse((node) => {
        if (node instanceof THREE.Mesh) node.geometry = node.geometry.clone();
      });
      const offset = new THREE.Group();
      offset.add(instance);
      instance.position.sub(anchor);
      const scale = path.cornerScale ?? 1,
        widthScale = path.cornerWidthScale ?? 1;
      offset.scale.set(scale * widthScale, scale * widthScale, scale);
      offset.rotation.z = corner.rotation;
      offset.position.copy(corner.position);
      offset.userData.cornerPoint = corner.index;
      result.add(offset);
    }
    return result;
  } catch (error) {
    result.traverse((node) => {
      if (node instanceof THREE.Mesh) node.geometry.dispose();
    });
    throw error;
  }
}

export function wallMesh(
  path: LevelSpline,
  camera: MapCamera,
  sources: Map<string, THREE.Object3D>,
): THREE.Group {
  if (path.cornerAsset && !excludedCornerAssetIds.has(path.cornerAsset))
    return towerWall(path, camera, sources);
  const source = new THREE.Group();
  for (const [key, node] of sources)
    if (key.startsWith("asset:" + path.asset + ":")) source.add(node.clone(true));
  if (!source.children.length) throw new Error("Missing wall source: " + path.asset);
  source.rotation.z = (-(path.sourceAngle ?? 0) * Math.PI) / 180;
  source.updateWorldMatrix(true, true);
  const bounds = new THREE.Box3().setFromObject(source);
  const profile = path.sourceStraight ? undefined : wallSectionProfile(source, bounds, path);
  const length = splineCurve(path, camera).getLength();
  const repeats = Math.ceil(length / path.repeatLength);
  if (repeats > 512) throw new Error("Wall path would exceed 512 repeats; increase repeat length");
  const result = new THREE.Group();
  try {
    source.traverse((node) => {
      if (!(node instanceof THREE.Mesh)) return;
      const materials = Array.isArray(node.material) ? node.material : [node.material];
      const groups = Array.isArray(node.material)
        ? node.geometry.groups
        : [
            {
              start: 0,
              count: node.geometry.index?.count ?? node.geometry.getAttribute("position").count,
              materialIndex: 0,
            },
          ];
      for (const group of groups) {
        const geometry = node.geometry.clone();
        geometry.clearGroups();
        const indices = Array.from({ length: group.count }, (_, index) =>
          node.geometry.index ? node.geometry.index.getX(group.start + index) : group.start + index,
        );
        geometry.setIndex(indices);
        try {
          for (let i = 0; i < repeats; i++)
            result.add(
              new THREE.Mesh(
                wallGeometry(geometry, node.matrixWorld, path, camera, bounds, i, profile),
                materials[group.materialIndex ?? 0],
              ),
            );
        } finally {
          geometry.dispose();
        }
      }
    });
    return result;
  } catch (error) {
    result.traverse((node) => {
      if (node instanceof THREE.Mesh) node.geometry.dispose();
    });
    throw error;
  }
}
