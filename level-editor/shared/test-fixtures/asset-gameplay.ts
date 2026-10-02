import type { GameplayAssetDescriptor } from "../src/asset-gameplay.ts";
import { IDENTITY_TRANSFORM, type Level3D, type Level3DObject } from "../src/level3d.ts";
import type { MaskTriangle } from "../src/compile-mask-geometry.ts";

export function preservedBoundaryCompilerFixture() {
  const fixture = assetCompilerFixture();
  const g = fixture.hut.gameplay!;
  g.collision = "none";
  g.doors = [];
  g.surfaces = [
    {
      id: "ground",
      node: "building-999",
      height: 0,
      polygon: [
        [0, 0],
        [100, 0],
        [100, 70],
      ],
      navigationRegion: "ground",
      preserveMovementBoundary: true,
    },
  ];
  g.movementBlockers = [
    {
      id: "crossing-wall",
      node: "building-999",
      height: 0,
      polygon: [
        [0, -10],
        [110, -10],
        [110, 76],
        [0, -1],
      ],
    },
  ];
  return fixture;
}

export function preservedContoursCompilerFixture() {
  const fixture = preservedBoundaryCompilerFixture();
  const surface = fixture.hut.gameplay!.surfaces[0]!;
  surface.polygon = [
    [0, 0],
    [100, 0],
    [100, 100],
    [0, 100],
  ];
  surface.holes = [
    [
      [0, 0],
      [100, 0],
      [0, 71],
    ],
  ];
  surface.holeContours = ["assembly/slope"];
  const wall = fixture.hut.gameplay!.movementBlockers![0]!;
  wall.polygon = [
    [40, -10],
    [60, -10],
    [60, 100],
    [40, 100],
  ];
  wall.movementContour = "assembly/wall";
  return fixture;
}

export function maskAssetCompilerFixture() {
  const fixture = assetCompilerFixture();
  const g = fixture.hut.gameplay!;
  const vertices: [number, number, number][] = [
    [40, 40, 0],
    [50, 40, 0],
    [50, 50, 0],
    [40, 50, 0],
    [40, 40, 30],
    [50, 40, 30],
    [50, 50, 30],
    [40, 50, 30],
  ];
  const triangles = [
    [0, 1, 2, 3],
    [4, 5, 6, 7],
    [0, 1, 5, 4],
    [1, 2, 6, 5],
    [2, 3, 7, 6],
    [3, 0, 4, 7],
  ].flatMap(([a, b, c, d]): MaskTriangle[] => [
    [vertices[a!]!, vertices[b!]!, vertices[c!]!],
    [vertices[a!]!, vertices[c!]!, vertices[d!]!],
  ]);
  const initial = {
    id: "covered",
    node: "building-999",
    triangles,
    anchor: [45, 45, 0] as [number, number, number],
    view: true,
    characterBoundary: vertices.slice(0, 4),
    obstacles: ["building-999"],
  };
  g.masks = [
    initial,
    { ...structuredClone(initial), id: "revealed", triangles: triangles.slice(0, 2) },
  ];
  g.movementTransitions = [
    {
      id: "cover-state",
      node: "building-999",
      waypoint: [45, 45, 0],
      active: true,
      definitive: false,
      initial: [],
      applied: [],
      initialMasks: ["covered"],
      appliedMasks: ["revealed"],
      applyPolygon: [],
      noApplyPolygon: [],
    },
  ];
  fixture.document.map = "Asset mask fixture";
  return fixture;
}

export function navigationRegionCompilerFixture() {
  const fixture = assetCompilerFixture();
  const [west, east] = fixture.hut.gameplay!.surfaces;
  if (!west || !east) throw new Error("Navigation fixture needs two rooms");
  west.polygon = [
    [0, 0],
    [100, 0],
    [100, 100],
    [0, 100],
  ];
  east.polygon = [
    [100, 0],
    [200, 0],
    [200, 100],
    [100, 100],
  ];
  west.navigationRegion = "west";
  east.navigationRegion = "east";
  return fixture;
}

export function nonrenderingVolumeCompilerFixture() {
  const fixture = assetCompilerFixture();
  const obstacle = fixture.hut.parts[0]?.obstacle_local_game;
  if (!obstacle) throw new Error("Volume fixture needs a collision shape");
  const { projection_area: _projection, material_indices: _materials, ...shape } = obstacle;
  fixture.hut.gameplay!.collision = "none";
  fixture.hut.gameplay!.volumes = [{ id: "invisible-wall", node: "building-999", shape }];
  fixture.document.map = "Non-rendering volume fixture";
  return fixture;
}

export function jumpAssetCompilerFixture() {
  const fixture = assetCompilerFixture();
  const g = fixture.hut.gameplay!;
  g.doors = [];
  const east = g.surfaces.find((surface) => surface.id === "east");
  if (!east) throw new Error("Jump fixture needs its eastern surface");
  east.height = 100;
  g.jumpZones = [
    {
      id: "low-zone",
      node: "building-999",
      anchor: [80, 50, 0],
      helperNeeded: true,
      polygon: [
        [60, 20, 0],
        [90, 20, 0],
        [90, 80, 0],
        [60, 80, 0],
      ],
    },
    {
      id: "high-zone",
      node: "building-999",
      anchor: [120, 50, 100],
      helperNeeded: false,
      polygon: [
        [110, 20, 100],
        [140, 20, 100],
        [140, 80, 100],
        [110, 80, 100],
      ],
    },
  ];
  g.jumpPairs = [
    {
      id: "wall-jump",
      node: "building-999",
      long: true,
      edges: [
        { zone: "low-zone", a: [85, 30, 0], b: [85, 70, 0] },
        { zone: "high-zone", a: [115, 70, 100], b: [115, 30, 100] },
      ],
    },
  ];
  return fixture;
}

