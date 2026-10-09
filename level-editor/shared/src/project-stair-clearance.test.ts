import test from "node:test";
import assert from "node:assert/strict";
import { projectStairClearance } from "./project-stair-clearance.ts";
import { heightPlane, type HeightPlane } from "./gameplay-plane.ts";
import { pointInGameplayPolygon } from "./navigation-anchor.ts";
import type { Point } from "./level.ts";
import type { Vec3 } from "./scene.ts";
import { compilePhysicalStairArea } from "./compile-physical-stair.ts";

const corners: Point[] = [
  [-5, -2],
  [5, -2],
  [5, 2],
  [-5, 2],
];
const fits = (center: Point, boundary: Point[]) =>
  corners.every(([x, y]) => pointInGameplayPolygon([center[0] + x, center[1] + y], boundary));

test("projected stair collision admits only physically supported movement footprints", () => {
  let corrected = 0;
  for (const degrees of [0, 37, 90, 137, 180, 270])
    for (const elevation of [0, 40]) {
      const angle = (degrees * Math.PI) / 180;
      const input: Vec3[] = [
        [-30, -60, 0],
        [30, -60, 0],
        [30, 60, 115],
        [-30, 60, 115],
      ];
      const vertices = input.map(([x, y, z]): Vec3 => [
        500 + x * Math.cos(angle) - y * Math.sin(angle),
        500 + x * Math.sin(angle) + y * Math.cos(angle),
        z + elevation,
      ]);
      const plane = heightPlane(vertices);
      const boundary = vertices.map(([x, y]): Point => [x, y]);
      const compiled = projectStairClearance(boundary, plane);
      if (!compiled) continue;
      corrected++;
      const [a, b, c] = plane;
      let checked = 0;
      for (let x = 400; x <= 600; x += 1)
        for (let y = 280; y <= 620; y += 0.25) {
          if (!fits([x, y], compiled)) continue;
          const physical: Point = [x, (y + a * x + c) / (1 - b)];
          assert.ok(fits(physical, boundary), `${degrees}/${elevation}: unsupported ${physical}`);
          checked++;
        }
      assert.ok(checked > 0, `${degrees}/${elevation}: usable floor was lost`);
      const compare = (p: Point, q: Point) => p[0] - q[0] || p[1] - q[1];
      assert.deepEqual(
        projectStairClearance([...boundary].reverse(), plane)?.toSorted(compare),
        compiled.toSorted(compare),
      );
    }
  assert.ok(corrected > 0, "at least one projected floor must require recovery");
});

test("flat, singular and usable nonconvex floors keep their existing representation", () => {
  const square: Point[] = [
    [0, 0],
    [50, 0],
    [50, 50],
    [0, 50],
  ];
  for (const plane of [
    [0, 0, 40],
    [0, 0.5, 0],
    [0, 1, 0],
  ] satisfies HeightPlane[])
    assert.equal(projectStairClearance(square, plane), undefined);
  assert.equal(
    projectStairClearance(
      [
        [0, 0],
        [50, 0],
        [20, 20],
        [50, 50],
        [0, 50],
      ],
      [0, 0.5, 0],
    ),
    undefined,
  );
});

test("collapsed concave stairs retain only a supported inner kernel", () => {
  const boundary: Point[] = [
    [512.0663294384332, 457.5641487248501],
    [517.6603996163185, 447.2436882251212],
    [520.4523779360266, 442.0927871730473],
    [550.6873579004468, 447.8252400245971],
    [566.3155681654293, 450.70217716141696],
    [557.9194061293663, 466.19219710880094],
  ];
  const plane: HeightPlane = [2.473787689559903, 1.3408841534826745, -1835.2838912296484];
  const projected = projectStairClearance(boundary, plane);
  assert.ok(projected);
  const [a, b, c] = plane;
  let checked = 0;
  for (let x = 480; x <= 600; x++)
    for (let y = 225; y <= 470; y++) {
      if (!fits([x, y], projected)) continue;
      assert.ok(fits([x, (y + a * x + c) / (1 - b)], boundary));
      checked++;
    }
  assert.ok(checked > 0);
});

test("clearance recovery retains physical receivers and excludes holes and barriers", () => {
  const rectangle = (low: number, high: number): Vec3[] =>
    [
      [low, low],
      [high, low],
      [high, high],
      [low, high],
    ].map(([x, y]): Vec3 => [x!, y!, y! * 0.95]);
  for (const kind of ["clear", "hole", "obstacle"] as const) {
    const input = {
      surfaces: [{ polygon: rectangle(0, 100), holes: kind === "hole" ? [rectangle(40, 60)] : [] }],
      doors: [],
      obstacles: kind === "obstacle" ? [{ stateId: 1, polygon: rectangle(40, 60) }] : [],
    };
    const ordinary = compilePhysicalStairArea(input);
    const prepared = compilePhysicalStairArea({ ...input, prepareWalkingClearance: true });
    assert.deepEqual(prepared.navigation, ordinary.navigation);
    if (kind === "clear") assert.notDeepEqual(prepared.area, ordinary.area);
    else assert.deepEqual(prepared.area, ordinary.area);
  }
});
