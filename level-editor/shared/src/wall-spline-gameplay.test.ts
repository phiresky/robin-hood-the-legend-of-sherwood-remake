import test from "node:test";
import assert from "node:assert/strict";
import { wallSplineFixture } from "../test-fixtures/wall-spline.ts";
import { wallSplineGameplay } from "./wall-spline-gameplay.ts";
import { compileAssetGameplay } from "./compile-asset-gameplay.ts";
import { validateAssetGameplay } from "./asset-gameplay.ts";
import { sceneToGame } from "./geometry.ts";

test("spline materials retain vertical faces and receiver ownership after moving and repeating", () => {
  const { document, asset, assets, bounds } = wallSplineFixture();
  const local = (x: number, y: number, z: number) => sceneToGame(document.camera, [x, y, z]);
  const top = [
    [-50, -10],
    [50, -10],
    [50, 10],
    [-50, 10],
  ].map(([x, y]) => local(x!, y!, 40));
  asset.gameplay!.materials = [
    {
      id: "face",
      node: "body",
      polygon: [local(-50, -10, 0), local(50, -10, 0), local(50, -10, 40), local(-50, -10, 40)],
      material: 4,
      ground: false,
      obstacles: ["body-solid"],
    },
    { id: "top", node: "body", polygon: top, material: 2, ground: false, obstacles: [] },
    {
      id: "ground",
      node: "body",
      polygon: [local(-40, -8, 0), local(0, -8, 0), local(0, 8, 0), local(-40, 8, 0)],
      material: 1,
      ground: true,
      obstacles: [],
    },
  ];
  asset.gameplay!.surfaces = [
    {
      id: "walkway",
      node: "body",
      polygon: top.map(([x, y]) => [x, y]),
      height: top[0]![2],
      projectionMaterials: { defaultMaterial: 3, regions: ["top"] },
    },
  ];
  document.splines![0]!.points = [
    [130, 260, 0],
    [345, 260, 0],
  ];
  const before = JSON.stringify([document, asset]);
  const result = wallSplineGameplay(document, assets, false);
  const generated = result.descriptors[0]!.gameplay!;
  assert.ok(generated.materials!.some((region) => region.material === 4));
  assert.ok(generated.materials!.some((region) => region.material === 2));
  assert.equal(
    new Set(generated.materials!.map((region) => region.id)).size,
    generated.materials!.length,
  );
  for (const region of generated.materials!) {
    assert.ok(region.polygon.every(([x]) => x >= 130 && x <= 345));
    if (region.material === 4) {
      assert.ok(region.obstacles.length > 0);
      assert.ok(region.obstacles.every((id) => generated.volumes!.some((v) => v.id === id)));
    } else if (!region.ground) {
      assert.ok(
        generated.surfaces.some((surface) =>
          surface.projectionMaterials?.regions.includes(region.id),
        ),
      );
    }
  }
  const compiled = compileAssetGameplay(document, assets, bounds);
  assert.ok(compiled.material_sectors!.some((region) => region.material === 4));
  assert.ok(compiled.material_sectors!.some((region) => region.material === 2));
  assert.ok(compiled.sight_material_indices!.length > 0);
  assert.ok(
    compiled.sight_material_indices!.every(
      (index) => compiled.material_sectors![index]!.material === 1,
    ),
  );
  assert.equal(JSON.stringify([document, asset]), before);
  for (const points of [
    [
      [130, 260, 0],
      [130, 45, 0],
    ],
    [
      [130, 260, 0],
      [280, 190, 20],
      [360, 290, 40],
    ],
  ]) {
    document.splines![0]!.points = points.map(([x, y, z]) => [x!, y!, z!]);
    document.splines![0]!.cornerAsset = "wall";
    const turned = wallSplineGameplay(document, assets, false);
    for (const descriptor of turned.descriptors)
      validateAssetGameplay(descriptor.gameplay, descriptor);
    for (const origin of [
      [0, 0],
      [0.3, -0.3],
    ]) {
      const result = compileAssetGameplay(document, assets, [
        origin[0]!,
        origin[1]!,
        bounds[2],
        bounds[3],
      ]);
      assert.ok(result.material_sectors!.some((region) => region.material === 2));
      assert.ok(
        result.sight_obstacles.some((obstacle) =>
          obstacle.material_indices.some(
            (index) => result.material_sectors![index]!.material === 2,
          ),
        ),
      );
    }
  }
});