export function lightAssetCompilerFixture() {
  const fixture = interiorAssetCompilerFixture();
  fixture.hut.gameplay!.lights = [
    {
      id: "day-shadow",
      node: "building-999",
      ambiences: 1,
      polygon: [
        [10, 10, 0],
        [30, 10, 0],
        [30, 30, 0],
        [10, 30, 0],
      ],
    },
    {
      id: "night-light",
      node: "building-999",
      ambiences: 2,
      polygon: [
        [60, 10, 0],
        [80, 10, 0],
        [80, 30, 0],
        [60, 30, 0],
      ],
    },
  ];
  return fixture;
}

export function detachedJumpCompilerFixture() {
  const fixture = crossAssetJumpCompilerFixture();
  const { document } = fixture;
  for (const group of document.groups.slice())
    document.groups.push({
      id: `${group.id}-copy`,
      transform: { ...IDENTITY_TRANSFORM, dx: 1000 },
    });
  for (const part of [...document.objects].filter((p) => p.group))
    document.objects.push({
      ...structuredClone(part),
      id: `${part.id}-copy`,
      group: `${part.group}-copy`,
    });
  document.groups.find((group) => group.id === "jump-upper")!.transform.dx = 20;
  return fixture;
}

export function crossAssetJumpCompilerFixture() {
  const fixture = jumpAssetCompilerFixture();
  const g = fixture.hut.gameplay!;
  const pair = g.jumpPairs?.[0];
  const lowZone = g.jumpZones?.[0],
    highZone = g.jumpZones?.[1];
  const highSurface = g.surfaces.find((s) => s.id === "east");
  const sourcePart = fixture.document.objects[0],
    sourceAsset = fixture.document.assetSources?.[0];
  if (!pair || !lowZone || !highZone || !highSurface || !sourcePart || !sourceAsset)
    throw new Error("Cross-asset jump fixture needs both sides");
  const upper = structuredClone(fixture.hut);
  upper.id = "jump-upper";
  const upperGameplay = upper.gameplay!;
  upperGameplay.collision = "none";
  upperGameplay.surfaces = [structuredClone(highSurface)];
  upperGameplay.jumpPairs = [];
  upperGameplay.jumpZones = [structuredClone(highZone)];
  upperGameplay.jumpSegments = [
    {
      id: "upper-edge",
      node: pair.node,
      long: pair.long,
      join: [100, 50, 50],
      edge: structuredClone(pair.edges[1]),
    },
  ];
  g.surfaces = g.surfaces.filter((s) => s !== highSurface);
  g.jumpPairs = [];
  g.jumpZones = [lowZone];
  g.jumpSegments = [
    {
      id: "lower-edge",
      node: pair.node,
      long: pair.long,
      join: [100, 50, 50],
      edge: structuredClone(pair.edges[0]),
    },
  ];
  fixture.assets.set(upper.id, upper);
  fixture.document.assetSources!.push({ ...sourceAsset, id: upper.id });
  fixture.document.objects.push({
    ...structuredClone(sourcePart),
    id: "jump-upper-body",
    group: "jump-upper",
    node: "asset:jump-upper:building-999",
  });
  fixture.document.groups.push({ id: "jump-upper", transform: { ...IDENTITY_TRANSFORM } });
  fixture.document.map = "Cross-asset jump fixture";
  return { ...fixture, upper };
}

export function geometricJumpCompilerFixture() {
  const fixture = crossAssetJumpCompilerFixture();
  for (const asset of [fixture.hut, fixture.upper]) {
    const segment = asset.gameplay!.jumpSegments![0]!;
    delete segment.join;
    [segment.edge.a, segment.edge.b] = [segment.edge.b, segment.edge.a];
    segment.attachment = { maxGap: 35, maxRise: 110, maxDrop: 110, minOverlap: 10 };
  }
  const placement = fixture.document.groups.find((group) => group.id === "jump-upper")!;
  placement.transform.dx = 1;
  placement.transform.dy = 110;
  fixture.document.map = "Rearranged geometric jump fixture";
  return fixture;
}

export function surfaceJumpCompilerFixture() {
  const fixture = geometricJumpCompilerFixture();
  for (const [asset, surfaceId, edge] of [
    [fixture.hut, "west", 1],
    [fixture.upper, "east", 3],
  ] as const) {
    const gameplay = asset.gameplay!;
    const surface = gameplay.surfaces.find((surface) => surface.id === surfaceId)!;
    surface.jump = {
      maxGap: 35,
      maxRise: 110,
      maxDrop: 110,
      minOverlap: 10,
      inset: 5,
      landingDepth: 6,
      edges: [edge],
      clearance: { radius: 4, height: 40 },
    };
    gameplay.jumpZones = [];
    gameplay.jumpSegments = [];
  }
  fixture.document.map = "Surface-derived jump fixture";
  return fixture;
}

