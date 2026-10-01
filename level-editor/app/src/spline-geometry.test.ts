import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import { parseLevel3D, createTerrainGrid, type LevelSpline, type Level3D } from "@rle/shared";
import { riverGeometry, splineCurve, wallMesh, blendedSplineTexture } from "./spline-geometry.ts";
import { SplineLayer } from "./spline-layer.ts";

const camera = { kind: "oblique-orthographic" as const, elevation_deg: 35 };
const river: LevelSpline = {
  id: "river",
  name: "River",
  kind: "river",
  points: [
    [0, 0, 0],
    [300, 0, 0],
    [600, 150, 0],
  ],
  width: 80,
  repeatLength: 100,
  closed: false,
};
test("roads drape both edges and interior across a sloped terrain grid", () => {
  const terrain = createTerrainGrid([-200, -200, 1000, 1000], 500);
  for (const vertex of terrain.vertices)
    vertex.position[2] = vertex.position[0] / 10 + vertex.position[1] / 5;
  const document = { terrain } as Level3D;
  const geometry = riverGeometry(
    { ...river, kind: "road", pointWidths: [20, 100, 40] },
    camera,
    document,
  );
  const positions = geometry.getAttribute("position"),
    sine = Math.sin((camera.elevation_deg * Math.PI) / 180),
    cosine = Math.cos((camera.elevation_deg * Math.PI) / 180);
  for (let i = 0; i < positions.count; i++) {
    const x = positions.getX(i),
      y = -positions.getY(i) * sine;
    assert.ok(Math.abs(positions.getZ(i) - 0.8 - (x / 10 + y / 5) / cosine) < 0.00002);
  }
  geometry.dispose();
});
test("point materials produce a synchronous texture spanning the complete path", () => {
  const path = {
    ...river,
    points: [
      [0, 0, 0],
      [100, 0, 0],
    ] as [number, number, number][],
    pointMaterials: ["water_still", "water_white"],
  };
  const texture = blendedSplineTexture(path, camera);
  assert.equal(texture.image.width, 128);
  assert.equal(texture.image.height, 50);
  assert.equal(texture.repeat.y, path.repeatLength / 100);
  assert.notDeepEqual(
    Array.from((texture.image.data as Uint8Array).slice(64 * 4, 64 * 4 + 4)),
    Array.from(
      (texture.image.data as Uint8Array).slice((49 * 128 + 64) * 4, (49 * 128 + 64) * 4 + 4),
    ),
  );
  texture.dispose();
});
test("river ribbons keep world width and repeat texture by arc length through curves", () => {
  const geometry = riverGeometry(river, camera),
    positions = geometry.getAttribute("position"),
    uv = geometry.getAttribute("uv");
  for (let i = 0; i < positions.count; i += 2) {
    assert.ok(
      Math.abs(
        new THREE.Vector3()
          .fromBufferAttribute(positions, i)
          .distanceTo(new THREE.Vector3().fromBufferAttribute(positions, i + 1)) - 80,
      ) < 0.001,
    );
  }
  assert.ok(Math.abs(uv.getY(uv.count - 1) - splineCurve(river, camera).getLength() / 100) < 1e-5);
  geometry.dispose();
});
test("wall repeats bend real mesh vertices, retain UVs and leave shared geometry untouched", () => {
  const source = new THREE.Mesh(new THREE.BoxGeometry(100, 12, 40), new THREE.MeshBasicMaterial());
  const original = Array.from(source.geometry.getAttribute("position").array);
  const path: LevelSpline = {
    ...river,
    kind: "wall",
    asset: "wall",
    axis: "x",
    width: 12,
    repeatLength: 100,
  };
  const wall = wallMesh(path, camera, new Map([["asset:wall:building-000", source]]));
  assert.ok(wall.children.length >= 6);
  const last = wall.children.at(-1) as THREE.Mesh;
  const end = new THREE.Box3().setFromObject(last);
  assert.ok(end.max.x > 590 && end.min.y < -200, "last section must follow the curved endpoint");
  assert.ok(last.geometry.getAttribute("uv").count > 0);
  assert.deepEqual(Array.from(source.geometry.getAttribute("position").array), original);
  wall.traverse((node) => {
    if (node instanceof THREE.Mesh) node.geometry.dispose();
  });
  source.geometry.dispose();
  (source.material as THREE.Material).dispose();
});
test("a skewed source wall keeps its requested thickness instead of its bounding-box ratio", () => {
  const source = new THREE.Mesh(new THREE.BoxGeometry(100, 12, 40), new THREE.MeshBasicMaterial());
  const positions = source.geometry.getAttribute("position");
  for (let i = 0; i < positions.count; i++)
    positions.setY(i, positions.getY(i) + positions.getX(i) * 0.6);
  const path: LevelSpline = {
    ...river,
    kind: "wall",
    asset: "wall",
    axis: "x",
    width: 30,
    repeatLength: 100,
    points: [
      [0, 0, 0],
      [200, 0, 0],
    ],
  };
  const wall = wallMesh(path, camera, new Map([["asset:wall:building-000", source]]));
  for (const child of wall.children) {
    const bounds = new THREE.Box3().setFromObject(child);
    assert.ok(Math.abs(bounds.max.y - bounds.min.y - 30) < 0.001);
  }
  wall.traverse((node) => {
    if (node instanceof THREE.Mesh) node.geometry.dispose();
  });
  source.geometry.dispose();
  source.material.dispose();
});
test("flipping the wall puts its parapet on the opposite side and preserves outward face winding", () => {
  const source = new THREE.Group();
  const base = new THREE.Mesh(new THREE.BoxGeometry(100, 20, 30), new THREE.MeshBasicMaterial());
  const parapet = new THREE.Mesh(new THREE.BoxGeometry(100, 4, 12), new THREE.MeshBasicMaterial());
  parapet.position.set(0, 8, 21);
  source.add(base, parapet);
  const path: LevelSpline = {
    ...river,
    kind: "wall",
    asset: "wall",
    axis: "x",
    width: 20,
    repeatLength: 100,
    points: [
      [0, 0, 0],
      [100, 0, 0],
    ],
  };
  const sources = new Map([["asset:wall:building-000", source]]);
  const normal = wallMesh(path, camera, sources),
    flipped = wallMesh({ ...path, flipCrossSection: true }, camera, sources);
  const before = new THREE.Box3().setFromObject(normal.children[1]);
  const after = new THREE.Box3().setFromObject(flipped.children[1]);
  assert.ok(before.min.y > 0 && after.max.y < 0, "parapet must swap sides");
  for (const group of [normal, flipped])
    group.traverse((node) => {
      if (!(node instanceof THREE.Mesh)) return;
      const p = node.geometry.getAttribute("position"),
        n = node.geometry.getAttribute("normal");
      let top = false;
      for (let i = 0; i < p.count; i++) if (n.getZ(i) > 0.99) top = true;
      assert.ok(top, "top-facing triangles must retain positive normals after reflection");
      node.geometry.dispose();
    });
  base.geometry.dispose();
  parapet.geometry.dispose();
  base.material.dispose();
  parapet.material.dispose();
});
test("spline geometry retirement does not dispose borrowed wall materials or source meshes", () => {
  const source = new THREE.Mesh(new THREE.BoxGeometry(100, 12, 40), new THREE.MeshBasicMaterial());
  let sourceDisposals = 0;
  source.geometry.addEventListener("dispose", () => sourceDisposals++);
  source.material.addEventListener("dispose", () => sourceDisposals++);
  const layer = new SplineLayer();
  const path: LevelSpline = {
    ...river,
    kind: "wall",
    asset: "wall",
    axis: "x",
    width: 12,
    repeatLength: 100,
  };
  layer.sync([path], camera, new Map([["asset:wall:building-000", source]]));
  layer.clear();
  assert.equal(sourceDisposals, 0);
  source.geometry.dispose();
  source.material.dispose();
});
test("documents round-trip splines and reject dangling sources and invalid control geometry", () => {
  const document: Level3D = {
    version: 1,
    map: "Test",
    sceneAssets: [],
    size: [1000, 1000],
    camera,
    objects: [],
    groups: [],
    splines: [river],
  };
  assert.deepEqual(parseLevel3D(JSON.parse(JSON.stringify(document))).splines, [river]);
  assert.throws(() => parseLevel3D({ ...document, splines: [{ ...river, width: 0 }] }), /width/);
  assert.throws(
    () => parseLevel3D({ ...document, splines: [{ ...river, points: [[0, 0, 0]] }] }),
    /control points/,
  );
  assert.throws(
    () =>
      parseLevel3D({
        ...document,
        splines: [{ ...river, kind: "wall", asset: "missing", axis: "x" }],
      }),
    /wall asset/,
  );
});

