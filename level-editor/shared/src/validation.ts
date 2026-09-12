import type { Level3D } from "./level3d.ts";
import type { ProtoLevel } from "./level.ts";
import type { SceneDoc } from "./scene.ts";
import type { AssetDescriptor } from "./asset.ts";
import type { TerrainSpec } from "./terrain.ts";

export function parseTerrainSpec(value: unknown): TerrainSpec {
  const d = object(value, "terrain");
  const points = (v: unknown, path: string) =>
    array(v, path).forEach((p, i) => tuple(p, 2, `${path}[${i}]`));
  for (const [i, road] of array(d.roads === undefined ? [] : d.roads, "terrain.roads").entries()) {
    object(road, "road");
    points(road.points, `terrain.roads[${i}].points`);
    finite(road.width, "road.width");
    check(road.width > 0, "road.width", "must be positive");
  }
  for (const region of array(d.regions === undefined ? [] : d.regions, "terrain.regions")) {
    object(region, "region");
    check(["grass", "dirt", "canopy", "water"].includes(region.material), "region.material", "unsupported material");
    points(region.polygon, "region.polygon");
  }
  for (const wall of array(d.walls === undefined ? [] : d.walls, "terrain.walls")) {
    object(wall, "wall");
    text(wall.asset, "wall.asset");
    points(wall.points, "wall.points");
    if (wall.segment_set !== undefined)
      array(wall.segment_set, "wall.segment_set").forEach((id) => text(id, "wall.segment_set[]"));
    if (wall.spacing !== undefined) {
      finite(wall.spacing, "wall.spacing");
      check(wall.spacing > 0, "wall.spacing", "must be positive");
    }
  }
  if (d.swatches !== undefined) {
    for (const [role, id] of Object.entries(object(d.swatches, "terrain.swatches"))) {
      check(["grass", "dirt", "road", "canopy", "water"].includes(role), "terrain.swatches", `unknown role ${role}`);
      text(id, `terrain.swatches.${role}`);
    }
  }
  return value as TerrainSpec;
}

/** Validate fields used by library transforms while preserving exporter extras. */
export function parseAssetDescriptor(value: unknown): AssetDescriptor {
  const d = object(value, "asset");
  for (const key of ["id", "name"]) text(d[key], `asset.${key}`);
  check(!/[\\/\0]/.test(d.id) && d.id !== "." && d.id !== "..", "asset.id", "expected filename component");
  array(d.tags, "asset.tags").forEach((tag) => text(tag, "asset.tags[]"));
  check(["unique", "variant", "spline-segment", "texture"].includes(d.scale_class), "asset.scale_class", "unsupported scale class");
  tuple(d.origin, 2, "asset.origin");
  tuple(d.anchor, 2, "asset.anchor");
  const source = object(d.source, "asset.source");
  text(source.map, "asset.source.map");
  text(source.ambiance, "asset.source.ambiance");
  tuple(source.bbox, 4, "asset.source.bbox");
  check(source.bbox[2] > 0 && source.bbox[3] > 0, "asset.source.bbox", "dimensions must be positive");
  text(object(source.extraction, "asset.source.extraction").tool, "asset.source.extraction.tool");
  const images = object(d.images, "asset.images");
  for (const key of ["day", "mask"]) text(images[key], `asset.images.${key}`);
  for (const key of ["fog", "night"])
    if (images[key] !== undefined) text(images[key], `asset.images.${key}`);
  const volumes = object(d.volumes, "asset.volumes");
  array(volumes.sight_obstacles, "asset.volumes.sight_obstacles").forEach((v) => {
    object(v, "volume");
    for (const key of ["opaque", "solid"]) check(typeof v[key] === "boolean", `volume.${key}`, "expected boolean");
    array(v.points, "volume.points").forEach((p) => {
      object(p, "volume.point");
      for (const key of ["x", "y", "z_bottom", "z_top"]) finite(p[key], `volume.point.${key}`);
    });
  });
  const motion = object(d.motion, "asset.motion");
  for (const key of ["obstacles", "walkable"]) array(motion[key], `asset.motion.${key}`).forEach((v) => {
    object(v, "motion polygon");
    finite(v.layer, "motion.layer");
    array(object(v.polygon, "motion.polygon").points, "motion.polygon.points").forEach((p) => tuple(p, 2, "motion.point"));
  });
  if (d.wall_direction_deg !== undefined) finite(d.wall_direction_deg, "asset.wall_direction_deg");
  if (d.fx !== undefined) {
    const fx = object(d.fx, "asset.fx");
    for (const key of ["bank", "profile", "action"]) text(fx[key], `asset.fx.${key}`);
    tuple(fx.position, 2, "asset.fx.position");
    tuple(fx.hotspot, 2, "asset.fx.hotspot");
    finite(fx.elevation, "asset.fx.elevation");
    check(Number.isInteger(fx.frame_count) && fx.frame_count > 0, "asset.fx.frame_count", "expected positive frame count");
  }
  if (d.merged_from !== undefined) array(d.merged_from, "asset.merged_from").forEach((id) => text(id, "asset.merged_from[]"));
  for (const model of [d.model, ...Object.values(d.alt_models === undefined ? {} : object(d.alt_models, "asset.alt_models"))]) {
    if (model === undefined) continue;
    const m = object(model, "asset.model");
    text(m.glb, "model.glb");
    check(typeof m.textured === "boolean", "model.textured", "expected boolean");
    if (m.pose_l2c !== undefined) {
      const pose = object(m.pose_l2c, "model.pose_l2c");
      tuple(pose.rotation, 4, "model.pose_l2c.rotation");
      tuple(pose.translation, 3, "model.pose_l2c.translation");
      tuple(pose.scale, 3, "model.pose_l2c.scale");
    }
    finite(m.fit_iou, "model.fit_iou");
    if (m.fit_appearance !== undefined) finite(m.fit_appearance, "model.fit_appearance");
    const bounds = object(m.bounds_local, "model.bounds_local");
    tuple(bounds.min, 3, "model.bounds_local.min");
    tuple(bounds.max, 3, "model.bounds_local.max");
    const p = object(m.placement, "model.placement");
    text(p.asset, "model.placement.asset");
    tuple(p.position, 3, "model.placement.position");
    tuple(p.rotation, 4, "model.placement.rotation");
    finite(p.scale, "model.placement.scale");
    check(p.scale > 0, "model.placement.scale", "must be positive");
    const extraction = object(m.extraction, "model.extraction");
    text(extraction.tool, "model.extraction.tool");
    tuple(extraction.crop, 4, "model.extraction.crop");
  }
  return value as AssetDescriptor;
}