export function obstructedJumpCompilerFixture() {
  const fixture = geometricJumpCompilerFixture();
  const blocker = structuredClone(fixture.upper);
  blocker.id = "jump-wall";
  blocker.gameplay = {
    version: 1,
    collision: "none",
    surfaces: [],
    doors: [],
    volumes: [
      {
        id: "jump-obstruction",
        node: "building-999",
        shape: {
          points: [
            [107, 90],
            [109, 90],
            [109, 95],
            [107, 95],
          ].map(([x, y]) => ({ x: x!, y: y!, z_bottom: 0, z_top: 200 })),
          solid: true,
          opaque: true,
          mouse: true,
          show_shadow_polygon: false,
          default_material: 0,
        },
      },
    ],
  };
  fixture.assets.set(blocker.id, blocker);
  fixture.document.assetSources!.push({ ...fixture.document.assetSources![0]!, id: blocker.id });
  fixture.document.objects.push({
    ...structuredClone(fixture.document.objects[0]!),
    id: "jump-wall-body",
    group: "jump-wall",
    node: "asset:jump-wall:building-999",
  });
  fixture.document.groups.push({ id: "jump-wall", transform: { ...IDENTITY_TRANSFORM } });
  fixture.document.map = "Obstructed geometric jump fixture";
  return fixture;
}

export function multiDestinationJumpCompilerFixture() {
  const fixture = surfaceJumpCompilerFixture();
  fixture.upper.gameplay!.surfaces.find((surface) => surface.id === "east")!.polygon = [
    [110, 20],
    [140, 20],
    [140, 80],
    [110, 80],
  ];
  fixture.hut.gameplay!.surfaces.find((surface) => surface.id === "west")!.polygon = [
    [60, 20],
    [90, 20],
    [90, 180],
    [60, 180],
  ];
  const upper = fixture.document.objects.find((part) => part.group === "jump-upper")!;
  fixture.document.objects.push({
    ...structuredClone(upper),
    id: "jump-upper-second-body",
    group: "jump-upper-second",
  });
  fixture.document.groups.push({
    id: "jump-upper-second",
    transform: { ...IDENTITY_TRANSFORM, dx: 1, dy: 200 },
  });
  fixture.document.map = "New courtyard with two jump destinations";
  return fixture;
}

export function sightTransitionCompilerFixture() {
  const fixture = movementTransitionCompilerFixture();
  const shape = structuredClone(fixture.hut.parts[0]!.obstacle_local_game!);
  const { projection_area: _projection, material_indices: _materials, ...volume } = shape;
  fixture.hut.gameplay!.volumes = [
    {
      id: "open-barrier",
      node: "building-999",
      shape: { ...volume, points: volume.points.map((p) => ({ ...p, x: p.x + 100 })) },
    },
  ];
  const transition = fixture.hut.gameplay!.movementTransitions![0]!;
  transition.initialSight = ["building-999"];
  transition.appliedSight = ["open-barrier"];
  fixture.document.map = "Sight transition fixture";
  return fixture;
}

export function preservedStateBoundaryCompilerFixture() {
  const fixture = preservedBoundaryCompilerFixture();
  const gameplay = fixture.hut.gameplay!;
  gameplay.movementTransitions = [
    {
      id: "crossing-state",
      node: "building-999",
      waypoint: [99, 69, 0],
      active: true,
      definitive: false,
      initial: gameplay.movementBlockers!,
      applied: [],
      applyPolygon: [],
      noApplyPolygon: [],
    },
  ];
  gameplay.movementBlockers = [];
  return fixture;
}

export function appearanceOnlyCompilerFixture() {
  const fixture = movementTransitionCompilerFixture();
  const transition = fixture.hut.gameplay!.movementTransitions![0]!;
  transition.initial = [];
  transition.applied = [];
  transition.appearances = ["roof"];
  return fixture;
}

export function endpointAppearanceCompilerFixture() {
  const fixture = appearanceOnlyCompilerFixture();
  const { hut, document, assets } = fixture;
  hut.gameplay!.movementTransitions![0]!.appearances = ["state"];
  const applied: GameplayAssetDescriptor["parts"][number] = {
    node: "scenery-open",
    name: "Applied roof",
    scenery: true,
  };
  hut.state_variants = {
    initial: { name: "Initial", model: hut.model, parts: hut.parts },
    applied: { name: "Applied", model: hut.model, parts: [applied] },
  };
  const alias = `${hut.id}--state-applied`;
  assets.set(alias, { ...hut, id: alias, parts: [applied] });
  document.assetSources!.push({
    ...document.assetSources![0]!,
    id: alias,
    state_variant: "applied",
  });
  const body = document.objects[0]!;
  document.objects.push({
    id: "hut-a-open",
    node: `asset:${alias}:scenery-open`,
    group: body.group,
    kind: "scenery",
    source: { map: "ignored" },
    transform: { ...body.transform },
  });
  document.groups[0]!.patches = { hut: { state: "preview-bridge" } };
  return { ...fixture, alias };
}

