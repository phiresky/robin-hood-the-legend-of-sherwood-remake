import test from "node:test";
import assert from "node:assert/strict";
import { BoxGeometry, Mesh, MeshBasicMaterial, Vector3 } from "three";
import { wallSplineFixture } from "../../shared/test-fixtures/wall-spline.ts";
import { wallSplineGameplay } from "../../shared/src/wall-spline-gameplay.ts";
import { sceneToGame } from "../../shared/src/geometry.ts";
import { heightPlane, planeHeight } from "../../shared/src/gameplay-plane.ts";
import { wallMesh } from "./spline-geometry.ts";
import { readFile } from "node:fs/promises";
import { compileMap } from "./map-compile.ts";

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

test("curved, trimmed, sloping and flipped wall collision covers the corresponding rendered wall", () => {
  const { document, assets } = wallSplineFixture();
  const geometry = new BoxGeometry(100, 20, 40).translate(0, 0, 20),
    material = new MeshBasicMaterial();
  const sources = new Map([["asset:wall:body", new Mesh(geometry, material)]]);
  for (const curved of [false, true])
    for (const flipCrossSection of [false, true]) {
      const path = {
        ...document.splines![0]!,
        curved,
        flipCrossSection,
        sourceStart: 0.2,
        sourceEnd: 0.85,
        points: [
          [80, 180, 0],
          [210, 280, 20],
          [370, 210, 60],
        ] as [number, number, number][],
      };
      document.splines = [path];
      const compiled = wallSplineGameplay(document, assets, false);
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
          const v = new Vector3().fromBufferAttribute(attribute, i).applyMatrix4(node.matrixWorld);
          const [x, y, z] = sceneToGame(document.camera, v.toArray());
          const contained = shapes.some(({ points, top, bottom }) => {
            const signs = points.map((a, j) => {
              const b = points[(j + 1) % points.length]!;
              return (b.x - a.x) * (y - a.y) - (b.y - a.y) * (x - a.x);
            });
            return (
              (signs.every((n) => n >= -0.01) || signs.every((n) => n <= 0.01)) &&
              z >= planeHeight(bottom, [x, y]) - 0.001 &&
              z <= planeHeight(top, [x, y]) + 0.001
            );
          });
          assert.ok(
            contained,
            `rendered vertex ${[x, y, z]} has matching physical geometry (curved=${curved}, flipped=${flipCrossSection})`,
          );
          checked++;
        }
        node.geometry.dispose();
      });
      assert.ok(checked > 100);
    }
  geometry.dispose();
  material.dispose();
});
