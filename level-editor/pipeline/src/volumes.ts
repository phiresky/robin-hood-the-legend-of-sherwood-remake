// Reconstruction orchestration; algorithms and export are independently importable stages.
import fs from "node:fs/promises";
import { constants } from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import sharp from "sharp";
import { snapFloatingParts, type MapCamera, type SceneDoc } from "@rle/shared";
import { libraryDir, workDir, loadEnvironment } from "./env.ts";
import { findMapPng, loadProtoLevel, mapImageSource } from "./asset-writer.ts";
import { fitMapCamera } from "./map-camera.ts";
import {
  buildGeometry,
  footprintArea,
  TERRACE_AREA,
  type Geometry,
} from "./volume-geometry.ts";
import { rasterOwners, type Owners } from "./volume-raster.ts";
import {
  buildTextures,
  synthesisOptions,
  type Textured,
  type Fill,
  type TextureOptions,
} from "./volume-fill.ts";
import { encode, exportGlb } from "./volume-export.ts";
import { renders } from "./volume-diagnostics.ts";
import { pathComponent } from "./inputs.ts";
import { isMissing } from "./provider-cache.ts";
export { buildGeometry } from "./volume-geometry.ts";
export type { Face, Geometry } from "./volume-geometry.ts";
export { rasterOwners } from "./volume-raster.ts";
export type { Owners } from "./volume-raster.ts";
export { buildTextures } from "./volume-fill.ts";
export type { Textured, Fill } from "./volume-fill.ts";
export { encode, exportGlb } from "./volume-export.ts";
/** everything the volume reconstruction of a map produces, for the exporter, the renders and the bake */
export interface Reconstruction {
  map: string;
  level: Awaited<ReturnType<typeof loadProtoLevel>>;
  cam: MapCamera;
  size: [number, number];
  mapPng: Buffer;
  mapRgb: Buffer;
  g: Geometry;
  own: Owners;
  tex: Textured;
  fill: Fill;
}

export interface ReconstructOptions {
  fill?: Fill;
  textures?: Partial<TextureOptions>;
  opaqueOnly?: boolean;
  flat?: boolean;
  /** move non-opaque parts that look displaced along the view ray onto their support (see snapFloatingParts) */
  snap?: boolean;
  /** when the requested fill is synth but the CLI is missing, fall back to proc (default) instead of throwing */
  fallbackFill?: boolean;
}

/** the synth fill if the texture-synthesis CLI is installed, else proc */
export async function defaultFill(
  binary = synthesisOptions().synthBinary,
): Promise<Fill> {
  try {
    await fs.access(binary, constants.X_OK);
    return "synth";
  } catch (error) {
    if (isMissing(error)) return "proc";
    throw error;
  }
}

