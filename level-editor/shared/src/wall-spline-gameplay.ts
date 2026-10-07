import earcut, { flatten } from "earcut";
import { Vector3 } from "three";
import type { Level3D } from "./level3d.ts";
import type { LevelSpline } from "./splines.ts";
import type { ProjectionAssetDescriptor } from "./projection-assets.ts";
import type {
  AssetGameplay,
  AssetLightRegion,
  AssetWalkableSurface,
  GameplayAssetDescriptor,
} from "./asset-gameplay.ts";
import { validateAssetGameplay } from "./asset-gameplay.ts";
import { gameToScene, type Vec3 } from "./scene.ts";
import { applyAffineMatrix, sceneToGame } from "./geometry.ts";
import { splineCurve } from "./spline-sampling.ts";
import { wallCorners, wallRuns } from "./wall-path.ts";
import { heightPlane, planeHeight } from "./gameplay-plane.ts";
import { matchesWallSource, wallSectionAt } from "./wall-section-profile.ts";
import { quantizeGeneratedMotionPolygon } from "./motion-quantization.ts";
import { clipSplinePolyline } from "./clip-spline-polyline.ts";
import { splineLightReceivers } from "./spline-light-receivers.ts";
import type { MaskTriangle } from "./compile-mask-geometry.ts";
import type { MaskTriangleAlpha } from "./mask-alpha-sampler.ts";

type Vertex = number[];
function clip(vertices: Vertex[], axis: number, boundary: number, above: boolean) {
  const result: Vertex[] = [];
  for (let i = 0; i < vertices.length; i++) {
    const a = vertices[i]!,
      b = vertices[(i + 1) % vertices.length]!;
    const insideA = above ? a[axis]! >= boundary : a[axis]! <= boundary;
    const insideB = above ? b[axis]! >= boundary : b[axis]! <= boundary;
    if (insideA) result.push(a);
    if (insideA !== insideB) {
      const t = (boundary - a[axis]!) / (b[axis]! - a[axis]!);
      result.push(a.map((n, k) => n + (b[k]! - n) * t));
    }
  }
  return result;
}
const area = (a: Vertex, b: Vertex, c: Vertex) =>
  (b[0]! - a[0]!) * (c[1]! - a[1]!) - (b[1]! - a[1]!) * (c[0]! - a[0]!);