export function joinedTransitionCompilerFixture() {
  const fixture = sightTransitionCompilerFixture();
  const { hut, document, assets } = fixture;
  const transition = hut.gameplay!.movementTransitions![0]!;
  transition.appearances = ["roof"];
  transition.join = { key: "hall-roof", point: [...transition.waypoint] };
  const wing = structuredClone(hut);
  wing.id = "wing";
  const wingTransition = wing.gameplay!.movementTransitions![0]!;
  wingTransition.waypoint[0] -= 500;
  wingTransition.join!.point[0] -= 500;
  assets.set(wing.id, wing);
  document.assetSources!.push({ ...document.assetSources![0]!, id: wing.id });
  const part = structuredClone(document.objects[0]!);
  part.id = "wing-body";
  part.group = "wing";
  part.node = "asset:wing:building-999";
  part.transform.dx += 500;
  document.objects.push(part);
  document.groups[0]!.patches = { hut: { roof: "preview-roof" } };
  document.groups.push({
    id: "wing",
    transform: { ...IDENTITY_TRANSFORM },
    patches: { wing: { roof: "preview-roof" } },
  });
  return { ...fixture, wing, wingPart: part };
}

export function movementTransitionCompilerFixture() {
  const fixture = assetCompilerFixture();
  fixture.hut.gameplay!.movementBlockers = [];
  fixture.hut.gameplay!.movementTransitions = [
    {
      id: "barriers",
      node: "building-999",
      waypoint: [20, 20, 0],
      active: true,
      definitive: false,
      initial: [
        {
          id: "west-barrier",
          node: "building-999",
          height: 0,
          polygon: [
            [45, 0],
            [55, 0],
            [55, 100],
            [45, 100],
          ],
        },
      ],
      applied: [
        {
          id: "east-barrier",
          node: "building-999",
          height: 0,
          polygon: [
            [155, 0],
            [165, 0],
            [165, 100],
            [155, 100],
          ],
        },
      ],
      applyPolygon: [],
      noApplyPolygon: [],
    },
  ];
  return fixture;
}

export function terrainTransitionCompilerFixture() {
  const fixture = movementTransitionCompilerFixture();
  const gameplay = fixture.hut.gameplay!;
  gameplay.surfaces = [];
  gameplay.doors = [];
  gameplay.collision = "none";
  const transition = gameplay.movementTransitions![0]!;
  transition.waypoint = [50, 50, 0];
  transition.waypointReceiverSegment = [
    [50, 50, -8],
    [50, 50, 8],
  ];
  for (const surface of [...transition.initial, ...transition.applied])
    surface.terrainReach = { below: 8, above: 8 };
  fixture.assets.get("marker")!.gameplay!.surfaces = [
    {
      id: "slope",
      node: "scenery-marker",
      polygon: [
        [0, 0],
        [200, 0],
        [200, 100],
        [0, 100],
      ],
      height: [2, 6, 6, 2],
    },
    {
      id: "upper-floor",
      node: "scenery-marker",
      polygon: [
        [0, 0],
        [200, 0],
        [200, 100],
        [0, 100],
      ],
      height: 100,
    },
  ];
  fixture.document.map = "Terrain transition fixture";
  return fixture;
}

export function unavailableTerrainControlCompilerFixture() {
  const fixture = terrainTransitionCompilerFixture();
  fixture.assets.get("marker")!.gameplay!.surfaces[1]!.height = 7;
  fixture.document.map = "Unavailable terrain control fixture";
  return fixture;
}

export function soundAssetCompilerFixture() {
  const fixture = assetCompilerFixture();
  fixture.hut.gameplay!.sounds = [
    {
      id: "stream",
      node: "building-999",
      sample: 17,
      kind: 2,
      active: true,
      delay: [100, 200, 4],
      altitude: 1,
      ambiences: 255,
      spatial: {
        polyline: [
          [10, 20, 5],
          [30, 40, 5],
        ],
        innerDistance: 30,
        outerDistance: 250,
        innerVolume: 70,
        outerVolume: 10,
        noiseCoveringDistance: 60,
      },
    },
    {
      id: "night",
      node: "building-999",
      sample: 18,
      kind: 1,
      active: true,
      altitude: 2,
      ambiences: 0,
    },
  ];
  return fixture;
}

export function projectionMaterialCompilerFixture() {
  const fixture = assetCompilerFixture();
  const gameplay = fixture.hut.gameplay!;
  gameplay.collision = "none";
  gameplay.doors = [];
  gameplay.surfaces = [0, 1].map((index) => ({
    id: `platform-${index}`,
    node: "building-999",
    height: 20,
    polygon: [
      [index * 100, 0],
      [(index + 1) * 100, 0],
      [(index + 1) * 100, 100],
      [index * 100, 100],
    ],
    projectionMaterials: { defaultMaterial: index ? 4 : 2, regions: index ? [] : ["inlay"] },
  }));
  gameplay.materials = [
    {
      id: "inlay",
      node: "building-999",
      material: 5,
      ground: false,
      obstacles: [],
      polygon: [
        [10, 10, 20],
        [30, 10, 20],
        [30, 30, 20],
        [10, 30, 20],
      ],
    },
  ];
  gameplay.surfaces.push({
    id: "ground-under-platform",
    node: "building-999",
    height: 0,
    polygon: [
      [-50, -50],
      [250, -50],
      [250, 150],
      [-50, 150],
    ],
  });
  return fixture;
}

