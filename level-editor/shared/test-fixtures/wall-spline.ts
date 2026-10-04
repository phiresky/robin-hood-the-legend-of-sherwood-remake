import { Matrix4 } from "three";
import type { GameplayAssetDescriptor } from "../src/asset-gameplay.ts";
import type { Level3D } from "../src/level3d.ts";
import { createTerrainGrid } from "../src/authored-terrain.ts";
import { sceneToGame } from "../src/geometry.ts";

export function wallSplineFixture() {
  const camera = { kind: "oblique-orthographic" as const, elevation_deg: 35 };
  const corners = [
    [-50, -10],
    [50, -10],
    [50, 10],
    [-50, 10],
  ];
  const points = corners.map(([x, y]) => {
    const a = sceneToGame(camera, [x!, y!, 0]),
      b = sceneToGame(camera, [x!, y!, 40]);
    return { x: a[0], y: a[1], z_bottom: a[2], z_top: b[2] };
  });
  const asset: GameplayAssetDescriptor = {
    version: 1,
    kind: "projection-mapped-asset",
    id: "wall",
    name: "Wall",
    source_map: "Authored",
    model: "wall.glb",
    parts: [{ node: "body", name: "Wall", scenery: true }],
    gameplay: {
      version: 1,
      collision: "none",
      doors: [],
      surfaces: [],
      volumes: [
        {
          id: "body-solid",
          node: "body",
          shape: {
            points,
            solid: true,
            opaque: true,
            mouse: true,
            show_shadow_polygon: false,
            default_material: 3,
          },
        },
      ],
      spline: {
        bounds: { min: [-50, -10, 0], max: [50, 10, 40] },
        frames: { body: new Matrix4().toArray() },
      },
    },
  };
  const document: Level3D = {
    version: 1,
    map: "Spline courtyard",
    size: [500, 500],
    camera,
    objects: [],
    groups: [],
    sceneAssets: [],
    assetSources: [
      {
        id: asset.id,
        model: "wall/wall.glb",
        descriptor: "wall/asset.json",
        model_sha256: "a".repeat(64),
        descriptor_sha256: "b".repeat(64),
      },
    ],
    terrain: createTerrainGrid([0, 0, 500, 500], 100),
    splines: [
      {
        id: "wall-path",
        name: "Wall",
        kind: "wall",
        asset: "wall",
        axis: "x",
        sourceStraight: true,
        curved: false,
        points: [
          [100, 200, 0],
          [400, 200, 0],
        ],
        closed: false,
        width: 20,
        repeatLength: 100,
      },
    ],
  };
  return {
    asset,
    document,
    assets: new Map([[asset.id, asset]]),
    bounds: [0, 0, 500, 500] as [number, number, number, number],
  };
}

export function wallMaterialFixture() {
  const fixture = wallSplineFixture();
  const { asset, document } = fixture;
  const local = (x: number, y: number, z: number) => sceneToGame(document.camera, [x, y, z]);
  asset.gameplay!.materials = [
    {
      id: "front",
      node: "body",
      material: 4,
      ground: false,
      obstacles: ["body-solid"],
      polygon: [local(-50, -10, 0), local(50, -10, 0), local(50, -10, 40), local(-50, -10, 40)],
    },
    {
      id: "ground",
      node: "body",
      material: 1,
      ground: true,
      obstacles: [],
      polygon: [local(-40, -40, 0), local(0, -40, 0), local(0, -20, 0), local(-40, -20, 0)],
    },
  ];
  asset.gameplay!.lights = [
    {
      id: "shadow",
      node: "body",
      ambiences: 5,
      polygon: asset.gameplay!.materials[1]!.polygon.map((point) => [...point]),
    },
  ];
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
        polyline: [local(0, 0, 0)],
        innerDistance: 10,
        outerDistance: 60,
        innerVolume: 80,
        outerVolume: 0,
        noiseCoveringDistance: 15,
      },
    },
  ];
  return fixture;
}
