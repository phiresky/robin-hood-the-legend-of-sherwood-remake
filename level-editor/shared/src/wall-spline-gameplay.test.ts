import test from "node:test";
import assert from "node:assert/strict";
import {
  wallSplineFixture,
  wallMaterialFixture,
  wallDisconnectedMaskFixture,
  wallDisconnectedLightFixture,
  wallDisconnectedBoundaryFixture,
  wallClosedBoundaryFixture,
} from "../test-fixtures/wall-spline.ts";
import { wallSplineGameplay } from "./wall-spline-gameplay.ts";
import { compileAssetGameplay } from "./compile-asset-gameplay.ts";
import { validateAssetGameplay } from "./asset-gameplay.ts";
import { sceneToGame } from "./geometry.ts";
import { splineCurve } from "./spline-sampling.ts";

test("closed mask boundaries split at cropped necks without reconnecting islands", () => {
  const { document, assets, bounds } = wallClosedBoundaryFixture();
  const generated = wallSplineGameplay(document, assets, false);
  assert.deepEqual(generated.warnings, []);
  const masks = generated.descriptors[0]!.gameplay!.masks!;
  assert.equal(masks.length, 6);
  for (const output of masks) {
    assert.equal(output.characterBoundaryClosed, true);
    assert.equal(output.projectileBoundaryClosed, true);
    const ys = output.characterBoundary!.map((p) => p[1]);
    assert.ok(Math.max(...ys) - Math.min(...ys) < 20, "Each island retains only its own boundary");
  }
  assert.equal(masks.filter((m) => m.view).length, 3);
  assert.equal(masks.filter((m) => m.obstacles.length).length, 3);
  assert.equal(compileAssetGameplay(document, assets, bounds).masks!.length, 6);
});

test("cropped open mask boundaries retain separate character and projectile applications", () => {
  const { document, asset, assets, bounds } = wallDisconnectedBoundaryFixture();
  const before = structuredClone(asset);
  const generated = wallSplineGameplay(document, assets, false);
  assert.deepEqual(generated.warnings, []);
  const masks = generated.descriptors[0]!.gameplay!.masks!;
  assert.equal(masks.length, 6);
  validateAssetGameplay(generated.descriptors[0]!.gameplay, generated.descriptors[0]!);
  for (let repeat = 0; repeat < 3; repeat++) {
    const first = masks[repeat * 2]!,
      second = masks[repeat * 2 + 1]!;
    assert.equal(first.view, true);
    assert.equal(second.view, false);
    assert.ok(first.obstacles.length);
    assert.deepEqual(second.obstacles, []);
    assert.deepEqual(first.triangles, second.triangles);
    for (const output of [first, second]) {
      assert.equal(output.characterBoundaryClosed, false);
      assert.equal(output.projectileBoundaryClosed, false);
      for (const boundary of [output.characterBoundary!, output.projectileBoundary!])
        assert.ok(boundary.every((p) => Math.abs(p[1] - boundary[0]![1]) < 1e-8));
    }
  }
  const compiled = compileAssetGameplay(document, assets, bounds);
  assert.equal(compiled.masks!.length, 6);
  assert.deepEqual(
    compiled.masks!.map((mask) => mask.mask_type),
    [23, 3, 23, 3, 23, 3],
  );
  assert.deepEqual(asset, before);
});