function object(v: unknown, path: string): Record<string, any> {
  if (!v || typeof v !== "object" || Array.isArray(v))
    throw new Error(`${path}: expected object`);
  return v as Record<string, any>;
}
function check(ok: unknown, path: string, message: string): asserts ok {
  if (!ok) throw new Error(`${path}: ${message}`);
}
function finite(v: unknown, path: string) {
  check(
    typeof v === "number" && Number.isFinite(v),
    path,
    "expected finite number",
  );
}
function text(v: unknown, path: string) {
  check(
    typeof v === "string" && v.length > 0,
    path,
    "expected nonempty string",
  );
}
function array(v: unknown, path: string): any[] {
  check(Array.isArray(v), path, "expected array");
  return v;
}
function tuple(v: unknown, n: number, path: string) {
  const a = array(v, path);
  check(a.length === n, path, `expected ${n} coordinates`);
  a.forEach((x, i) => finite(x, `${path}[${i}]`));
}
function camera(v: unknown, path: string) {
  const c = object(v, path);
  check(c.kind === "oblique-orthographic", path, "unsupported camera");
  finite(c.elevation_deg, path);
  check(
    c.elevation_deg > 0 && c.elevation_deg < 90,
    path,
    "elevation must be between 0 and 90 degrees",
  );
}
function base(v: unknown, path: string) {
  const d = object(v, path);
  check(d.version === 1, path, `unsupported version ${d.version}`);
  text(d.map, `${path}.map`);
  tuple(d.size, 2, `${path}.size`);
  check(
    d.size.every((x: number) => x > 0),
    path,
    "size must be positive",
  );
  camera(d.camera, `${path}.camera`);
  return d;
}
function obstacle(v: unknown, path: string) {
  const o = object(v, path);
  array(o.points, `${path}.points`).forEach((p, i) => {
    object(p, path);
    for (const k of ["x", "y", "z_bottom", "z_top"])
      finite(p[k], `${path}.points[${i}].${k}`);
    check(p.z_bottom <= p.z_top, path, "inverted obstacle height");
  });
  for (const k of ["opaque", "solid", "mouse", "show_shadow_polygon"])
    check(typeof o[k] === "boolean", `${path}.${k}`, "expected boolean");
  finite(o.default_material, `${path}.default_material`);
  array(o.material_indices, `${path}.material_indices`).forEach((x) =>
    check(Number.isInteger(x) && x >= 0, path, "invalid material index"),
  );
}
function transform(v: unknown, path: string) {
  const t = object(v, path);
  for (const k of ["dx", "dy", "dz", "rot_deg"]) finite(t[k], `${path}.${k}`);
}

