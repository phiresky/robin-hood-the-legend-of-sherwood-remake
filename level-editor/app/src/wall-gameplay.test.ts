import test from "node:test";
import assert from "node:assert/strict";
import { BoxGeometry, Group, Mesh, MeshBasicMaterial, Vector3 } from "three";
import {
  wallSplineFixture,
  wallMaterialFixture,
  wallDisconnectedMaskFixture,
  wallDisconnectedLightFixture,
  wallAutomaticLightFixture,
} from "../../shared/test-fixtures/wall-spline.ts";
import { wallSplineGameplay } from "../../shared/src/wall-spline-gameplay.ts";
import { sceneToGame } from "../../shared/src/geometry.ts";
import { heightPlane, planeHeight } from "../../shared/src/gameplay-plane.ts";
import { wallMesh } from "./spline-geometry.ts";
import { readFile } from "node:fs/promises";
import { compileMap } from "./map-compile.ts";
import {
  prepareWallGameplayAssets,
  prepareWallGameplayAssetsAsync,
} from "./wall-gameplay-calibration.ts";
import { parseStoredMap } from "@rle/shared";
import {
  validateAssetGameplay,
  type GameplayAssetDescriptor,
} from "../../shared/src/asset-gameplay.ts";

test("wall export matches the fixture exercised by native routing and sight", async () => {
  const f = wallSplineFixture();
  const expected = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-spline-wall.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(f.document, f.bounds, f.assets).descriptor, expected);
});

test("wall materials match the fixture exercised by native material queries", async () => {
  const f = wallMaterialFixture();
  const expected = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-spline-material.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(f.document, f.bounds, f.assets).descriptor, expected);
  const split = wallDisconnectedMaskFixture();
  assert.deepEqual(compileMap(split.document, split.bounds, split.assets).descriptor, expected);
  const lighting = wallDisconnectedLightFixture();
  assert.deepEqual(
    compileMap(lighting.document, lighting.bounds, lighting.assets).descriptor,
    expected,
  );
});

test("automatic curved wall lighting matches the native elevated shadow fixture", async () => {
  const f = wallAutomaticLightFixture();
  const expected = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-spline-auto-light.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(f.document, f.bounds, f.assets).descriptor, expected);
});

test("curved, trimmed, sloping and flipped wall collision covers the corresponding rendered wall", () => {
  const { document, assets } = wallSplineFixture();
  const geometry = new BoxGeometry(100, 20, 40).translate(0, 0, 20),
    material = new MeshBasicMaterial();
  const sources = new Map([["asset:wall:body", new Mesh(geometry, material)]]);
  sources.get("asset:wall:body")!.rotation.z = 0.13;
  sources.get("asset:wall:body")!.position.set(25, 17, 4);
  const before = JSON.stringify([...assets]);
  for (const sourceAngle of [0, 31, -82.4])
    for (const sourceStraight of [false, true])
      for (const curved of [false, true])
        for (const flipCrossSection of [false, true]) {
          const path = {
            ...document.splines![0]!,
            curved,
            flipCrossSection,
            sourceAngle,
            sourceStraight,
            sourceStart: 0.2,
            sourceEnd: 0.85,
            points: [
              [80, 180, 0],
              [210, 280, 20],
              [370, 210, 60],
            ] as [number, number, number][],
          };
          document.splines = [path];
          const prepared = prepareWallGameplayAssets(document, assets, sources);
          assert.deepEqual(prepared.warnings, []);
          const compiled = wallSplineGameplay(document, prepared.assets, false);
          assert.deepEqual(compiled.warnings, []);
          const shapes = compiled.descriptors[0]!.gameplay!.volumes!.map((v) => {
            const points = v.shape.points;
            return {
              points,
              top: heightPlane(points.map((p) => [p.x, p.y, p.z_top])),
              bottom: heightPlane(points.map((p) => [p.x, p.y, p.z_bottom])),
            };
          });
          const wall = wallMesh(path, document.camera, sources);
          wall.updateMatrixWorld(true);
          let checked = 0;
          wall.traverse((node) => {
            if (!(node instanceof Mesh)) return;
            const attribute = node.geometry.getAttribute("position");
            for (let i = 0; i < attribute.count; i++) {
              const v = new Vector3()
                .fromBufferAttribute(attribute, i)
                .applyMatrix4(node.matrixWorld);
              const [x, y, z] = sceneToGame(document.camera, v.toArray());
              const near: { horizontal: number; below: number; above: number; points: unknown }[] =
                [];
              const contained = shapes.some(({ points, top, bottom }) => {
                const signs = points.map((a, j) => {
                  const b = points[(j + 1) % points.length]!;
                  return (
                    ((b.x - a.x) * (y - a.y) - (b.y - a.y) * (x - a.x)) /
                    Math.hypot(b.x - a.x, b.y - a.y)
                  );
                });
                near.push({
                  horizontal: Math.min(Math.max(...signs), -Math.min(...signs)),
                  below: planeHeight(bottom, [x, y]) - z,
                  above: z - planeHeight(top, [x, y]),
                  points,
                });
                return (
                  (signs.every((n) => n >= -0.002) || signs.every((n) => n <= 0.002)) &&
                  z >= planeHeight(bottom, [x, y]) - 0.001 &&
                  z <= planeHeight(top, [x, y]) + 0.001
                );
              });
              if (!contained)
                assert.fail(
                  `rendered vertex ${[x, y, z]} has matching physical geometry (angle=${sourceAngle}, straight=${sourceStraight}, curved=${curved}, flipped=${flipCrossSection}); closest=${JSON.stringify(near.sort((a, b) => Math.max(a.horizontal, a.below, a.above) - Math.max(b.horizontal, b.below, b.above)).slice(0, 2))}`,
                );
              checked++;
            }
            node.geometry.dispose();
          });
          assert.ok(checked > 100);
        }
  geometry.dispose();
  material.dispose();
  assert.equal(JSON.stringify([...assets]), before);
});