test("closed mask islands match independently authored contours after bending and moving", () => {
  for (const angle of [0, 37, 180]) {
    const { document, asset, assets, bounds } = wallClosedBoundaryFixture();
    asset.gameplay!.materials = [];
    asset.gameplay!.lights = [];
    asset.gameplay!.sounds = [];
    const radians = (angle * Math.PI) / 180;
    const path = document.splines![0]!;
    path.curved = true;
    path.points = [
      [-120, 0, 20],
      [0, 25, 30],
      [120, 0, 40],
    ].map(([x, y, z]) => [
      250 + x! * Math.cos(radians) - y! * Math.sin(radians),
      250 + x! * Math.sin(radians) + y! * Math.cos(radians),
      z!,
    ]);
    path.repeatLength = splineCurve(path, document.camera).getLength() / 3;
    const original = asset.gameplay!.masks![0]!;
    original.receiverSegment = [
      sceneToGame(document.camera, [-40, 0, -100]),
      sceneToGame(document.camera, [-40, 0, 100]),
    ];
    const split = compileAssetGameplay(document, assets, bounds).masks!;
    assert.ok(split.length >= 4);
    const second = structuredClone(original);
    second.id = "second-island";
    second.view = false;
    second.obstacles = [];
    for (const [mask, minY] of [
      [original, -20],
      [second, 10],
    ] as const) {
      mask.characterBoundary = [
        [-40, minY],
        [80, minY],
        [80, minY + 10],
        [-40, minY + 10],
      ].map(([x, y]) => sceneToGame(document.camera, [x!, y!, 0]));
      mask.projectileBoundary = structuredClone(mask.characterBoundary);
    }
    asset.gameplay!.masks!.push(second);
    const separate = compileAssetGameplay(document, assets, bounds).masks!;
    const ordered = (masks: typeof split) => masks.map((m) => JSON.stringify(m)).sort();
    assert.deepEqual(ordered(split), ordered(separate));
  }
});

test("split mask applications do not inherit another fragment's boundary or closure flag", () => {
  for (const splitRule of ["character", "projectile"] as const) {
    const { document, asset, assets, bounds } = wallDisconnectedBoundaryFixture();
    const mask = asset.gameplay!.masks![0]!;
    if (splitRule === "character") mask.projectileBoundary!.splice(2);
    else mask.characterBoundary!.splice(2);
    const generated = wallSplineGameplay(document, assets, false);
    assert.deepEqual(generated.warnings, []);
    for (const descriptor of generated.descriptors) {
      validateAssetGameplay(descriptor.gameplay, descriptor);
      for (const [index, output] of descriptor.gameplay!.masks!.entries()) {
        if (index % 2 === 0) continue;
        if (splitRule === "character") {
          assert.equal(output.projectileBoundary, undefined);
          assert.equal(output.projectileBoundaryClosed, undefined);
        } else {
          assert.equal(output.characterBoundary, undefined);
          assert.equal(output.characterBoundaryClosed, undefined);
        }
      }
    }
    assert.deepEqual(
      compileAssetGameplay(document, assets, bounds).masks!.map((m) => m.mask_type),
      [
        23,
        splitRule === "character" ? 1 : 2,
        23,
        splitRule === "character" ? 1 : 2,
        23,
        splitRule === "character" ? 1 : 2,
      ],
    );
  }
});

test("split mask boundaries match separately authored fragments on moved bent and rising walls", () => {
  for (const angle of [0, 37, 180]) {
    const { document, asset, assets, bounds } = wallDisconnectedBoundaryFixture();
    asset.gameplay!.materials = [];
    asset.gameplay!.lights = [];
    asset.gameplay!.sounds = [];
    asset.gameplay!.masks![0]!.receiverSegment = [
      sceneToGame(document.camera, [-40, 0, -100]),
      sceneToGame(document.camera, [-40, 0, 100]),
    ];
    const radians = (angle * Math.PI) / 180;
    const path = document.splines![0]!;
    path.curved = true;
    path.points = [
      [-120, 0, 20],
      [0, 25, 30],
      [120, 0, 40],
    ].map(([x, y, z]) => [
      250 + x! * Math.cos(radians) - y! * Math.sin(radians),
      250 + x! * Math.sin(radians) + y! * Math.cos(radians),
      z!,
    ]);
    path.repeatLength = splineCurve(path, document.camera).getLength() / 3;
    const generated = wallSplineGameplay(document, assets, false);
    assert.deepEqual(generated.warnings, []);
    const split = generated.descriptors[0]!.gameplay!.masks!;
    assert.ok(split.length >= 4);
    const original = asset.gameplay!.masks![0]!;
    const second = structuredClone(original);
    second.id = "second-fragment";
    second.view = false;
    second.obstacles = [];
    second.characterBoundary = original.characterBoundary!.slice(2);
    second.projectileBoundary = original.projectileBoundary!.slice(2);
    original.characterBoundary = original.characterBoundary!.slice(0, 2);
    original.projectileBoundary = original.projectileBoundary!.slice(0, 2);
    asset.gameplay!.masks!.push(second);
    const separate = wallSplineGameplay(document, assets, false);
    assert.deepEqual(separate.warnings, []);
    const ordered = (masks: typeof split) =>
      masks.map(({ id: _id, ...mask }) => JSON.stringify(mask)).sort();
    assert.deepEqual(ordered(split), ordered(separate.descriptors[0]!.gameplay!.masks!));
    // Validate transformed application lines through the normal mask compiler too.
    assert.ok(compileAssetGameplay(document, assets, bounds).masks!.length >= 4);
  }
});