test("corner towers join turns, skip straight controls and retain shared resources", () => {
  const curtain = new THREE.Mesh(new THREE.BoxGeometry(100, 12, 40), new THREE.MeshBasicMaterial());
  const tower = new THREE.Mesh(
    new THREE.CylinderGeometry(24, 24, 50, 16).rotateX(Math.PI / 2),
    new THREE.MeshBasicMaterial(),
  );
  const sources = new Map([
    ["asset:wall:building-000", curtain],
    ["asset:tower:building-001", tower],
  ]);
  const path: LevelSpline = {
    ...river,
    kind: "wall",
    asset: "wall",
    axis: "x",
    cornerAsset: "tower",
    cornerMinAngle: 40,
    cornerWidthScale: 2,
    width: 12,
    repeatLength: 100,
    points: [
      [0, 0, 0],
      [100, 0, 0],
      [200, 0, 0],
      [200, 150, 20],
    ],
  };
  const layer = new SplineLayer();
  let disposed = 0;
  tower.geometry.addEventListener("dispose", () => disposed++);
  tower.material.addEventListener("dispose", () => disposed++);
  layer.sync([path], camera, sources);
  const joined = layer.root.children[1];
  const corners = joined.children.filter((c) => c.userData.cornerPoint !== undefined);
  assert.equal(corners.length, 1);
  assert.equal(corners[0].userData.cornerPoint, 2);
  assert.ok(corners[0].position.distanceTo(new THREE.Vector3(200, 0, 0)) < 0.001);
  const fitted = new THREE.Box3().setFromObject(corners[0], true).getSize(new THREE.Vector3());
  assert.ok(
    fitted.x > 94 && fitted.x < 98 && Math.abs(fitted.z - 50) < 0.001,
    "tower width must change independently of height",
  );
  layer.sync([{ ...path, cornerDisabled: [2] }], camera, sources);
  assert.ok(!layer.root.children[1].children.some((c) => c.userData.cornerPoint !== undefined));
  layer.clear();
  assert.equal(disposed, 0);
  curtain.geometry.dispose();
  curtain.material.dispose();
  tower.geometry.dispose();
  tower.material.dispose();
});
test("closed walls place seam towers once, including a single remaining corner", () => {
  const source = new THREE.Mesh(new THREE.BoxGeometry(100, 12, 40), new THREE.MeshBasicMaterial());
  const sources = new Map([
    ["asset:wall:building-000", source],
    ["asset:tower:building-001", source],
  ]);
  const path: LevelSpline = {
    ...river,
    kind: "wall",
    asset: "wall",
    axis: "x",
    cornerAsset: "tower",
    closed: true,
    points: [
      [0, 0, 0],
      [200, 0, 0],
      [200, 200, 0],
      [0, 200, 0],
    ],
  };
  for (const disabled of [[], [1, 2, 3]]) {
    const result = wallMesh({ ...path, cornerDisabled: disabled }, camera, sources);
    assert.equal(
      result.children.filter((c) => c.userData.cornerPoint !== undefined).length,
      4 - disabled.length,
    );
    result.traverse((n) => {
      if (n instanceof THREE.Mesh) n.geometry.dispose();
    });
  }
  source.geometry.dispose();
  source.material.dispose();
});
test("footpaths save independently of water and follow authored height", () => {
  const road: LevelSpline = {
    ...river,
    kind: "road",
    width: 24,
    points: [
      [0, 0, 5],
      [80, 20, 30],
      [160, 40, 40],
    ],
  };
  const doc: Level3D = {
    version: 1,
    map: "Roads",
    sceneAssets: [],
    size: [300, 300],
    camera,
    objects: [],
    groups: [],
    splines: [road],
  };
  assert.deepEqual(parseLevel3D(JSON.parse(JSON.stringify(doc))).splines, [road]);
  const geometry = riverGeometry(road, camera);
  assert.ok(geometry.getAttribute("position").getZ(0) > 5);
  const layer = new SplineLayer();
  layer.sync([road], camera, new Map());
  assert.equal(layer.root.children[1].userData.noSunShadow, true);
  layer.clear();
  geometry.dispose();
});