function elementFx(value: unknown, path: string) {
  const fx = object(value, path);
  const sprite = object(fx.sprite, `${path}.sprite`);
  // Empty names are serialized for inactive/no-sprite FX; require strings,
  // without rejecting the converter's legitimate absence representation.
  for (const key of ["frame_profile_name", "profile_name"]) {
    check(
      typeof sprite[key] === "string",
      `${path}.sprite.${key}`,
      "expected string",
    );
  }
  for (const key of ["position_x", "position_y", "elevation"])
    finite(sprite[key], `${path}.sprite.${key}`);
  finite(fx.blit_type, `${path}.blit_type`);
  for (const key of ["active", "force_display"])
    check(typeof fx[key] === "boolean", `${path}.${key}`, "expected boolean");
  array(fx.display_polyline, `${path}.display_polyline`).forEach((point, i) =>
    tuple(point, 2, `${path}.display_polyline[${i}]`),
  );
}

/** Validate in place: unknown Rust/exporter fields survive every round trip. */
export function parseProtoLevel(value: unknown): ProtoLevel {
  const d = object(value, "level");
  check(
    d.format === "Demo" || d.format === "Fullgame",
    "level.format",
    `unsupported Rust level format ${d.format}`,
  );
  array(d.sight_obstacles, "level.sight_obstacles").forEach((o, i) =>
    obstacle(o, `level.sight_obstacles[${i}]`),
  );
  for (const k of [
    "patches",
    "animations",
    "material_sectors",
    "light_sectors",
    "elevation_lines",
    "masks",
    "sound_sources",
    "jump_zones",
    "jump_line_pairs",
    "lifts",
    "buildings",
  ])
    array(d[k], `level.${k}`);
  d.animations.forEach((fx: unknown, i: number) =>
    elementFx(fx, `level.animations[${i}]`),
  );
  d.patches.forEach((value: unknown, i: number) => {
    const path = `level.patches[${i}]`;
    const patch = object(value, path);
    for (const key of ["active", "integrate_in_background"])
      check(
        typeof patch[key] === "boolean",
        `${path}.${key}`,
        "expected boolean",
      );
    elementFx(patch.element_fx, `${path}.element_fx`);
  });
  d.elevation_lines.forEach((e: any, i: number) => {
    object(e, "elevation line");
    tuple(e.point_a, 2, `elevation_lines[${i}].point_a`);
    tuple(e.point_b, 2, `elevation_lines[${i}].point_b`);
  });
  const polygon = (p: unknown, path: string) =>
    array(object(p, path).points, `${path}.points`).forEach((point, i) =>
      tuple(point, 2, `${path}.points[${i}]`),
    );
  for (const key of ["material_sectors", "light_sectors", "jump_zones"])
    d[key].forEach((item: any, i: number) =>
      polygon(object(item, key).polygon, `${key}[${i}].polygon`),
    );
  d.material_sectors.forEach((item: any, i: number) =>
    finite(item.material, `material_sectors[${i}].material`),
  );
  d.masks.forEach((mask: any, i: number) => {
    object(mask, "mask");
    tuple(mask.box_top_left, 2, `masks[${i}].box_top_left`);
    tuple(mask.box_size, 2, `masks[${i}].box_size`);
    for (const key of ["character_polyline", "projectile_polyline"])
      array(mask[key], `masks[${i}].${key}`).forEach((point, j) =>
        tuple(point, 2, `masks[${i}].${key}[${j}]`),
      );
    array(mask.obstacle_indices, `masks[${i}].obstacle_indices`).forEach(
      (index) =>
        check(
          Number.isInteger(index) &&
            index >= 0 &&
            index < d.sight_obstacles.length,
          `masks[${i}]`,
          "dangling obstacle index",
        ),
    );
    array(mask.mask_data, `masks[${i}].mask_data`);
  });
  object(d.misc, "level.misc");
  object(d.motion_data, "level.motion_data");
  array(d.motion_data.layers, "level.motion_data.layers");
  array(d.motion_data.graph_bytes, "level.motion_data.graph_bytes");
  d.motion_data.layers.forEach((layer: unknown, i: number) =>
    array(layer, `motion_data.layers[${i}]`).forEach((area) => {
      polygon(object(area, "motion area").polygon, "motion area.polygon");
      array(area.obstacles, "motion area.obstacles").forEach((o) =>
        polygon(
          object(o, "motion obstacle").polygon,
          "motion obstacle.polygon",
        ),
      );
    }),
  );
  return value as ProtoLevel;
}