test("spline volume headroom survives repeated deformation", () => {
  const { document, asset, assets } = wallSplineFixture();
  const shape = asset.gameplay!.volumes![0]!.shape;
  asset.gameplay!.volumes!.push({
    id: "overhead",
    node: "body",
    movementHeadroom: 80,
    shape: {
      points: shape.points,
      solid: shape.solid,
      opaque: shape.opaque,
      mouse: shape.mouse,
      show_shadow_polygon: shape.show_shadow_polygon,
      default_material: shape.default_material,
    },
  });
  const result = wallSplineGameplay(document, assets, false);
  const volumes = result.descriptors.flatMap((d) => d.gameplay!.volumes ?? []);
  assert.ok(volumes.filter((v) => v.movementHeadroom === 80).length > 1);
  assert.ok(volumes.some((v) => v.movementHeadroom === undefined));
});

test("spline clearances never silently lose their separate navigation plane", () => {
  const { document, asset, assets } = wallSplineFixture();
  asset.gameplay!.movementClearances = [
    {
      id: "raised-opening",
      node: "body",
      polygon: [
        [0, 0],
        [10, 0],
        [10, 10],
        [0, 10],
      ],
      height: 30,
      navigationHeight: 0,
    },
  ];
  assert.throws(
    () => wallSplineGameplay(document, assets, false),
    /navigation heights are unsupported/,
  );
  const result = wallSplineGameplay(document, assets, true);
  assert.ok(
    result.warnings.some((warning) => warning.includes("clearance omitted, collision retained")),
  );
  assert.equal(result.descriptors[0]!.gameplay!.movementClearances!.length, 0);
  assert.ok(result.descriptors[0]!.gameplay!.movementSolids!.length > 0);
});

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
  asset.gameplay!.lights[0]!.receivers = [sceneToGame(document.camera, [-49, 0, 40])];
  const anchored = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(anchored.light_sectors, raised.light_sectors);
  asset.gameplay!.lights[0]!.receivers = [sceneToGame(document.camera, [-49, 0, 41])];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /receiving segment/);
  asset.gameplay!.lights[0]!.receivers = [sceneToGame(document.camera, [0, 0, 40])];
  assert.ok(
    wallSplineGameplay(document, assets, true).warnings.some((warning) =>
      warning.includes("cropping removed every receiving anchor"),
    ),
  );
  delete asset.gameplay!.lights[0]!.receivers;
  asset.gameplay!.lights[0]!.receiverSegments = [
    [sceneToGame(document.camera, [-49, 0, 20]), sceneToGame(document.camera, [-49, 0, 60])],
  ];
  const segmented = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(segmented.light_sectors, raised.light_sectors);
  const path = document.splines![0]!;
  path.curved = true;
  path.points = [
    [100, 200, 20],
    [240, 240, 60],
    [400, 200, 80],
  ];
  path.repeatLength = splineCurve(path, document.camera).getLength();
  asset.gameplay!.lights[0]!.receiverSegments = [
    [sceneToGame(document.camera, [-49, 0, 20]), sceneToGame(document.camera, [49, 0, 60])],
  ];
  const bent = wallSplineGameplay(document, assets, false);
  assert.ok(
    bent.descriptors[0]!.gameplay!.lights!.every(
      (light) => light.receiverPolylines![0]!.length > 2,
    ),
  );
  const bentCompiled = compileAssetGameplay(document, assets, bounds);
  assert.ok(bentCompiled.light_sectors!.length);
  const ground = bentCompiled.sight_obstacles.find(
    (obstacle) => obstacle.projection_area && obstacle.points.every((point) => point.z_top === 0),
  );
  assert.ok(Array.isArray(ground?.projection_area));
  const groundLayer = ground.projection_area[1];
  assert.ok(bentCompiled.light_sectors!.every((light) => light.layer !== groundLayer));
  delete asset.gameplay!.lights[0]!.receiverSegments;
  assert.deepEqual(
    compileAssetGameplay(document, assets, bounds).light_sectors,
    bentCompiled.light_sectors,
  );
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