test("prepared fence strips preserve narrow rails between thick posts", () => {
  const source = new THREE.Group();
  const rail = new THREE.Mesh(new THREE.BoxGeometry(100, 2, 4), new THREE.MeshBasicMaterial());
  rail.position.z = 10;
  const post = new THREE.Mesh(new THREE.BoxGeometry(8, 10, 20), new THREE.MeshBasicMaterial());
  post.position.set(-40, 0, 10);
  source.add(rail, post);
  const path: LevelSpline = {
    ...river,
    kind: "wall",
    asset: "fence",
    axis: "x",
    sourceStraight: true,
    width: 10,
    repeatLength: 100,
    points: [
      [0, 0, 0],
      [100, 0, 0],
    ],
  };
  const original = Array.from(rail.geometry.getAttribute("position").array);
  const result = wallMesh(path, camera, new Map([["asset:fence:rails", source]]));
  const bounds = new THREE.Box3().setFromObject(result.children[0]!);
  assert.ok(
    Math.abs(bounds.max.y - bounds.min.y - 2) < 1e-5,
    "Rails must keep their two-unit thickness, not expand to the ten-unit posts",
  );
  assert.deepEqual(Array.from(rail.geometry.getAttribute("position").array), original);
  result.traverse((node) => {
    if (node instanceof THREE.Mesh) node.geometry.dispose();
  });
  rail.geometry.dispose();
  post.geometry.dispose();
});