test("invalid source sections do not discard other calibrated paths using the same asset", () => {
  const { document, assets } = wallSplineFixture();
  const body = new Group();
  for (const x of [-40, 40]) body.add(new Mesh(new BoxGeometry(20, 20, 40).translate(x, 0, 20)));
  const path = { ...document.splines![0]!, sourceStraight: false };
  document.splines = [path, { ...path, id: "trimmed", sourceEnd: 0.2 }];
  const prepared = prepareWallGameplayAssets(
    document,
    assets,
    new Map([["asset:wall:body", body]]),
  );
  assert.equal(prepared.warnings.length, 1);
  assert.match(prepared.warnings[0]!, /wall-path.*gap/);
  const result = wallSplineGameplay(document, prepared.assets, true);
  assert.equal(result.warnings.length, 1);
  assert.equal(result.descriptors[0]!.gameplay!.volumes!.length, 0);
  assert.ok(result.descriptors[1]!.gameplay!.volumes!.length);
  body.traverse((node) => {
    if (node instanceof Mesh) {
      node.geometry.dispose();
      (node.material as MeshBasicMaterial).dispose();
    }
  });
});

test("prepared wall calibration validates section coordinates and does not hide missing models", () => {
  const { document, assets } = wallSplineFixture();
  document.splines![0]!.sourceStraight = false;
  const mesh = new Mesh(new BoxGeometry(100, 20, 40));
  const prepared = prepareWallGameplayAssets(
    document,
    assets,
    new Map([["asset:wall:body", mesh]]),
  );
  assert.deepEqual(prepared.warnings, []);
  const asset = prepared.assets.get("wall") as GameplayAssetDescriptor;
  for (const kind of ["width", "range", "sections", "angle"] as const) {
    const gameplay = structuredClone(asset.gameplay!);
    const deformation = gameplay.spline!.deformations![0]!;
    if (kind === "width") deformation.profile!.sections[0]!.width = 0;
    if (kind === "range") deformation.profile!.start += 1;
    if (kind === "sections") deformation.profile!.sections.pop();
    if (kind === "angle") deformation.sourceAngle = NaN;
    assert.throws(() => validateAssetGameplay(gameplay, asset), /spline/);
  }
  const missing = prepareWallGameplayAssets(document, assets, new Map());
  assert.match(missing.warnings[0]!, /mesh is not loaded/);
  assert.equal(missing.assets.get("wall"), assets.get("wall"));
  mesh.geometry.dispose();
  (mesh.material as MeshBasicMaterial).dispose();
});

test("rotated wall export reopens with the original asset pins and recalibrates identically", () => {
  const { document, assets, bounds } = wallSplineFixture();
  Object.assign(document.splines![0]!, {
    sourceAngle: 23,
    sourceStraight: false,
    sourceStart: 0.17,
    sourceEnd: 0.83,
  });
  const mesh = new Mesh(new BoxGeometry(100, 20, 40).translate(0, 0, 20));
  const sources = new Map([["asset:wall:body", mesh]]);
  const initial = prepareWallGameplayAssets(document, assets, sources);
  const compiled = compileMap(document, bounds, initial.assets);
  const reopened = parseStoredMap(compiled.editorDocument, assets);
  assert.deepEqual(reopened.assetSources, document.assetSources);
  const next = prepareWallGameplayAssets(reopened, assets, sources);
  assert.deepEqual(next.warnings, []);
  assert.deepEqual(compileMap(reopened, bounds, next.assets).descriptor, compiled.descriptor);
  assert.equal(compiled.descriptor.spawn_points.length, 0);
  mesh.geometry.dispose();
  (mesh.material as MeshBasicMaterial).dispose();
});

test("wall calibration yields between sources and sections and propagates cancellation", async () => {
  const { document, assets } = wallSplineFixture();
  const mesh = new Mesh(new BoxGeometry(100, 20, 40));
  const sources = new Map([["asset:wall:body", mesh]]);
  let frames = 0;
  const result = await prepareWallGameplayAssetsAsync(document, assets, sources, async () => {
    frames++;
  });
  assert.equal(frames, 2);
  assert.deepEqual(result, prepareWallGameplayAssets(document, assets, sources));
  await assert.rejects(
    prepareWallGameplayAssetsAsync(document, assets, sources, async () => {
      throw new Error("cancelled");
    }),
    /cancelled/,
  );
  mesh.geometry.dispose();
  (mesh.material as MeshBasicMaterial).dispose();
});