test("wall lighting follows repeated and turned paths and preserves ambience filters", () => {
  const { document, asset, assets, bounds } = wallSplineFixture();
  asset.gameplay!.lights = [
    {
      id: "shadow",
      node: "body",
      ambiences: 5,
      polygon: [
        [-40, -10, 0],
        [40, -10, 0],
        [40, 10, 0],
        [-40, 10, 0],
      ].map(([x, y, z]) => sceneToGame(document.camera, [x!, y!, z!])),
    },
  ];
  const before = JSON.stringify(asset);
  for (const points of [
    [
      [100, 200, 0],
      [345, 200, 0],
    ],
    [
      [130, 300, 0],
      [130, 70, 0],
    ],
    [
      [100, 200, 0],
      [240, 250, 0],
      [390, 100, 0],
    ],
  ]) {
    document.splines![0]!.points = points.map(([x, y, z]) => [x!, y!, z!]);
    const generated = wallSplineGameplay(document, assets, false);
    assert.deepEqual(generated.warnings, []);
    const result = compileAssetGameplay(document, assets, bounds);
    assert.ok(result.light_sectors!.length >= 4);
    assert.ok(result.light_sectors!.every((light) => light.layer === 0 && light.ambience === 5));
  }
  assert.equal(JSON.stringify(asset), before);
  const top = [
    [-50, -10, 40],
    [50, -10, 40],
    [50, 10, 40],
    [-50, 10, 40],
  ].map(([x, y, z]) => sceneToGame(document.camera, [x!, y!, z!]));
  asset.gameplay!.surfaces = [
    { id: "top", node: "body", polygon: top.map(([x, y]) => [x, y]), height: top[0]![2] },
  ];
  asset.gameplay!.lights[0]!.polygon = top;
  document.splines![0]!.points = [
    [100, 200, 20],
    [400, 200, 80],
  ];
  const raised = compileAssetGameplay(document, assets, bounds);
  assert.ok(raised.light_sectors!.length);
  assert.ok(raised.light_sectors!.every((light) => light.layer > 0 && light.ambience === 5));
  asset.gameplay!.lights[0]!.receivers = [sceneToGame(document.camera, [0, 0, 0])];
  const unsupported = wallSplineGameplay(document, assets, true);
  assert.ok(unsupported.warnings.some((warning) => warning.includes("explicit receiving anchors")));
  assert.equal(unsupported.descriptors[0]!.gameplay!.lights!.length, 0);
});

test("wall spatial sounds repeat and crop with their acoustic rules intact", () => {
  const { document, asset, assets, bounds } = wallSplineFixture();
  asset.gameplay!.sounds = [
    {
      id: "wind",
      node: "body",
      sample: 12,
      kind: 2,
      active: true,
      delay: [10, 20, 2],
      altitude: 3,
      ambiences: 5,
      spatial: {
        polyline: [
          [-50, 0, 0],
          [50, 0, 0],
        ].map(([x, y, z]) => sceneToGame(document.camera, [x!, y!, z!])),
        innerDistance: 10,
        outerDistance: 60,
        innerVolume: 80,
        outerVolume: 0,
        noiseCoveringDistance: 15,
      },
    },
  ];
  document.splines![0]!.points = [
    [100, 200, 0],
    [345, 200, 0],
  ];
  const before = JSON.stringify(asset);
  const result = compileAssetGameplay(document, assets, bounds);
  assert.equal(result.sound_sources!.length, 3);
  assert.deepEqual(
    result.sound_sources!.map((sound) => sound.polyline),
    [
      [
        [100, 200],
        [200, 200],
      ],
      [
        [200, 200],
        [300, 200],
      ],
      [
        [300, 200],
        [345, 200],
      ],
    ],
  );
  for (const sound of result.sound_sources!) {
    assert.deepEqual(sound.delayed_params, [10, 20, 2]);
    assert.equal(sound.altitude, 3);
    assert.equal(sound.ambience_filter, 5);
    assert.equal(sound.inner_volume, 80);
    assert.equal(sound.noise_covering_distance, 15);
  }
  assert.equal(JSON.stringify(asset), before);
  document.splines![0]!.curved = true;
  document.splines![0]!.points = [
    [100, 200, 0],
    [220, 300, 20],
    [345, 200, 40],
  ];
  const curved = wallSplineGameplay(document, assets, false);
  assert.deepEqual(curved.warnings, []);
  assert.ok(
    curved.descriptors[0]!.gameplay!.sounds!.some((sound) => sound.spatial!.polyline.length > 2),
  );
  assert.ok(compileAssetGameplay(document, assets, bounds).sound_sources!.length > 0);
  asset.gameplay!.sounds[0]!.spatial = undefined;
  assert.ok(
    wallSplineGameplay(document, assets, true).warnings.some((warning) =>
      warning.includes("global emitters"),
    ),
  );
});