test("Paths browsing exposes every spline without enabling other paths' point handles", () => {
  const other: LevelSpline = {
    ...river,
    id: "other",
    points: river.points.map(([x, y, z]) => [x, y + 400, z]),
  };
  const layer = new SplineLayer();
  layer.sync([river, other], camera, new Map());
  assert.equal(layer.controls.children.length, 0);
  layer.setBrowse(true);
  assert.deepEqual(
    new Set(
      layer.controls.children.flatMap((child) =>
        child.userData.splinePath ? [child.userData.splinePath] : [],
      ),
    ),
    new Set(["river", "other"]),
  );
  const handles = layer.controls.children.filter(
    (child) => child instanceof THREE.Mesh && child.geometry instanceof THREE.SphereGeometry,
  ) as THREE.Mesh<THREE.SphereGeometry>[];
  assert.equal(handles.length, river.points.length + other.points.length);
  const inactive = handles.find((handle) => handle.userData.splinePath === "other")!;
  const handleRay = new THREE.Raycaster(
    inactive.getWorldPosition(new THREE.Vector3()).add(new THREE.Vector3(10, 0, 100)),
    new THREE.Vector3(0, 0, -1),
  );
  assert.equal(layer.hitPath(handleRay), "other", "Visible points select their own path");
  assert.equal(layer.hitHandle(handleRay), null, "Inactive points must not drag the active path");
  const wide = layer.controls.children.filter(
    (child) => "isLine2" in child,
  ) as import("three/addons/lines/Line2.js").Line2[];
  assert.equal(wide.length, 4);
  for (const line of wide) {
    assert.equal(line.material.linewidth, 2.5);
    assert.equal(line.material.transparent, true);
    assert.equal(line.material.depthTest, false);
  }
  const line = layer.controls.children.find(
    (child) => child.userData.splinePath === "other",
  ) as THREE.Line;
  const point = new THREE.Vector3().fromBufferAttribute(line.geometry.getAttribute("position"), 12);
  const ray = new THREE.Raycaster(
    point.clone().add(new THREE.Vector3(0, 0, 100)),
    new THREE.Vector3(0, 0, -1),
  );
  assert.equal(layer.hitPath(ray), "other");
  assert.equal(layer.hitHandle(ray), null);
  layer.setMode({
    path: river,
    drawing: false,
    point: 0,
    append() {},
    move() {},
    selectPoint() {},
  });
  assert.equal(
    layer.controls.children.filter((child) => typeof child.userData.splinePoint === "number")
      .length,
    river.points.length,
  );
  assert.equal(
    layer.controls.children.filter(
      (child) => child instanceof THREE.Mesh && child.geometry instanceof THREE.SphereGeometry,
    ).length,
    river.points.length,
    "Selecting a path hides the other paths' points",
  );
  assert.equal(
    layer.hitPath(ray),
    "other",
    "Other paths remain selectable while editing a saved path",
  );
  layer.setMode(null);
  assert.equal(layer.hitPath(ray), "other", "Done editing returns to browse overlays");
  assert.equal(
    layer.controls.children.filter(
      (child) => child instanceof THREE.Mesh && child.geometry instanceof THREE.SphereGeometry,
    ).length,
    river.points.length + other.points.length,
    "Clearing selection restores every path's points",
  );
  layer.setBrowse(false);
  assert.equal(layer.controls.children.length, 0, "Leaving Paths removes overlays");
  layer.clear();
});