export function projectionVolumeCompilerFixture() {
  const fixture = projectionMaterialCompilerFixture();
  const gameplay = fixture.hut.gameplay!;
  const surface = gameplay.surfaces[0]!;
  delete surface.projectionMaterials;
  surface.projectionVolume = "platform-volume";
  gameplay.volumes = [
    {
      id: "platform-volume",
      node: surface.node,
      shape: {
        points: surface.polygon.map(([x, y]) => ({ x, y, z_bottom: 15, z_top: 20 })),
        solid: true,
        opaque: true,
        mouse: true,
        show_shadow_polygon: true,
        default_material: 2,
      },
    },
  ];
  gameplay.materials![0]!.obstacles = ["platform-volume"];
  gameplay.movementSolids = [];
  gameplay.movementTransitions = [
    {
      id: "platform-state",
      node: surface.node,
      waypoint: [50, 50, 20],
      active: true,
      definitive: false,
      initial: [],
      applied: [],
      initialSight: [],
      appliedSight: ["platform-volume"],
      applyPolygon: [],
      noApplyPolygon: [],
    },
  ];
  return fixture;
}

export function anchoredReceiverCompilerFixture() {
  const fixture = assetCompilerFixture();
  const gameplay = fixture.hut.gameplay!;
  gameplay.surfaces = [];
  gameplay.doors = [];
  gameplay.movementBlockers = [];
  const part = fixture.hut.parts[0]!;
  part.obstacle_local_game!.points = [
    [0, 0],
    [100, 0],
    [100, 100],
    [0, 100],
  ].map(([x, y]) => ({
    x: x!,
    y: y!,
    z_bottom: -10,
    z_top: x! / 2,
  }));
  gameplay.projectionReceivers = [
    {
      id: "slope-receiver",
      node: part.node,
      volume: part.node,
      anchor: [50, 50, 0],
    },
  ];
  fixture.assets.get("marker")!.gameplay!.surfaces = [
    {
      id: "ground",
      node: "scenery-marker",
      height: 0,
      polygon: [
        [-100, -100],
        [250, -100],
        [250, 250],
        [-100, 250],
      ],
      navigationRegion: "ground",
      preserveMovementBoundary: true,
    },
  ];
  return fixture;
}

export function receivingIslandCompilerFixture() {
  const fixture = projectionVolumeCompilerFixture();
  const gameplay = fixture.hut.gameplay!;
  gameplay.surfaces[0]!.holes = [
    [
      [20, 20],
      [80, 20],
      [80, 80],
      [20, 80],
    ],
  ];
  gameplay.surfaces[1]!.polygon = [
    [30, 30],
    [70, 30],
    [70, 70],
    [30, 70],
  ];
  gameplay.movementTransitions![0]!.waypoint = [10, 10, 20];
  return fixture;
}

export function terrainReceiverCompilerFixture() {
  const fixture = anchoredReceiverCompilerFixture();
  fixture.assets.get("marker")!.gameplay!.surfaces[0]!.height = [0, 7, 7, 0];
  fixture.hut.gameplay!.projectionReceivers![0]!.receiverSegment = [
    [50, 50, -8],
    [50, 50, 8],
  ];
  fixture.document.map = "Terrain receiver fixture";
  return fixture;
}

export function receivingGapCompilerFixture() {
  const fixture = projectionMaterialCompilerFixture();
  const gameplay = fixture.hut.gameplay!;
  const ground = gameplay.surfaces[2]!;
  gameplay.materials = [];
  gameplay.surfaces = [
    [0, 0, 100, 20],
    [0, 80, 100, 100],
    [0, 20, 20, 80],
    [80, 20, 100, 80],
  ].map(([x0, y0, x1, y1], index) => ({
    id: `platform-edge-${index}`,
    node: "building-999",
    height: 20,
    polygon: [
      [x0!, y0!],
      [x1!, y0!],
      [x1!, y1!],
      [x0!, y1!],
    ],
    projectionMaterials: {
      defaultMaterial: 2,
      regions: [],
      planePoints: [
        [100, 0, 20],
        [100, 100, 20],
        [0, 0, 20],
      ],
    },
  }));
  gameplay.surfaces.push(ground);
  fixture.document.map = "Receiving gap fixture";
  return fixture;
}

export function materialAssetCompilerFixture() {
  const fixture = assetCompilerFixture();
  fixture.hut.gameplay!.materials = [
    {
      id: "inlay",
      node: "building-999",
      material: 5,
      ground: false,
      obstacles: ["building-999"],
      polygon: [
        [40, 40, 0],
        [50, 40, 0],
        [50, 50, 0],
        [40, 50, 0],
      ],
    },
    {
      id: "paving",
      node: "building-999",
      material: 2,
      ground: true,
      obstacles: [],
      polygon: [
        [10, 10, 0],
        [30, 10, 0],
        [30, 30, 0],
        [10, 30, 0],
      ],
    },
  ];
  return fixture;
}

export function doorAnchorCompilerFixture() {
  const fixture = assetCompilerFixture();
  const door = fixture.hut.gameplay!.doors[0]!;
  door.outsideAnchor = [...door.outside];
  door.insideAnchor = [...door.inside];
  door.outside = [95, 50, 0];
  door.inside = [105, 50, 0];
  return fixture;
}