/** Compile calibrated asset definitions through the same spline frames as their artwork. */
export function wallSplineGameplay(
  document: Level3D,
  descriptors: ReadonlyMap<string, ProjectionAssetDescriptor>,
  bestEffort: boolean,
  imageOrigin: readonly number[] = [0, 0],
) {
  const warnings: string[] = [],
    result: GameplayAssetDescriptor[] = [];
  const report = (message: string) => {
    if (!bestEffort) throw new Error(message);
    warnings.push(message);
  };
  for (const path of document.splines ?? []) {
    if (path.kind !== "wall") continue;
    const generated: GameplayAssetDescriptor = {
      version: 1,
      kind: "projection-mapped-asset",
      id: `wall-spline-${path.id}`,
      name: path.name,
      source_map: document.map,
      model: "generated",
      editor_usage: "map-background",
      parts: [],
      gameplay: {
        version: 1,
        collision: "none",
        surfaces: [],
        doors: [],
        volumes: [],
        movementSolids: [],
        movementBlockers: [],
        movementClearances: [],
        materials: [],
        lights: [],
        sounds: [],
        masks: [],
      },
    };
    const out = generated.gameplay!;
    let materialSequence = 0;
    let lightGroupSequence = 0;
    let lightSequence = 0;
    function append(
      assetId: string | undefined,
      run: LevelSpline | undefined,
      corner?: ReturnType<typeof wallCorners>[number],
    ) {
      const descriptor = assetId ? descriptors.get(assetId) : undefined,
        data = descriptor && (descriptor as GameplayAssetDescriptor).gameplay;
      if (!descriptor || !data?.spline)
        throw new Error(
          `asset ${assetId ?? "(missing)"} needs spline model calibration and local gameplay`,
        );
      validateAssetGameplay(data, descriptor);
      const pinned = document.assetSources?.find((s) => s.id === assetId);
      if (data.spline.modelSha256 && pinned && pinned.model_sha256 !== data.spline.modelSha256)
        throw new Error(`asset ${assetId} spline calibration belongs to a different model`);
      const { frames } = data.spline;
      const deformation = run && data.spline.deformations?.find((c) => matchesWallSource(c, run));
      const bounds = deformation?.bounds ?? data.spline.bounds;
      const profile = run && !run.sourceStraight ? deformation?.profile : undefined;
      if (run && (!run.sourceStraight || (run.sourceAngle ?? 0) !== 0) && !deformation)
        throw new Error(
          "cross-section straightening and source rotation require matching asset mesh calibration",
        );
      if (data.movementTransitions?.length || descriptor.states)
        throw new Error(`asset ${assetId} has stateful geometry; a static wall source is required`);
      for (const issue of data.draft?.issues ?? [])
        warnings.push(`Wall spline ${path.id}, asset ${assetId}: ${issue}`);
      if (
        data.doors.length ||
        data.lifts?.length ||
        data.interiors?.length ||
        data.jumpPairs?.length ||
        data.jumpSegments?.length
      )
        warnings.push(
          `Wall spline ${path.id}, asset ${assetId}: static surfaces and collision exported; doors, lifts, interiors and saved jump connections require separate placed assets.`,
        );
      const source = (node: string, point: Vec3): Vec3 => {
        const matrix = frames[node];
        if (!matrix) throw new Error(`asset ${assetId} needs a spline frame for ${node}`);
        const p = applyAffineMatrix(matrix, gameToScene(document.camera, ...point));
        const angle = (-(run?.sourceAngle ?? 0) * Math.PI) / 180;
        return [
          p[0] * Math.cos(angle) - p[1] * Math.sin(angle),
          p[0] * Math.sin(angle) + p[1] * Math.cos(angle),
          p[2],
        ];
      };
      const axis = run?.axis === "y" ? 1 : 0,
        cross = 1 - axis;
      const start =
        bounds.min[axis] + (bounds.max[axis] - bounds.min[axis]) * (run?.sourceStart ?? 0);
      const end = bounds.min[axis] + (bounds.max[axis] - bounds.min[axis]) * (run?.sourceEnd ?? 1);
      const center = (bounds.min[cross]! + bounds.max[cross]!) / 2,
        width = bounds.max[cross]! - bounds.min[cross]!;
      const curve = run ? splineCurve(run, document.camera) : undefined,
        length = curve?.getLength() ?? 0;
      const repeats = run ? Math.ceil(length / run.repeatLength) : 1;
      if (run && (end - start <= 0.001 || width <= 0.001 || length <= 0.001 || repeats > 512))
        throw new Error("invalid or excessive wall repetition");
      const bands = run?.curved === false ? 1 : 12;
      const framesByDistance = new Map<number, { point: Vector3; normal: Vector3 }>();
      const toGame = (point: Vec3): Vec3 => {
        const p = sceneToGame(document.camera, point);
        return [
          Math.round(p[0] * 1024) / 1024,
          Math.round(p[1] * 1024) / 1024,
          Math.round(p[2] * 1048576) / 1048576,
        ];
      };
      const warp = (p: Vec3, repeat: number): Vec3 => {
        if (corner) {
          const sx = (path.cornerScale ?? 1) * (path.cornerWidthScale ?? 1),
            sz = path.cornerScale ?? 1;
          const x = (p[0] - (bounds.min[0] + bounds.max[0]) / 2) * sx,
            y = (p[1] - (bounds.min[1] + bounds.max[1]) / 2) * sx;
          return toGame([
            corner.position.x + x * Math.cos(corner.rotation) - y * Math.sin(corner.rotation),
            corner.position.y + x * Math.sin(corner.rotation) + y * Math.cos(corner.rotation),
            corner.position.z + (p[2] - bounds.min[2]) * sz,
          ]);
        }
        if (!run || !curve) throw new Error("Missing wall deformation");
        const along = (p[axis] - start) / (end - start),
          t = Math.min(1, Math.max(0, ((repeat + along) * run.repeatLength) / length));
        let frame = framesByDistance.get(t);
        if (!frame) {
          const tangent = curve.getTangentAt(t);
          frame = {
            point: curve.getPointAt(t),
            normal: new Vector3(-tangent.y, tangent.x, 0).normalize(),
          };
          framesByDistance.set(t, frame);
        }
        const section = profile ? wallSectionAt(profile, along) : { center, width };
        const lateral =
          (((p[cross]! - section.center) * run.width) / section.width) *
          (axis === 1 ? -1 : 1) *
          (run.flipCrossSection ? -1 : 1);
        return toGame([
          frame.point.x + frame.normal.x * lateral,
          frame.point.y + frame.normal.y * lateral,
          frame.point.z + p[2] - bounds.min[2],
        ]);
      };
      let stations = Array.from(
        { length: bands + 1 },
        // Keep exact endpoints: arithmetic can put the final station just beyond
        // `end`, causing range filtering to remove an entire terminal band.
        (_, i) => (i === 0 ? start : i === bands ? end : start + ((end - start) * i) / bands),
      );
      const pieces = (
        vertices: Vertex[],
        emit: (v: Vertex[], repeat: number) => void,
        material = false,
      ) => {
        // Material contours can lie on vertical faces. Triangulate on their
        // largest plane rather than discarding them in the ground projection.
        let axes = [0, 1];
        if (material) {
          const normal = [0, 0, 0];
          for (let i = 0; i < vertices.length; i++) {
            const a = vertices[i]!,
              b = vertices[(i + 1) % vertices.length]!;
            for (let axis = 0; axis < 3; axis++)
              normal[axis]! +=
                (a[(axis + 1) % 3]! - b[(axis + 1) % 3]!) *
                (a[(axis + 2) % 3]! + b[(axis + 2) % 3]!);
          }
          const largest = normal.map(Math.abs).indexOf(Math.max(...normal.map(Math.abs)));
          axes = [0, 1, 2].filter((axis) => axis !== largest);
        }
        const indices = earcut(vertices.flatMap((v) => axes.map((axis) => v[axis]!)));
        if (material && !indices.length)
          warnings.push(
            `Wall spline ${path.id}, asset ${assetId}: a degenerate spatial contour could not be triangulated and was omitted.`,
          );
        for (let repeat = 0; repeat < repeats; repeat++)
          for (let t = 0; t < indices.length; t += 3) {
            const triangle = indices.slice(t, t + 3).map((i) => vertices[i]!);
            if (!run) {
              emit(triangle, repeat);
              continue;
            }
            for (let band = 0; band + 1 < stations.length; band++) {
              const a = stations[band]!,
                b = Math.min(
                  stations[band + 1]!,
                  start + (end - start) * (length / run.repeatLength - repeat),
                );
              if (b - a <= 1e-7) continue;
              const polygon = clip(clip(triangle, axis, a, true), axis, b, false);
              for (let j = 1; j + 1 < polygon.length; j++)
                if (material || Math.abs(area(polygon[0]!, polygon[j]!, polygon[j + 1]!)) > 1e-7)
                  emit([polygon[0]!, polygon[j]!, polygon[j + 1]!], repeat);
            }
          }
      };
      const templates: NonNullable<AssetGameplay["volumes"]> = [...(data.volumes ?? [])];
      if (data.collision === "parts")
        for (const part of descriptor.parts) {
          if (
            part.default_hidden ||
            part.collision === "none" ||
            part.mission_profile !== undefined ||
            !part.obstacle_local_game
          )
            continue;
          templates.push({ id: part.node, node: part.node, shape: part.obstacle_local_game });
        }
      // Shared longitudinal cuts prevent T-junctions opening between independently
      // triangulated solids and walkways when their common edge bends along a curve.
      if (run?.curved !== false) {
        for (const volume of templates)
          for (const p of volume.shape.points)
            stations.push(source(volume.node, [p.x, p.y, p.z_bottom])[axis]);
        for (const surface of [
          ...data.surfaces,
          ...(data.movementBlockers ?? []),
          ...(data.movementClearances ?? []),
        ]) {
          const points = surface.polygon.map(([x, y], i): Vec3 => [
            x,
            y,
            typeof surface.height === "number" ? surface.height : surface.height[i]!,
          ]);
          const plane = heightPlane(points);
          for (const p of [
            ...points,
            ...(surface.holes ?? []).flatMap((h) =>
              h.map(([x, y]): Vec3 => [x, y, planeHeight(plane, [x, y])]),
            ),
          ])
            stations.push(source(surface.node, p)[axis]);
        }
      }
      stations = [...new Set(stations.filter((x) => x >= start && x <= end))].sort((a, b) => a - b);
      const volumeRefs = new Map<string, Map<number, string[]>>();
      const materialRefs = new Map<string, Map<number, string[]>>();
      const remember = (
        refs: Map<string, Map<number, string[]>>,
        source: string,
        repeat: number,
        id: string,
      ) => {
        const repeats = refs.get(source) ?? new Map<number, string[]>();
        const ids = repeats.get(repeat) ?? [];
        ids.push(id);
        repeats.set(repeat, ids);
        refs.set(source, repeats);
      };
      for (const volume of templates) {
        const points = volume.shape.points.map((p) => {
          const a = source(volume.node, [p.x, p.y, p.z_bottom]),
            b = source(volume.node, [p.x, p.y, p.z_top]);
          if (Math.hypot(a[0] - b[0], a[1] - b[1]) > 1e-5)
            throw new Error(`nonvertical volume ${volume.id}`);
          return [a[0], a[1], a[2], b[2]];
        });
        pieces(points, (vertices, repeat) => {
          const points = vertices.map((p) => {
            const a = warp([p[0]!, p[1]!, p[2]!], repeat),
              b = warp([p[0]!, p[1]!, p[3]!], repeat);
            return { x: a[0], y: a[1], z_bottom: a[2], z_top: b[2] };
          });
          if (Math.abs(area(...(points.map((p) => [p.x, p.y]) as [Vertex, Vertex, Vertex]))) < 1e-5)
            return;
          const id = `volume-${out.volumes!.length}`;
          remember(volumeRefs, volume.id, repeat, id);
          out.volumes!.push({
            id,
            node: "$root",
            ...(volume.movementHeadroom !== undefined
              ? { movementHeadroom: volume.movementHeadroom }
              : {}),
            shape: {
              points,
              solid: volume.shape.solid,
              opaque: volume.shape.opaque,
              mouse: volume.shape.mouse,
              show_shadow_polygon: volume.shape.show_shadow_polygon,
              default_material: volume.shape.default_material,
            },
          });
          if (data.movementSolids?.includes(volume.id) ?? data.movementBlockers === undefined)
            out.movementSolids!.push(id);
        });
      }
      for (const mask of data.masks ?? []) {
        const coverage = new Map<number, MaskTriangle[]>();
        const alphaCoverage = new Map<number, MaskTriangleAlpha[]>();
        for (const [index, triangle] of mask.triangles.entries()) {
          const sampling = mask.alphaCoverage?.triangles[index];
          pieces(
            triangle.map((p, i) => [
              ...source(mask.node, p),
              ...(sampling ? [...sampling.uv[i]!, sampling.alpha[i]!] : []),
            ]),
            (vertices, repeat) => {
              const warped = vertices.map((p) => warp([p[0]!, p[1]!, p[2]!], repeat));
              const world: MaskTriangle = [warped[0]!, warped[1]!, warped[2]!];
              if (
                Math.abs(
                  area(...(world.map(([x, y, z]) => [x, y - z]) as [Vertex, Vertex, Vertex])),
                ) < 1e-8
              )
                return;
              const triangles = coverage.get(repeat) ?? [];
              triangles.push(world);
              coverage.set(repeat, triangles);
              if (sampling) {
                const alpha = alphaCoverage.get(repeat) ?? [];
                alpha.push({
                  ...sampling,
                  uv: vertices.map((v) => [v[3]!, v[4]!]) as MaskTriangleAlpha["uv"],
                  alpha: [vertices[0]![5]!, vertices[1]![5]!, vertices[2]![5]!],
                });
                alphaCoverage.set(repeat, alpha);
              }
            },
            true,
          );
        }
        for (const [repeat, triangles] of coverage) {
          const limit = run
            ? Math.min(end, start + (end - start) * (length / run.repeatLength - repeat))
            : end;
          const anchorSource = source(mask.node, mask.anchor);
          const anchorCropped = run && (anchorSource[axis] < start || anchorSource[axis] > limit);
          if (
            anchorCropped &&
            !mask.receiverSegment &&
            !mask.receiverPolyline &&
            !mask.receiverPoints &&
            !mask.receiverPolylines
          ) {
            warnings.push(
              `Wall spline ${path.id}, mask ${mask.id}, repeat ${repeat}: cropped receiving anchor; mask omitted.`,
            );
            continue;
          }
          const receiver = (
            mask.receiverPolylines ??
            (mask.receiverPolyline
              ? [mask.receiverPolyline]
              : mask.receiverSegment
                ? [mask.receiverSegment]
                : undefined)
          )?.map((line) => line.map((p) => source(mask.node, p)));
          const receiverFragments = receiver
            ? run
              ? receiver
                  .flatMap((line) => clipSplinePolyline(line, axis, start, limit, stations))
                  .filter((line) => line.length >= 2)
              : receiver
            : [];
          const receiverPoints = mask.receiverPoints
            ?.map((p) => source(mask.node, p))
            .filter((p) => !run || (p[axis] >= start && p[axis] <= limit));
          if (receiverPoints && !receiverPoints.length) {
            warnings.push(
              `Wall spline ${path.id}, mask ${mask.id}: cropped receiving points; mask omitted.`,
            );
            continue;
          }
          if (receiver && !receiverFragments.length) {
            warnings.push(
              `Wall spline ${path.id}, mask ${mask.id}: cropped receiving probe; mask omitted.`,
            );
            continue;
          }
          // An explicit probe selects the layer. Keep its representative anchor
          // inside the surviving span so export-frame checks do not discard it.
          const anchor = warp(
            anchorCropped ? (receiverPoints?.[0] ?? receiverFragments[0]![0]!) : anchorSource,
            repeat,
          );
          let boundaryMissing = false;
          const boundary = (points: Vec3[] | undefined, closed = true): Vec3[][] => {
            if (!points) return [];
            let local = points.map((p) => source(mask.node, p));
            if (run && closed)
              local = clip(clip(local, axis, start, true), axis, limit, false).map((p): Vec3 => [
                p[0]!,
                p[1]!,
                p[2]!,
              ]);
            if (local.length < (closed ? 3 : 2)) {
              boundaryMissing = true;
              return [];
            }
            const line = closed ? [...local, local[0]!] : local;
            const fragments = run ? clipSplinePolyline(line, axis, start, limit, stations) : [line];
            if (
              !fragments.length ||
              (closed && fragments.length !== 1) ||
              fragments.some((fragment) => fragment.length < (closed ? 4 : 2))
            ) {
              boundaryMissing = true;
              return [];
            }
            return fragments.map((fragment) =>
              (closed ? fragment.slice(0, -1) : fragment).map((p) => warp(p, repeat)),
            );
          };
          const characterBoundary = boundary(mask.characterBoundary, mask.characterBoundaryClosed);
          const projectileBoundary = boundary(
            mask.projectileBoundary,
            mask.projectileBoundaryClosed,
          );
          const obstacles = mask.obstacles.flatMap((id) => volumeRefs.get(id)?.get(repeat) ?? []);
          if (
            boundaryMissing ||
            (!mask.view &&
              !characterBoundary.length &&
              !projectileBoundary.length &&
              !obstacles.length)
          ) {
            warnings.push(
              `Wall spline ${path.id}, mask ${mask.id}, repeat ${repeat}: cropping removed or disconnected its application boundary; mask omitted.`,
            );
            continue;
          }
          const receiverSegment: [Vec3, Vec3] = [
            [anchor[0], anchor[1] - 1 / 1024, anchor[2] - 1 / 1024],
            [anchor[0], anchor[1] + 1 / 1024, anchor[2] + 1 / 1024],
          ];
          // Each native mask carries one continuous application line. Separate
          // cropped fragments must not be reconnected across the trimmed span.
          for (
            let fragment = 0;
            fragment < Math.max(1, characterBoundary.length, projectileBoundary.length);
            fragment++
          )
            out.masks!.push({
              ...mask,
              id: `mask-${out.masks!.length}`,
              node: "$root",
              triangles,
              alphaCoverage: mask.alphaCoverage && {
                textures: mask.alphaCoverage.textures,
                triangles: alphaCoverage.get(repeat)!,
              },
              anchor,
              receiverPoints: receiverPoints?.map((p) => warp(p, repeat)),
              receiverSegment: receiver || receiverPoints ? undefined : receiverSegment,
              receiverPolyline:
                receiver && receiverFragments.length === 1
                  ? receiverFragments[0]!.map((p) => warp(p, repeat))
                  : undefined,
              receiverPolylines:
                receiverFragments.length > 1
                  ? receiverFragments.map((line) => line.map((p) => warp(p, repeat)))
                  : undefined,
              view: fragment === 0 && mask.view,
              obstacles: fragment === 0 ? obstacles : [],
              characterBoundary: characterBoundary[fragment],
              characterBoundaryClosed: characterBoundary[fragment]
                ? mask.characterBoundaryClosed
                : undefined,
              projectileBoundary: projectileBoundary[fragment],
              projectileBoundaryClosed: projectileBoundary[fragment]
                ? mask.projectileBoundaryClosed
                : undefined,
            });
        }
      }
      for (const region of data.materials ?? []) {
        pieces(
          region.polygon.map((p) => source(region.node, p)),
          (vertices, repeat) => {
            const world = vertices.map((p) => warp([p[0]!, p[1]!, p[2]!], repeat));
            const projected = world.map(([x, y, z]): [number, number] => [
              x - imageOrigin[0]!,
              y - z - imageOrigin[1]!,
            ]);
            if (
              !quantizeGeneratedMotionPolygon(
                [projected],
                Math.round,
                `Wall spline ${path.id}, material ${region.id}`,
                warnings,
              )
            )
              return;
            const id = `material-${materialSequence++}`;
            remember(materialRefs, region.id, repeat, id);
            out.materials!.push({
              id,
              node: "$root",
              polygon: world,
              material: region.material,
              ground: region.ground,
              obstacles: region.obstacles.flatMap(
                (owner) => volumeRefs.get(owner)?.get(repeat) ?? [],
              ),
            });
          },
          true,
        );
      }
      const automaticLights: { light: AssetLightRegion; sources: Set<string>; repeat: number }[] =
        [];
      const surfaceOrigins = new Map<string, { source: string; repeat: number }>();
      const sourceProjectionPlane = (points: Vec3[]) =>
        heightPlane(
          points.map((point): Vec3 => {
            const [x, y, z] = sceneToGame(document.camera, point);
            return [x, y - z, z];
          }),
        );
      const sourcePlanes = data.lights?.some(
        (light) => !light.receivers && !light.receiverSegments && !light.receiverPolylines,
      )
        ? data.surfaces.map((surface) => ({
            id: surface.id,
            plane: sourceProjectionPlane(
              surface.polygon.map(([x, y], i) =>
                source(surface.node, [
                  x,
                  y,
                  typeof surface.height === "number" ? surface.height : surface.height[i]!,
                ]),
              ),
            ),
          }))
        : [];
      for (const light of data.lights ?? []) {
        const explicit = light.receivers || light.receiverSegments || light.receiverPolylines;
        const sourcePlane = !explicit
          ? sourceProjectionPlane(light.polygon.map((p) => source(light.node, p)))
          : undefined;
        const sourceReceivers = new Set(
          sourcePlane
            ? sourcePlanes
                .filter(({ plane }) => plane.every((n, i) => Math.abs(n - sourcePlane[i]!) < 1e-7))
                .map(({ id }) => id)
            : [],
        );
        const group = `light-group-${lightGroupSequence++}`;
        const cropped = new Set<number>();
        pieces(
          light.polygon.map((p) => source(light.node, p)),
          (vertices, repeat) => {
            const receivers = light.receivers
              ?.map((p) => source(light.node, p))
              .filter(
                (p) =>
                  !run ||
                  (p[axis] >= start &&
                    p[axis] <=
                      Math.min(end, start + (end - start) * (length / run.repeatLength - repeat))),
              );
            const hasProbes = light.receivers || light.receiverSegments || light.receiverPolylines;
            const receiverPolylines = [
              ...(light.receiverSegments ?? []),
              ...(light.receiverPolylines ?? []),
            ].flatMap((line) => {
              const points = line.map((p) => source(light.node, p));
              const fragments = run
                ? clipSplinePolyline(
                    points,
                    axis,
                    start,
                    Math.min(end, start + (end - start) * (length / run.repeatLength - repeat)),
                    stations,
                  )
                : [points];
              // Keep each fragment separate so trimming cannot invent a receiver.
              return fragments
                .filter((line) => line.length > 1)
                .map((line) => line.map((p) => warp(p, repeat)));
            });
            if (hasProbes && !receivers?.length && !receiverPolylines.length) {
              if (!cropped.has(repeat))
                warnings.push(
                  `Wall spline ${path.id}, light ${light.id}, repeat ${repeat}: cropping removed every receiving anchor; light region omitted.`,
                );
              cropped.add(repeat);
              return;
            }
            const polygon = vertices.map((p) => warp([p[0]!, p[1]!, p[2]!], repeat));
            const projected = polygon.map(([x, y, z]): [number, number] => [
              x - imageOrigin[0]!,
              y - z - imageOrigin[1]!,
            ]);
            if (
              !quantizeGeneratedMotionPolygon(
                [projected],
                Math.round,
                `Wall spline ${path.id}, light ${light.id}`,
                warnings,
              )
            )
              return;
            out.lights!.push({
              id: `light-${lightSequence++}`,
              node: "$root",
              polygon,
              ambiences: light.ambiences,
              ...(hasProbes ? { receiverGroup: `${group}-${repeat}` } : {}),
              ...(receiverPolylines.length ? { receiverPolylines } : {}),
              ...(receivers?.length
                ? {
                    // Horizontal source deformation is quantized to 1/1024 game
                    // units. A narrow vertical probe tolerates the corresponding
                    // receiving-plane rounding without selecting another floor.
                    receiverSegments: receivers.map((p): [Vec3, Vec3] => {
                      const [x, y, z] = warp(p, repeat);
                      return [
                        [x, y - 1 / 1024, z - 1 / 1024],
                        [x, y + 1 / 1024, z + 1 / 1024],
                      ];
                    }),
                  }
                : {}),
            });
            if (sourceReceivers.size)
              automaticLights.push({
                light: out.lights!.at(-1)!,
                sources: sourceReceivers,
                repeat,
              });
          },
        );
      }
      for (const sound of data.sounds ?? []) {
        if (!sound.spatial) {
          warnings.push(
            `Wall spline ${path.id}, sound ${sound.id}: global emitters cannot be repeated along a wall; emitter omitted.`,
          );
          continue;
        }
        const points = sound.spatial.polyline.map((p) => source(sound.node, p));
        for (let repeat = 0; repeat < repeats; repeat++) {
          const fragments = run
            ? clipSplinePolyline(
                points,
                axis,
                start,
                Math.min(end, start + (end - start) * (length / run.repeatLength - repeat)),
                stations,
              )
            : [points];
          if (fragments.length > 1) {
            warnings.push(
              `Wall spline ${path.id}, sound ${sound.id}, repeat ${repeat}: cropping produces disconnected emitter fragments; emitter omitted.`,
            );
            continue;
          }
          if (!fragments.length) continue;
          out.sounds!.push({
            ...sound,
            id: `sound-${out.sounds!.length}`,
            node: "$root",
            spatial: { ...sound.spatial, polyline: fragments[0]!.map((p) => warp(p, repeat)) },
          });
        }
      }
      const surfaceSet = `span-${out.surfaces.length}`;
      const appendSurface = (surface: AssetWalkableSurface, target: AssetWalkableSurface[]) => {
        if (surface.navigationHeight !== undefined) {
          report(
            `Wall spline ${path.id}, clearance ${surface.id}: separate physical and navigation heights are unsupported; clearance omitted, collision retained.`,
          );
          return;
        }
        const local = surface.polygon.map(([x, y], i): Vec3 => [
          x,
          y,
          typeof surface.height === "number" ? surface.height : surface.height[i]!,
        ]);
        const plane = heightPlane(local);
        const rings = [
          local,
          ...(surface.holes ?? []).map((h) =>
            h.map(([x, y]): Vec3 => [x, y, planeHeight(plane, [x, y])]),
          ),
        ];
        const flat = flatten(rings.map((r) => r.map((p) => source(surface.node, p)))),
          ids = earcut(flat.vertices, flat.holes, 3);
        const varyingHeight =
          flat.vertices.some((n, i) => i % 3 === 2 && Math.abs(n - flat.vertices[2]!) > 1e-7) ||
          run?.points.some((p) => Math.abs(p[2] - run.points[0]![2]) > 1e-7);
        const navigationRegion =
          surface.navigationRegion ?? (varyingHeight ? surface.id : undefined);
        for (let i = 0; i < ids.length; i += 3)
          pieces(
            ids.slice(i, i + 3).map((j) => flat.vertices.slice(j * 3, j * 3 + 3)),
            (vertices, repeat) => {
              const world = vertices.map((v) => warp([v[0]!, v[1]!, v[2]!], repeat));
              if (Math.abs(area(world[0]!, world[1]!, world[2]!)) < 1e-5) return;
              target.push({
                id: `${target === out.surfaces ? "surface" : target === out.movementBlockers ? "blocker" : "clearance"}-${target.length}`,
                node: "$root",
                polygon: world.map((p) => [p[0], p[1]]),
                height: world.map((p) => p[2]),
                preserveMovementPrecision: true,
                ...(target === out.surfaces && navigationRegion !== undefined
                  ? {
                      navigationRegion: `${surfaceSet}/${navigationRegion}`,
                    }
                  : {}),
                ...(surface.projectionMaterials
                  ? {
                      projectionMaterials: {
                        defaultMaterial: surface.projectionMaterials.defaultMaterial,
                        regions: surface.projectionMaterials.regions.flatMap(
                          (id) => materialRefs.get(id)?.get(repeat) ?? [],
                        ),
                      },
                    }
                  : {}),
              });
              if (target === out.surfaces)
                surfaceOrigins.set(target.at(-1)!.id, { source: surface.id, repeat });
            },
          );
      };
      for (const surface of data.surfaces) appendSurface(surface, out.surfaces);
      for (const surface of data.movementBlockers ?? [])
        appendSurface(surface, out.movementBlockers!);
      for (const surface of data.movementClearances ?? [])
        appendSurface(surface, out.movementClearances!);
      const omittedLights = new Set<AssetLightRegion>();
      for (const { light, sources, repeat } of automaticLights) {
        const candidates = out.surfaces.filter((surface) => {
          const origin = surfaceOrigins.get(surface.id);
          return origin?.repeat === repeat && sources.has(origin.source);
        });
        const probes = splineLightReceivers(light, candidates, imageOrigin);
        if (probes.length) light.receiverSegments = probes;
        else {
          warnings.push(
            `Wall spline ${path.id}, light ${light.id}: no deformed source receiver overlaps the exported contour; light region omitted.`,
          );
          omittedLights.add(light);
        }
      }
      out.lights = out.lights!.filter((light) => !omittedLights.has(light));
      // Trimming may remove every owner of a source material region.
      out.materials = out.materials!.filter(
        (region) =>
          region.ground ||
          region.obstacles.length ||
          out.surfaces.some((surface) => surface.projectionMaterials?.regions.includes(region.id)),
      );
      validateAssetGameplay(out, generated);
    }
    for (const run of wallRuns(path, document.camera)) {
      const before = structuredClone(out);
      try {
        append(path.asset, run);
      } catch (error) {
        Object.assign(out, before);
        report(`Wall spline ${path.id}: ${String(error)}; this span's gameplay is omitted.`);
      }
    }
    for (const corner of wallCorners(path, document.camera)) {
      const before = structuredClone(out);
      try {
        append(path.cornerAsset, undefined, corner);
      } catch (error) {
        Object.assign(out, before);
        report(
          `Wall spline ${path.id}, corner ${corner.index}: ${String(error)}; corner gameplay is omitted.`,
        );
      }
    }
    result.push(generated);
  }
  return { descriptors: result, warnings: [...new Set(warnings)] };
}