test("live path blends use bounded textures while preserving endpoint materials and world repeat", () => {
  const path: LevelSpline = {
    ...river,
    points: [
      [0, 0, 0],
      [4000, 0, 0],
    ],
    pointMaterials: ["water_still", "water_white"],
  };
  const committed = blendedSplineTexture(path, camera);
  const preview = blendedSplineTexture(path, camera, undefined, true);
  assert.equal(committed.image.width, 128);
  assert.equal(committed.image.height, 2000);
  assert.equal(preview.image.width, 64);
  assert.equal(preview.image.height, 512);
  assert.equal(preview.repeat.y, committed.repeat.y);
  for (const row of [0, 1]) {
    const pixel = (t: THREE.DataTexture) => {
      const offset = ((t.image.height - 1) * row * t.image.width + t.image.width / 2) * 4;
      return Array.from(t.image.data.slice(offset, offset + 4));
    };
    const endpointMaterial = path.pointMaterials![row]!;
    for (const [texture, isPreview] of [
      [preview, true],
      [committed, false],
    ] as const) {
      const reference = blendedSplineTexture(
        { ...path, pointMaterials: [endpointMaterial, endpointMaterial] },
        camera,
        undefined,
        isPreview,
      );
      assert.deepEqual(
        pixel(texture),
        pixel(reference),
        "Endpoint sampling must use only its own material",
      );
      reference.dispose();
    }
  }
  committed.dispose();
  preview.dispose();
});

test("river preview sync builds one surface and restores committed quality even for the same path", () => {
  const path: LevelSpline = {
    ...river,
    pointMaterials: ["water_still", "water_white", "water_still"],
  };
  const document = { camera, splines: [path] } as Level3D;
  const layer = new SplineLayer();
  const sources = new Map<string, THREE.Object3D>();
  const surface = () =>
    layer.root.children.find((child) => child instanceof THREE.Mesh) as THREE.Mesh<
      THREE.BufferGeometry,
      THREE.MeshBasicMaterial
    >;
  layer.sync([path], camera, sources, document, true);
  const initial = surface();
  assert.equal((initial.material.map as THREE.DataTexture).image.width, 64);
  layer.showPreview(path);
  assert.equal(surface(), initial);
  assert.equal(layer.root.children.filter((child) => child instanceof THREE.Mesh).length, 1);
  layer.sync([path], camera, sources, document);
  assert.notEqual(surface(), initial);
  assert.equal((surface().material.map as THREE.DataTexture).image.width, 128);
  layer.clear();
});

test("terrain drags retain river surfaces and road textures through commit", () => {
  const straight: LevelSpline = {
    ...river,
    points: [
      [0, 0, 0],
      [100, 0, 0],
    ],
    width: 20,
  };
  const paths = [
    { ...straight, pointMaterials: ["water_still", "water_still"] },
    {
      ...straight,
      points: [
        [0, 50, 0],
        [100, 50, 0],
      ] as [number, number, number][],
      id: "road",
      kind: "road" as const,
      pointMaterials: ["path_dirt", "path_dirt"],
    },
  ];
  const terrain = createTerrainGrid([-50, -50, 200, 150], 50);
  const document = { camera, terrain, splines: paths } as Level3D;
  const layer = new SplineLayer();
  const sources = new Map<string, THREE.Object3D>();
  layer.sync(paths, camera, sources, document);
  const meshes = layer.root.children.filter((child) => child instanceof THREE.Mesh) as THREE.Mesh<
    THREE.BufferGeometry,
    THREE.MeshBasicMaterial
  >[];
  const [water, road] = meshes;
  const waterGeometry = water!.geometry,
    roadGeometry = road!.geometry;
  const textures = meshes.map((mesh) => mesh.material.map);
  let disposed = 0;
  for (const texture of textures) texture!.addEventListener("dispose", () => disposed++);
  const changed = {
    ...document,
    terrain: {
      ...terrain,
      vertices: terrain.vertices.map((v) => ({
        ...v,
        position: [v.position[0], v.position[1], 30] as [number, number, number],
      })),
    },
  };
  layer.sync(paths, camera, sources, changed, true);
  assert.equal(water!.parent, layer.root);
  assert.equal(water!.geometry, waterGeometry);
  assert.notEqual(road!.geometry, roadGeometry);
  const previewGeometry = road!.geometry;
  layer.sync(paths, camera, sources, changed, false);
  assert.notEqual(
    road!.geometry,
    previewGeometry,
    "Release must restore exact geometry even when the document is unchanged",
  );
  const exact = riverGeometry(paths[1]!, camera, changed);
  assert.deepEqual(
    road!.geometry.getAttribute("position").array,
    exact.getAttribute("position").array,
  );
  exact.dispose();
  assert.deepEqual(
    meshes.map((mesh) => mesh.material.map),
    textures,
  );
  assert.equal(disposed, 0);
  layer.clear();
  assert.equal(disposed, textures.length);
});