/** Reconstruct a map; synthesis/debug stages may write to the supplied work directory. */
export async function reconstruct(
  map: string,
  opts: ReconstructOptions = {},
): Promise<Reconstruction> {
  pathComponent(map, "map");
  const textures = synthesisOptions(opts.textures);
  const synthBinary = textures.synthBinary;
  let fill: Fill = opts.fill ?? "synth";
  if (!["proc", "synth", "smear", "none"].includes(fill))
    throw new Error(`unknown fill ${fill}`);
  if (fill === "synth" && (await defaultFill(synthBinary)) !== "synth") {
    if (opts.fallbackFill === false)
      throw new Error(
        `--fill synth needs the texture-synthesis CLI at ${synthBinary} (cargo install --locked texture-synthesis-cli)`,
      );
    console.warn(
      `texture-synthesis CLI not found at ${synthBinary}: falling back to --fill proc`,
    );
    fill = "proc";
  }
  const raw = await loadProtoLevel(map);
  // repair parts stored displaced along the view ray (see snapFloatingParts)
  const terracesForSnap = new Set<number>();
  raw.sight_obstacles.forEach((o, i) => {
    if (o.points.length >= 3 && footprintArea(o.points) > TERRACE_AREA)
      terracesForSnap.add(i);
  });
  // the detector cannot tell an overhang or a cornice gap from a displaced piece, so
  // snapping is opt-in here; the editor lists the suspects and snaps per part on request
  const snap = opts.snap
    ? snapFloatingParts(raw.sight_obstacles, terracesForSnap)
    : {
        obstacles: raw.sight_obstacles,
        snapped: [],
        suspects: snapFloatingParts(raw.sight_obstacles, terracesForSnap, {
          includeOpaque: true,
        }).snapped,
      };
  if (snap.snapped.length)
    console.log(
      `snapped ${snap.snapped.length} floating parts onto their supports: ${snap.snapped
        .slice(0, 8)
        .map((s) => `#${s.index}→#${s.support} Δ${s.delta.toFixed(0)}`)
        .join(", ")}${snap.snapped.length > 8 ? ", …" : ""}`,
    );
  else if (snap.suspects.length)
    console.log(
      `${snap.suspects.length} parts look displaced along the view ray (not moved; --snap moves the non-opaque ones): ${snap.suspects
        .slice(0, 10)
        .map((s) => `#${s.index} Δ${s.delta.toFixed(0)}`)
        .join(", ")}${snap.suspects.length > 10 ? ", …" : ""}`,
    );
  const level = { ...raw, sight_obstacles: snap.obstacles };
  const fit = fitMapCamera(level);
  const cam: MapCamera = { kind: fit.kind, elevation_deg: fit.elevation_deg };
  console.log(`${map}: camera elevation ${fit.elevation_deg.toFixed(2)}°`);
  const src =
    (await mapImageSource(map, "Day", true, level)) ??
    (await findMapPng("Day", map));
  if (!src) throw new Error(`no Day map for ${map}`);
  const mapPng = await sharp(src).png().toBuffer();
  const meta = await sharp(mapPng).metadata();
  const size: [number, number] = [meta.width!, meta.height!];
  const mapRgb = await sharp(mapPng).removeAlpha().raw().toBuffer();

  const g = buildGeometry(
    level,
    cam,
    opts.opaqueOnly ?? false,
    opts.flat ?? false,
  );
  console.log(
    `${g.obstacles} obstacles (${g.terraceIds.size} terraces: ${[
      ...g.terraceIds,
    ]
      .map(
        (i) =>
          `#${i} h${Math.round(Math.max(...level.sight_obstacles[i]!.points.map((p) => p.z_top)))}`,
      )
      .join(", ")}) -> ${g.faces.length} faces, ${g.tris.length / 3} triangles`,
  );
  const own = rasterOwners(g, cam, size[0], size[1]);
  const sceneWork = path.join(
    textures.workDirectory,
    `${map.toLowerCase()}-scene`,
  );
  if (fill === "synth") await fs.mkdir(sceneWork, { recursive: true });
  const synthDir =
    fill === "synth"
      ? await fs.mkdtemp(path.join(sceneWork, "synth-"))
      : sceneWork;
  let tex: Textured;
  try {
    tex = await buildTextures(
      g,
      own,
      cam,
      mapRgb,
      size[0],
      size[1],
      fill,
      synthDir,
      textures,
    );
  } finally {
    if (fill === "synth")
      await fs.rm(synthDir, { recursive: true, force: true });
  }
  const s = tex.stats;
  console.log(
    `atlas ${s.atlasSide}²: ${s.own} faces with own pixels (${s.donorFilled} completed from a donor; ${s.unknownPx} tile px without own data, ${s.leftPx} ${fill === "none" ? "left transparent" : "unfillable"}), ${s.borrowedWalls} hidden walls borrowing a visible wall, ${s.borrowedRoofs} hidden roofs borrowing a neighbouring roof, ${s.flat} faces ${fill === "none" ? "transparent" : "in mean colour"}`,
  );
  return { map, level, cam, size, mapPng, mapRgb, g, own, tex, fill };
}

async function main() {
  loadEnvironment();
  const argv = process.argv.slice(2);
  const get = (flag: string): string | undefined => {
    const i = argv.indexOf(`--${flag}`);
    return i >= 0 ? argv[i + 1] : undefined;
  };
  const map = get("map");
  if (!map)
    throw new Error(
      "usage: --map <name> [--render] [--fill proc|synth|smear|none] [--opaque-only] [--flat] [--out dir]",
    );
  const outDir = get("out") ?? path.join(libraryDir, "scenes");
  const fillArg = get("fill") as Fill | undefined;
  if (
    fillArg &&
    fillArg !== "proc" &&
    fillArg !== "synth" &&
    fillArg !== "smear" &&
    fillArg !== "none"
  )
    throw new Error(`unknown --fill ${fillArg}`);
  const r = await reconstruct(map, {
    textures: {
      ...(process.env.TEXTURE_SYNTHESIS
        ? { synthBinary: process.env.TEXTURE_SYNTHESIS }
        : {}),
      debug: !!process.env.VOLUMES_DEBUG,
      dump: process.env.VOLUMES_DUMP,
      idAtlas: !!process.env.VOLUMES_ID_ATLAS,
    },
    fill: fillArg,
    opaqueOnly: argv.includes("--opaque-only"),
    flat: argv.includes("--flat"),
    snap: argv.includes("--snap"),
    fallbackFill: !fillArg,
  });
  const { g, tex, cam, size, mapPng, fill } = r;

  await fs.mkdir(outDir, { recursive: true });
  const stem = `${map.toLowerCase()}-volumes${fill === "none" ? "-holes" : fill === "synth" ? "" : `-${fill}`}`;
  const glbFile = path.join(outDir, `${stem}.scene.glb`);
  await exportGlb(glbFile, g, tex, cam, size, fill);
  const doc: SceneDoc = {
    version: 1,
    map,
    size,
    camera: cam,
    placements: [],
    notes: `sight-obstacle volumes textured by reverse projection of the Day map, per-face atlas, fill=${fill} (volumes.ts)`,
  };
  await fs.writeFile(
    path.join(outDir, `${stem}.scene.json`),
    JSON.stringify(doc, null, 2),
  );
  const st = await fs.stat(glbFile);
  console.log(`wrote ${glbFile} (${(st.size / 1e6).toFixed(1)} MB)`);
  const atlasOut = path.join(
    workDir,
    `${map.toLowerCase()}-scene`,
    `${stem.slice(map.length + 1)}-atlas.${fill === "none" ? "png" : "jpg"}`,
  );
  await fs.mkdir(path.dirname(atlasOut), { recursive: true });
  await fs.writeFile(atlasOut, (await encode(tex.atlas, fill)).data);
  await fs.writeFile(
    atlasOut.replace("-atlas.", "-ground."),
    (await encode(tex.ground, fill)).data,
  );

  // --closeups x,y;x,y;… : orbit close-ups around these map points (4 yaws each)
  const closeups = (get("closeups") ?? "")
    .split(";")
    .filter(Boolean)
    .map((p) => p.split(",").map(Number) as [number, number]);
  if (argv.includes("--render")) {
    await renders(
      map,
      g,
      tex,
      cam,
      size,
      mapPng,
      stem.slice(map.length + 1),
      closeups,
      false,
      {
        closeupSpan: get("closeup-span")
          ? Number(get("closeup-span"))
          : undefined,
        closeupYaws: get("closeup-yaws")?.split(",").map(Number),
      },
    );
    // --debug-fill: the close-ups again with every face in its fill-category colour
    if (argv.includes("--debug-fill"))
      await renders(
        map,
        g,
        tex,
        cam,
        size,
        mapPng,
        stem.slice(map.length + 1),
        closeups,
        true,
        {
          closeupSpan: get("closeup-span")
            ? Number(get("closeup-span"))
            : undefined,
          closeupYaws: get("closeup-yaws")?.split(",").map(Number),
        },
      );
  }
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  main().catch((e) => {
    console.error(e);
    process.exit(1);
  });
}