test("wall collision follows moved paths, crops repeats and participates in terrain navigation", () => {
  const { document, assets, bounds } = wallSplineFixture();
  const first = compileAssetGameplay(document, assets, bounds);
  assert.equal(first.warnings?.length ?? 0, 0);
  assert.ok(first.sight_obstacles.filter((s) => s.solid).length >= 6);
  const points = first.sight_obstacles.filter((s) => s.solid).flatMap((s) => s.points);
  assert.equal(Math.min(...points.map((p) => p.x)), 100);
  assert.equal(Math.max(...points.map((p) => p.x)), 400);
  document.splines![0]!.points = [
    [130, 260, 0],
    [345, 260, 0],
  ];
  const second = compileAssetGameplay(document, assets, bounds);
  const changed = second.sight_obstacles.filter((s) => s.solid).flatMap((s) => s.points);
  assert.equal(Math.min(...changed.map((p) => p.x)), 130);
  assert.equal(Math.max(...changed.map((p) => p.x)), 345);
  assert.ok(changed.every((p) => p.y > 250 && p.y < 270));
  assert.notDeepEqual(first.motion_data, second.motion_data);
});

test("a raised wall keeps its underpass and opaque and solid flags independent", () => {
  const { document, assets, asset, bounds } = wallSplineFixture();
  asset.gameplay!.volumes![0]!.shape.opaque = false;
  document.splines![0]!.points = [
    [100, 200, 80],
    [400, 200, 80],
  ];
  const raised = compileAssetGameplay(document, assets, bounds);
  const empty = compileAssetGameplay({ ...document, splines: [] }, assets, bounds);
  assert.deepEqual(raised.motion_data, empty.motion_data);
  assert.ok(
    raised.sight_obstacles
      .filter((s) => s.solid)
      .every((s) => !s.opaque && s.points.every((p) => p.z_bottom === 80)),
  );
});

test("calibrated wall-top surfaces and tower corners survive deformation", () => {
  const { document, asset, assets, bounds } = wallSplineFixture();
  const shape = asset.gameplay!.volumes![0]!.shape;
  asset.gameplay!.surfaces = [
    {
      id: "walkway",
      node: "body",
      polygon: shape.points.map((p) => [p.x, p.y]),
      height: shape.points[0]!.z_top,
    },
  ];
  document.splines![0]!.points = [
    [100, 100, 0],
    [300, 100, 0],
    [300, 300, 0],
  ];
  document.splines![0]!.cornerAsset = "wall";
  document.splines![0]!.cornerScale = 1.2;
  const walls = wallSplineGameplay(document, assets, false);
  assert.deepEqual(walls.warnings, []);
  for (const descriptor of walls.descriptors)
    validateAssetGameplay(descriptor.gameplay, descriptor);
  const result = compileAssetGameplay(document, assets, bounds);
  assert.ok(result.sight_obstacles.some((s) => s.points.some((p) => p.z_top > 39)));
  assert.ok(result.motion_data.layers.length > 1);
});

test("missing calibration warns without removing other wall spans or corrupting input", () => {
  const { document, asset, assets } = wallSplineFixture();
  document.splines!.push({ ...document.splines![0]!, id: "missing", asset: "unknown" });
  const before = JSON.stringify([document, asset]);
  assert.throws(() => wallSplineGameplay(document, assets, false), /calibration/);
  const result = wallSplineGameplay(document, assets, true);
  assert.equal(result.warnings.length, 1);
  assert.ok(result.descriptors[0]!.gameplay!.volumes!.length);
  assert.equal(JSON.stringify([document, asset]), before);
});