test("straight wall controls form independent straight sections and round-trip", () => {
  const source = new THREE.Mesh(new THREE.BoxGeometry(100, 12, 40), new THREE.MeshBasicMaterial());
  const path: LevelSpline = {
    ...river,
    kind: "wall",
    asset: "wall",
    curved: false,
    sourceStraight: true,
    width: 12,
    points: [
      [0, 0, 0],
      [200, 0, 0],
      [200, 200, 0],
    ],
  };
  const document: Level3D = {
    version: 1,
    map: "test",
    sceneAssets: [],
    size: [500, 500],
    camera,
    objects: [],
    groups: [],
    splines: [path],
  };
  const stored = { ...document, splines: [{ ...path, kind: "road" }] };
  assert.equal(parseLevel3D(JSON.parse(JSON.stringify(stored))).splines![0]!.curved, false);
  assert.throws(
    () => parseLevel3D({ ...document, splines: [{ ...path, curved: "yes" }] }),
    /curved/,
  );
  for (const closed of [false, true]) {
    const wall = wallMesh({ ...path, closed }, camera, new Map([["asset:wall:mesh", source]]));
    assert.equal(wall.children.length, closed ? 3 : 2);
    const first = new THREE.Box3().setFromObject(wall.children[0]!);
    const second = new THREE.Box3().setFromObject(wall.children[1]!);
    assert.ok(Math.abs(first.min.y + 6) < 1e-5 && Math.abs(first.max.y - 6) < 1e-5);
    assert.ok(Math.abs(second.min.x - 194) < 1e-5 && Math.abs(second.max.x - 206) < 1e-5);
    wall.traverse((node) => {
      if (node instanceof THREE.Mesh) node.geometry.dispose();
    });
  }
  source.geometry.dispose();
  source.material.dispose();
});

function projectedArea(geometry: THREE.BufferGeometry) {
  const p = geometry.getAttribute("position"),
    index = geometry.index;
  let area = 0;
  for (let i = 0; i < (index?.count ?? p.count); i += 3) {
    const a = index?.getX(i) ?? i,
      b = index?.getX(i + 1) ?? i + 1,
      c = index?.getX(i + 2) ?? i + 2;
    area +=
      Math.abs(
        (p.getX(b) - p.getX(a)) * (p.getY(c) - p.getY(a)) -
          (p.getY(b) - p.getY(a)) * (p.getX(c) - p.getX(a)),
      ) / 2;
  }
  return area;
}

test("road clipping preserves coverage across coincident river edges and beyond the terrain", () => {
  const road: LevelSpline = { ...river, kind: "road" };
  const ribbon = riverGeometry(road, camera);
  for (const bounds of [
    [-200, -200, 1000, 1000],
    [0, 0, 250, 250],
  ] as [number, number, number, number][]) {
    const document = {
      camera,
      terrain: createTerrainGrid(bounds, 250),
      splines: [river],
    } as Level3D;
    const draped = riverGeometry(road, camera, document);
    assert.ok(Math.abs(projectedArea(draped) / projectedArea(ribbon) - 1) < 1e-5);
    assert.ok(
      draped.getAttribute("position").count < 150000,
      "degenerate fragments must not proliferate",
    );
    draped.dispose();
  }
  ribbon.dispose();
});

test("editing one road preserves other roads and their evaluated terrain", () => {
  const road: LevelSpline = { ...river, id: "road", kind: "road" };
  const other = { ...road, id: "other" };
  const document = {
    camera,
    terrain: createTerrainGrid([-200, -200, 1000, 1000], 250),
    splines: [river, road, other],
  } as Level3D;
  const layer = new SplineLayer(),
    sources = new Map<string, THREE.Object3D>();
  layer.sync(document.splines!, camera, sources, document);
  const mesh = layer.root.children
    .filter((child) => child instanceof THREE.Mesh)
    .at(-1) as THREE.Mesh;
  const geometry = mesh.geometry;
  const next = { ...document, splines: [river, { ...road, width: 100 }, other] };
  layer.sync(next.splines, camera, sources, next);
  assert.equal(mesh.geometry, geometry);
  layer.clear();
});