test("split lighting probes keep their receiving regions after spline trimming", () => {
  const { document, asset, assets, bounds } = wallDisconnectedLightFixture();
  const base = wallMaterialFixture();
  asset.gameplay!.masks = [];
  base.asset.gameplay!.masks = [];
  const before = structuredClone(asset);
  for (const points of [
    [
      [100, 200, 0],
      [400, 200, 0],
    ],
    [
      [100, 200, 0],
      [345, 200, 0],
    ],
    [
      [100, 200, 0],
      [220, 300, 0],
      [345, 200, 0],
    ],
  ]) {
    document.splines![0]!.points = points.map(([x, y, z]) => [x!, y!, z!]);
    document.splines![0]!.curved = points.length > 2;
    document.splines![0]!.repeatLength =
      points.length > 2 ? splineCurve(document.splines![0]!, document.camera).getLength() / 3 : 100;
    base.document.splines = structuredClone(document.splines);
    const generated = wallSplineGameplay(document, assets, false);
    assert.deepEqual(
      generated.warnings,
      wallSplineGameplay(base.document, base.assets, false).warnings,
    );
    const lights = generated.descriptors[0]!.gameplay!.lights!;
    assert.ok(lights.length > 0);
    assert.ok(lights.every((light) => light.receiverPolylines!.length === 2));
    assert.deepEqual(
      compileAssetGameplay(document, assets, bounds).light_sectors,
      compileAssetGameplay(base.document, base.assets, base.bounds).light_sectors,
    );
  }
  assert.deepEqual(asset, before);
});