export function parseSceneDoc(value: unknown): SceneDoc {
  const d = base(value, "scene");
  array(d.placements, "scene.placements").forEach((p, i) => {
    object(p, "placement");
    text(p.asset, `placements[${i}].asset`);
    tuple(p.position, 3, "placement.position");
    tuple(p.rotation, 4, "placement.rotation");
    finite(p.scale, "placement.scale");
    check(p.scale > 0, "placement.scale", "must be positive");
  });
  if (d.ground) {
    text(d.ground.texture, "ground.texture");
    tuple(d.ground.rect, 4, "ground.rect");
  }
  return value as SceneDoc;
}

export interface DocumentContext {
  scene?: Pick<SceneDoc, "size" | "camera">;
  map?: string;
  glb?: string;
  level?: ProtoLevel;
  nodes?: ReadonlySet<string>;
  sourceSha256?: string;
  glbSha256?: string;
}

/** Source hash uses the complete parsed export, including unknown fields, in JSON key order. */
export async function documentProvenance(
  level: ProtoLevel | null,
  glb: ArrayBuffer,
) {
  const hash = async (bytes: ArrayBuffer) =>
    Array.from(
      new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)),
      (b) => b.toString(16).padStart(2, "0"),
    ).join("");
  return {
    source_sha256: level
      ? await hash(new TextEncoder().encode(JSON.stringify(level)).buffer)
      : undefined,
    glb_sha256: await hash(glb),
  };
}

export function parseLevel3D(
  value: unknown,
  context: DocumentContext = {},
): Level3D {
  const d = base(value, "level3d");
  if (context.scene) {
    check(
      d.size[0] === context.scene.size[0] &&
        d.size[1] === context.scene.size[1],
      "level3d.size",
      "does not match source scene",
    );
    check(
      d.camera.elevation_deg === context.scene.camera.elevation_deg,
      "level3d.camera",
      "does not match source scene",
    );
  }
  text(d.glb, "level3d.glb");
  if (context.map)
    check(
      d.map.toLowerCase() === context.map.toLowerCase(),
      "level3d.map",
      `expected source ${context.map}, got ${d.map}`,
    );
  if (context.glb)
    check(d.glb === context.glb, "level3d.glb", `expected ${context.glb}`);
  const ids = new Set<string>();
  const groups = new Set<string>();
  for (const g of array(d.groups, "level3d.groups")) {
    object(g, "level3d.groups[]");
    text(g.id, "group.id");
    check(!ids.has(g.id), g.id, "duplicate ID");
    ids.add(g.id);
    groups.add(g.id);
    transform(g.transform, g.id);
    if (g.hidden !== undefined)
      check(typeof g.hidden === "boolean", g.id, "invalid hidden flag");
  }
  for (const o of array(d.objects, "level3d.objects")) {
    object(o, "level3d.objects[]");
    text(o.id, "object.id");
    check(!ids.has(o.id), o.id, "duplicate ID");
    ids.add(o.id);
    check(
      o.kind === "building" || o.kind === "terrace",
      o.id,
      "unsupported kind",
    );
    text(o.node, `${o.id}.node`);
    if (context.nodes)
      check(context.nodes.has(o.node), o.id, `missing source node ${o.node}`);
    object(o.source, `${o.id}.source`);
    check(o.source.map === d.map, o.id, "mismatched source map");
    check(
      Number.isInteger(o.source.obstacle) && o.source.obstacle >= 0,
      o.id,
      "invalid obstacle index",
    );
    if (context.level)
      check(
        o.source.obstacle < context.level.sight_obstacles.length,
        o.id,
        "dangling obstacle index",
      );
    if (o.group !== undefined)
      check(groups.has(o.group), o.id, `dangling group ${o.group}`);
    if (o.hidden !== undefined)
      check(typeof o.hidden === "boolean", o.id, "invalid hidden flag");
    obstacle(o.obstacle, `${o.id}.obstacle`);
    check(
      o.obstacle.points.length >= 3,
      o.id,
      "editable obstacle needs at least three points",
    );
    transform(o.transform, o.id);
  }
  if (d.provenance !== undefined) {
    const p = object(d.provenance, "provenance");
    for (const [key, expected] of [
      ["source_sha256", context.sourceSha256],
      ["glb_sha256", context.glbSha256],
    ]) {
      if (p[key!] !== undefined) {
        check(
          typeof p[key!] === "string" && /^[a-f0-9]{64}$/.test(p[key!]),
          `provenance.${key}`,
          "expected SHA-256",
        );
        if (expected)
          check(
            p[key!] === expected,
            `provenance.${key}`,
            "source content changed; regenerate or explicitly migrate the document",
          );
      }
    }
  }
  return value as Level3D;
}