export function assetCompilerFixture() {
  const obstacle = {
    points: (
      [
        [40, 40],
        [50, 40],
        [50, 50],
        [40, 50],
      ] as const
    ).map(([x, y]) => ({ x, y, z_bottom: 0, z_top: 30 })),
    projection_area: null,
    solid: true,
    opaque: true,
    mouse: true,
    show_shadow_polygon: false,
    default_material: 0,
    material_indices: [999],
  };
  const hut: GameplayAssetDescriptor = {
    version: 1,
    kind: "projection-mapped-asset",
    id: "hut",
    name: "Two rooms",
    source_map: "unused-provenance",
    model: "hut.glb",
    parts: [
      { node: "building-999", name: "Body", source_obstacle: 999, obstacle_local_game: obstacle },
    ],
    gameplay: {
      version: 1,
      collision: "parts",
      surfaces: [
        {
          id: "west",
          node: "building-999",
          polygon: [
            [0, 0],
            [90, 0],
            [90, 100],
            [0, 100],
          ],
          height: 0,
        },
        {
          id: "east",
          node: "building-999",
          polygon: [
            [110, 0],
            [200, 0],
            [200, 100],
            [110, 100],
          ],
          height: 0,
        },
      ],
      doors: [
        {
          id: "passage",
          node: "building-999",
          polygon: [
            [90, 40],
            [110, 40],
            [110, 60],
            [90, 60],
          ],
          outside: [80, 50, 0],
          inside: [120, 50, 0],
          middle: [100, 50, 0],
          type: 0,
          locked: false,
          unlockable: false,
        },
      ],
    },
  };
  const marker: GameplayAssetDescriptor = {
    version: 1,
    kind: "projection-mapped-asset",
    id: "marker",
    name: "Scenery marker",
    source_map: "unused",
    model: "marker.glb",
    parts: [{ node: "scenery-marker", name: "Marker", scenery: true }],
    gameplay: {
      version: 1,
      collision: "none",
      surfaces: [],
      doors: [],
    },
  };
  const body: Level3DObject = {
    id: "hut-a-body",
    node: "asset:hut:building-999",
    kind: "building",
    source: { map: "ignored", obstacle: 999 },
    obstacle,
    transform: { ...IDENTITY_TRANSFORM, dx: 300, dy: 300 },
    group: "hut-a",
  };
  const document: Level3D = {
    version: 1,
    map: "Authored fixture",
    sourceMap: "never-read",
    camera: { kind: "oblique-orthographic", elevation_deg: 35 },
    size: [2000, 2000],
    sceneAssets: [],
    assetSources: [hut, marker].map((d) => ({
      id: d.id,
      descriptor: `${d.id}.json`,
      model: d.model,
      descriptor_sha256: "0".repeat(64),
      model_sha256: "0".repeat(64),
    })),
    objects: [
      body,
      {
        id: "marker",
        node: "asset:marker:scenery-marker",
        kind: "scenery",
        source: { map: "ignored" },
        transform: { ...IDENTITY_TRANSFORM, dx: 300, dy: 300 },
      },
    ],
    groups: [{ id: "hut-a", transform: { ...IDENTITY_TRANSFORM } }],
  };
  return {
    document,
    assets: new Map([
      [hut.id, hut],
      [marker.id, marker],
    ]),
    hut,
  };
}

export function slopedAssetCompilerFixture() {
  const fixture = assetCompilerFixture();
  const { hut, document } = fixture;
  hut.gameplay!.doors = [];
  hut.gameplay!.surfaces = [
    {
      id: "ramp",
      node: "building-999",
      polygon: [
        [0, 0],
        [200, 0],
        [200, 100],
        [0, 100],
      ],
      height: [0, 100, 100, 0],
      holes: [
        [
          [70, 60],
          [80, 60],
          [80, 80],
          [70, 80],
        ],
      ],
    },
  ];
  document.map = "Sloped asset fixture";
  return fixture;
}

export function clearanceAssetCompilerFixture() {
  const fixture = assetCompilerFixture();
  fixture.document.map = "Movement clearance fixture";
  fixture.hut.gameplay!.movementClearances = [
    {
      id: "pass-through",
      node: "building-999",
      height: 0,
      polygon: [
        [39, 42],
        [51, 42],
        [51, 48],
        [39, 48],
      ],
    },
  ];
  return fixture;
}

export function liftAssetCompilerFixture() {
  const fixture = assetCompilerFixture();
  const { hut } = fixture;
  hut.gameplay!.doors = [];
  const landing = hut.gameplay!.surfaces[1];
  if (!landing) throw new Error("Missing fixture landing");
  landing.height = 100;
  hut.gameplay!.surfaces.push({
    id: "stairs-surface",
    node: "building-999",
    polygon: [
      [90, 0],
      [110, 0],
      [110, 100],
      [90, 100],
    ],
    height: [0, 100, 100, 0],
  });
  hut.gameplay!.lifts = [
    {
      id: "stairs",
      node: "building-999",
      surface: "stairs-surface",
      type: 1,
      direction: [1, 0],
      doors: [
        {
          id: "stairs-low",
          node: "building-999",
          polygon: [],
          type: 5,
          outside: [80, 50, 0],
          inside: [92, 50, 10],
          middle: [90, 50, 0],
          locked: false,
          unlockable: false,
        },
        {
          id: "stairs-high",
          node: "building-999",
          polygon: [],
          type: 4,
          outside: [120, 50, 100],
          inside: [108, 50, 90],
          middle: [110, 50, 100],
          locked: false,
          unlockable: false,
        },
      ],
    },
  ];
  fixture.document.map = "Lift asset fixture";
  return fixture;
}

export function liftLightCompilerFixture() {
  const fixture = liftAssetCompilerFixture();
  const surface = fixture.hut.gameplay!.surfaces.find(
    (surface) => surface.id === "stairs-surface",
  )!;
  fixture.hut.gameplay!.lights = [
    {
      id: "night-stair-shadow",
      node: surface.node,
      ambiences: 2,
      polygon: surface.polygon.map(([x, y], index) => [
        x,
        y,
        typeof surface.height === "number" ? surface.height : surface.height[index]!,
      ]),
    },
  ];
  return fixture;
}