test("wall masks deform coverage, front boundaries and obstacle ownership together", () => {
  const { document, asset, assets, bounds } = wallSplineFixture();
  const local = (x: number, y: number, z: number) => sceneToGame(document.camera, [x, y, z]);
  const a = local(-50, -10, 0),
    b = local(50, -10, 0),
    c = local(50, -10, 40),
    d = local(-50, -10, 40);
  const boundary = [local(-50, -10, 0), local(50, -10, 0), local(50, 10, 0), local(-50, 10, 0)];
  asset.gameplay!.masks = [
    {
      id: "front",
      node: "body",
      anchor: local(-49, 0, 0),
      view: true,
      triangles: [
        [a, b, c],
        [a, c, d],
      ],
      characterBoundary: boundary,
      projectileBoundary: boundary,
      obstacles: ["body-solid"],
    },
  ];
  document.splines![0]!.points = [
    [100, 200, 0],
    [345, 200, 0],
  ];
  const before = JSON.stringify(asset);
  const generated = wallSplineGameplay(document, assets, false);
  assert.deepEqual(generated.warnings, []);
  const masks = generated.descriptors[0]!.gameplay!.masks!;
  assert.equal(masks.length, 3);
  for (const mask of masks) {
    assert.ok(mask.obstacles.length >= 2);
    assert.ok(mask.triangles.flat().every(([x]) => x >= 100 && x <= 345));
    assert.ok(mask.characterBoundary!.every(([x]) => x >= 100 && x <= 345));
    assert.ok(mask.projectileBoundary!.every(([x]) => x >= 100 && x <= 345));
  }
  assert.deepEqual(
    masks.map((mask) => mask.obstacles.length),
    [2, 2, 3],
  );
  assert.equal(new Set(masks.flatMap((mask) => mask.obstacles)).size, 7);
  const result = compileAssetGameplay(document, assets, bounds);
  assert.equal(result.masks!.length, 3);
  assert.ok(result.masks!.every((mask) => mask.mask_type === 23));
  assert.equal(JSON.stringify(asset), before);
  const path = document.splines![0]!;
  path.curved = true;
  path.points = [
    [100, 200, 0],
    [220, 300, 0],
    [345, 200, 0],
  ];
  const bent = wallSplineGameplay(document, assets, false);
  assert.deepEqual(bent.warnings, []);
  assert.ok(
    bent.descriptors[0]!.gameplay!.masks!.some((mask) => mask.characterBoundary!.length > 4),
  );
  assert.ok(compileAssetGameplay(document, assets, bounds).masks!.length > 0);
  asset.gameplay!.masks![0]!.receiverSegment = [local(-49, 0, -10), local(49, 0, 10)];
  const cropped = compileAssetGameplay(document, assets, bounds, { bestEffort: true });
  assert.ok(
    cropped.warnings!.some((warning) => warning.includes("receiving layer is unavailable")),
  );
  assert.ok(cropped.masks!.length > 0);
  path.repeatLength = splineCurve(path, document.camera).getLength() / 3;
  const probed = wallSplineGameplay(document, assets, false);
  assert.deepEqual(probed.warnings, []);
  assert.ok(
    probed.descriptors[0]!.gameplay!.masks!.some((mask) => mask.receiverPolyline!.length > 2),
  );
  assert.ok(compileAssetGameplay(document, assets, bounds).masks!.length > 0);
});

test("spline mask splitting interpolates compact UVs and alpha with the geometry", () => {
  const { document, asset, assets, bounds } = wallMaterialFixture();
  document.splines![0]!.points = [
    [100, 200, 0],
    [345, 200, 0],
  ];
  const mask = asset.gameplay!.masks![0]!;
  mask.anchor = sceneToGame(document.camera, [-49, 0, 0]);
  delete mask.receiverSegment;
  mask.alphaCoverage = {
    textures: [{ width: 2, height: 1, alphaBase64: "/wA=" }],
    triangles: mask.triangles.map(() => ({
      uv: [
        [0, 0],
        [1, 0],
        [1, 1],
      ],
      alpha: [0, 1, 1],
      cutoff: 0.5,
      texture: 0,
      wrap: ["clamp", "clamp"],
      doubleSided: true,
    })),
  };
  const before = structuredClone(mask);
  const generated = wallSplineGameplay(document, assets, false);
  let interpolated = false;
  for (const descriptor of generated.descriptors)
    for (const output of descriptor.gameplay!.masks!) {
      assert.equal(output.alphaCoverage!.triangles.length, output.triangles.length);
      assert.deepEqual(output.alphaCoverage!.textures, mask.alphaCoverage.textures);
      for (const rule of output.alphaCoverage!.triangles) {
        assert.equal(rule.doubleSided, true);
        for (let i = 0; i < 3; i++) {
          assert.ok(Math.abs(rule.uv[i]![0] - rule.alpha[i]!) < 1e-10);
          if (rule.alpha[i]! > 0 && rule.alpha[i]! < 1) interpolated = true;
        }
      }
    }
  assert.ok(interpolated, "Clipped vertices must retain interpolated sampling coordinates");
  assert.ok(compileAssetGameplay(document, assets, bounds).masks!.length > 0);
  assert.deepEqual(mask, before);
});