export function joinedNavigationCompilerFixture() {
  const fixture = compoundLiftCompilerFixture();
  for (const asset of [fixture.hut, fixture.upper]) {
    const gameplay = asset.gameplay!;
    const surface = gameplay.surfaces.find((s) => s.id === gameplay.lifts![0]!.surface)!;
    surface.navigationRegion = "roof";
    surface.navigationJoins = [
      [
        [100, 0, 40],
        [100, 100, 40],
      ],
    ];
    gameplay.surfaces = [surface];
    gameplay.collision = "none";
    gameplay.lifts = [];
    gameplay.doors = [];
  }
  fixture.document.map = "Multi-plane navigation fixture";
  return fixture;
}

export function multiPlaneRegionCompilerFixture() {
  const fixture = compoundLiftCompilerFixture();
  const gameplay = fixture.hut.gameplay!;
  const lower = gameplay.surfaces.find((s) => s.id === gameplay.lifts![0]!.surface)!;
  const upper = structuredClone(fixture.upper.gameplay!.surfaces[0]!);
  upper.id = "upper-roof";
  lower.navigationRegion = upper.navigationRegion = "roof";
  gameplay.surfaces = [lower, upper];
  gameplay.collision = "none";
  gameplay.lifts = [];
  gameplay.doors = [];
  fixture.document.objects = fixture.document.objects.filter((p) => p.group !== "upper");
  fixture.document.map = "Multi-plane navigation fixture";
  return fixture;
}

export function partialNavigationCompilerFixture() {
  const fixture = joinedNavigationCompilerFixture();
  fixture.hut.gameplay!.surfaces[0]!.navigationJoinMinimumOverlap = 24;
  fixture.hut.gameplay!.surfaces[0]!.polygon = fixture.hut.gameplay!.surfaces[0]!.polygon.map(
    ([x, y]) => [x === 90 ? 70 : x, y],
  );
  const upper = fixture.upper.gameplay!.surfaces[0]!;
  upper.navigationJoinMinimumOverlap = 24;
  upper.polygon = upper.polygon.map(([x, y]) => [x === 110 ? 130 : x, y === 0 ? 20 : 80]);
  upper.navigationJoins = [
    [
      [100, 20, 40],
      [100, 80, 40],
    ],
  ];
  fixture.document.groups.find((g) => g.id === "upper")!.transform.dy = 10;
  fixture.document.map = "Partial walkway connection";
  return fixture;
}

export function compoundLiftCompilerFixture() {
  const fixture = liftAssetCompilerFixture();
  const g = fixture.hut.gameplay!;
  const lift = g.lifts?.[0];
  if (!lift) throw new Error("Compound fixture needs a lift");
  const surface = g.surfaces.find((s) => s.id === lift.surface)!;
  const [lowDoor, sourceHighDoor] = lift.doors;
  const sourceAsset = fixture.document.assetSources?.[0];
  const sourcePart = fixture.document.objects[0];
  if (!lowDoor || !sourceHighDoor || !sourceAsset || !sourcePart)
    throw new Error("Compound fixture needs both doors and a placed asset");
  const highDoor = structuredClone(sourceHighDoor);
  highDoor.inside[2] = 88;
  const upper = structuredClone(fixture.hut);
  upper.id = "upper-stairs";
  upper.gameplay!.collision = "none";
  upper.gameplay!.surfaces = [
    {
      ...structuredClone(surface),
      polygon: [
        [100, 0],
        [110, 0],
        [110, 100],
        [100, 100],
      ],
      height: [40, 100, 100, 40],
    },
  ];
  upper.gameplay!.lifts = [{ ...structuredClone(lift), joins: [[100, 50, 40]], doors: [highDoor] }];
  surface.polygon = [
    [90, 0],
    [100, 0],
    [100, 100],
    [90, 100],
  ];
  surface.height = [0, 40, 40, 0];
  lift.joins = [[100, 50, 40]];
  lift.doors = [lowDoor];
  lowDoor.inside[2] = 8;
  fixture.assets.set(upper.id, upper);
  fixture.document.assetSources!.push({ ...sourceAsset, id: upper.id });
  const part = structuredClone(sourcePart);
  part.id = "upper-body";
  part.group = "upper";
  part.node = `asset:${upper.id}:building-999`;
  fixture.document.objects.push(part);
  fixture.document.groups.push({ id: "upper", transform: { ...IDENTITY_TRANSFORM } });
  fixture.document.map = "Compound lift fixture";
  return { ...fixture, upper };
}

export function interiorAssetCompilerFixture() {
  const fixture = assetCompilerFixture();
  fixture.hut.gameplay!.interiors = [
    {
      id: "room",
      node: "building-999",
      doors: [20, 80].map((x, i) => ({
        id: `entrance-${i}`,
        node: "building-999",
        type: 1,
        polygon: [
          [x - 5, 90],
          [x + 5, 90],
          [x + 5, 100],
          [x - 5, 100],
        ],
        outside: [x, 80, 0],
        inside: [x, 120, 0],
        middle: [x, 95, 0],
        locked: i === 1,
        unlockable: true,
        lockedCivilians: true,
      })),
    },
  ];
  fixture.document.map = "Interior asset fixture";
  return fixture;
}

export function terrainInteriorCompilerFixture() {
  const fixture = interiorAssetCompilerFixture();
  const gameplay = fixture.hut.gameplay!;
  gameplay.doors = [];
  for (const surface of gameplay.surfaces) surface.height = [0, 9, 9, 0];
  for (const door of gameplay.interiors![0]!.doors) {
    const [x, y, z] = door.outside;
    door.outsideReceiverSegment = [
      [x, y, z - 10],
      [x, y, z + 10],
    ];
  }
  fixture.document.map = "Terrain interior fixture";
  return fixture;
}

export function terrainPassageCompilerFixture() {
  const fixture = assetCompilerFixture();
  const gameplay = fixture.hut.gameplay!;
  gameplay.surfaces[0]!.height = 2;
  gameplay.surfaces[1]!.height = 6;
  const door = gameplay.doors[0]!;
  door.outsideReceiverSegment = [
    [80, 50, -8],
    [80, 50, 8],
  ];
  door.insideReceiverSegment = [
    [120, 50, -8],
    [120, 50, 8],
  ];
  door.locked = true;
  door.unlockable = true;
  door.afterTransition = {
    locked: false,
    unlockable: true,
    lockedVillains: false,
    lockedCivilians: false,
  };
  fixture.document.map = "Terrain passage fixture";
  return fixture;
}

export function joinedInteriorCompilerFixture() {
  const fixture = interiorAssetCompilerFixture();
  const room = fixture.hut.gameplay!.interiors![0]!;
  const secondDoor = room.doors.pop()!;
  fixture.hut.gameplay!.doors = [];
  room.joins = [{ point: [100, 120, 0], direction: [1, 0] }];
  const annex = structuredClone(fixture.hut);
  annex.id = "annex";
  annex.gameplay!.interiors = [
    {
      id: "room",
      node: "building-999",
      doors: [secondDoor],
      joins: [{ point: [-100, 120, 0], direction: [-1, 0] }],
    },
  ];
  const passage = structuredClone(fixture.hut);
  passage.id = "connector";
  passage.gameplay = {
    version: 1,
    collision: "none",
    surfaces: [],
    doors: [],
    interiors: [
      {
        id: "passage",
        node: "building-999",
        doors: [],
        joins: [
          { point: [-50, 120, 0], direction: [-1, 0] },
          { point: [50, 120, 0], direction: [1, 0] },
        ],
      },
    ],
  };
  for (const [descriptor, dx] of [
    [annex, 600],
    [passage, 450],
  ] as const) {
    fixture.assets.set(descriptor.id, descriptor);
    const part = structuredClone(fixture.document.objects[0]!);
    part.id = `${descriptor.id}-body`;
    part.node = `asset:${descriptor.id}:building-999`;
    part.group = descriptor.id;
    part.transform.dx = dx;
    fixture.document.objects.push(part);
    fixture.document.groups.push({ id: descriptor.id, transform: { ...IDENTITY_TRANSFORM } });
    fixture.document.assetSources!.push({
      ...fixture.document.assetSources![0]!,
      id: descriptor.id,
      descriptor: `${descriptor.id}.json`,
    });
  }
  return { ...fixture, annex, passage };
}

export function connectedInteriorCompilerFixture() {
  const fixture = joinedInteriorCompilerFixture();
  fixture.hut.gameplay!.interiors![0]!.doors.push(
    interiorAssetCompilerFixture().hut.gameplay!.interiors![0]!.doors[1]!,
  );
  fixture.document.objects = fixture.document.objects.filter((part) => part.group !== "connector");
  fixture.document.groups = fixture.document.groups.filter((group) => group.id !== "connector");
  fixture.document.assetSources = fixture.document.assetSources!.filter(
    (source) => source.id !== "connector",
  );
  fixture.assets.delete("connector");
  delete fixture.hut.gameplay!.interiors![0]!.joins;
  delete fixture.annex.gameplay!.interiors![0]!.joins;
  fixture.document.interiorConnections = [
    {
      id: "passage",
      from: { placement: "hut-a", asset: "hut", interior: "room" },
      to: { placement: "annex", asset: "annex", interior: "room" },
    },
  ];
  return fixture;
}

export function doorTransitionCompilerFixture() {
  const fixture = interiorAssetCompilerFixture();
  const gameplay = fixture.hut.gameplay!;
  for (const door of gameplay.interiors![0]!.doors)
    door.afterTransition = {
      locked: !door.locked,
      unlockable: false,
      lockedVillains: true,
      lockedCivilians: false,
    };
  gameplay.movementTransitions = [
    {
      id: "open-gate",
      node: "building-999",
      waypoint: [20, 20, 0],
      active: true,
      definitive: false,
      initial: [],
      applied: [],
      applyPolygon: [],
      noApplyPolygon: [],
      doorLinks: { mode: "trigger-transition", ids: ["passage"] },
    },
    {
      id: "room-rights",
      node: "building-999",
      waypoint: [20, 20, 0],
      active: true,
      definitive: false,
      initial: [],
      applied: [],
      applyPolygon: [],
      noApplyPolygon: [],
      doorLinks: { mode: "swap-rights", ids: ["entrance-0", "entrance-1"] },
    },
  ];
  fixture.document.map = "Door transition asset fixture";
  return fixture;
}