test("a surviving mask probe retains a cropped repeat when its point anchor is trimmed", () => {
  const { document, asset, assets, bounds } = wallMaterialFixture();
  const mask = asset.gameplay!.masks![0]!;
  const local = (x: number, z: number) => sceneToGame(document.camera, [x, 0, z]);
  mask.anchor = local(49, 0);
  mask.receiverSegment = [local(-49, -10), local(-40, 10)];
  document.splines![0]!.points = [
    [100, 200, 0],
    [345, 200, 0],
  ];
  const generated = wallSplineGameplay(document, assets, false);
  assert.deepEqual(generated.warnings, []);
  assert.equal(generated.descriptors[0]!.gameplay!.masks!.length, 3);
  assert.equal(compileAssetGameplay(document, assets, bounds).masks!.length, 3);
  delete mask.receiverSegment;
  const pointOnly = wallSplineGameplay(document, assets, true);
  assert.equal(pointOnly.descriptors[0]!.gameplay!.masks!.length, 2);
  assert.ok(pointOnly.warnings.some((warning) => warning.includes("cropped receiving anchor")));
});

test("spline masks retain only ground contact points inside each cropped repeat", () => {
  const { document, asset, assets, bounds } = wallMaterialFixture();
  const mask = asset.gameplay!.masks![0]!;
  delete mask.receiverSegment;
  const local = (x: number) => sceneToGame(document.camera, [x, 0, 0]);
  mask.anchor = local(49);
  mask.receiverPoints = [local(-49), local(49)];
  document.splines![0]!.points = [
    [100, 200, 0],
    [345, 200, 0],
  ];
  const generated = wallSplineGameplay(document, assets, false);
  const masks = generated.descriptors[0]!.gameplay!.masks!;
  assert.deepEqual(
    masks.map((m) => m.receiverPoints!.length),
    [2, 2, 1],
  );
  assert.ok(masks.every((m) => m.receiverPoints!.every(([x]) => x >= 100 && x <= 345)));
  assert.ok(masks.every((m) => m.receiverSegment === undefined));
  assert.equal(compileAssetGameplay(document, assets, bounds).masks!.length, 3);
  mask.receiverPoints = [local(49)];
  const cropped = wallSplineGameplay(document, assets, false);
  assert.equal(cropped.descriptors[0]!.gameplay!.masks!.length, 2);
  assert.ok(cropped.warnings.some((warning) => warning.includes("cropped receiving points")));
});

test("spline masks keep disconnected probe fragments after clipping", () => {
  const base = wallMaterialFixture();
  const baseline = compileAssetGameplay(base.document, base.assets, base.bounds).masks;
  const { document, asset, assets, bounds } = wallDisconnectedMaskFixture();
  const mask = asset.gameplay!.masks![0]!;
  const before = structuredClone(mask);
  const generated = wallSplineGameplay(document, assets, false);
  assert.deepEqual(generated.warnings, []);
  for (const part of generated.descriptors) {
    validateAssetGameplay(part.gameplay, part);
    for (const output of part.gameplay!.masks!) {
      assert.equal(output.receiverPolyline, undefined);
      assert.equal(output.receiverSegment, undefined);
      assert.equal(output.receiverPolylines!.length, 2);
    }
  }
  assert.deepEqual(compileAssetGameplay(document, assets, bounds).masks, baseline);
  assert.deepEqual(mask, before);
  document.splines![0]!.curved = true;
  document.splines![0]!.points = [
    [100, 200, 0],
    [220, 300, 0],
    [345, 200, 0],
  ];
  const curved = wallSplineGameplay(document, assets, false);
  base.document.splines = structuredClone(document.splines);
  assert.deepEqual(curved.warnings, wallSplineGameplay(base.document, base.assets, false).warnings);
  assert.ok(
    curved.descriptors[0]!.gameplay!.masks!.some((part) => part.receiverPolylines?.length === 2),
  );
  assert.ok(compileAssetGameplay(document, assets, bounds).masks!.length > 0);
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
