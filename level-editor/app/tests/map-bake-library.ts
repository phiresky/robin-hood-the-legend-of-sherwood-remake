import { openHttpLibrary } from "../src/http-library.ts";
import { prepareMapCandidate } from "../src/map-candidate.ts";
import { EditorViewport } from "../src/editor-viewport.ts";
import { MapExportWorker } from "../src/map-export-client.ts";
import { readPinnedAssetDescriptors } from "../src/projection-library.ts";
import { serializeStoredMap } from "@rle/shared";
import { unzipSync, strFromU8 } from "fflate";
import { decode } from "fast-png";

async function imageHash(data: Uint8Array | Uint8ClampedArray | Uint16Array) {
  const bytes = new Uint8Array(data.byteLength);
  bytes.set(new Uint8Array(data.buffer, data.byteOffset, data.byteLength));
  return Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes))).join(",");
}

function check(condition: boolean, message: string) {
  if (!condition) throw new Error(message);
}

const result = document.querySelector("#result")!;
let viewport: EditorViewport | undefined;
let worker: MapExportWorker | undefined;
try {
  const parameters = new URLSearchParams(location.search);
  const name = parameters.get("map") ?? "leicester";
  const library = await openHttpLibrary(parameters.get("library") ?? "/library/");
  let candidate = await prepareMapCandidate(name, library.handle, null, (count, total) => {
    result.textContent = `RUNNING loading ${count}/${total}`;
  });
  viewport = new EditorViewport({
    document: () => candidate.document,
    selection: () => null,
    level: () => null,
    showObstacles: () => false,
    showElevation: () => false,
    onSelection: () => {},
    commitTransform: () => {},
  });
  viewport.replaceMap(
    candidate.asset,
    candidate.ground,
    candidate.sources,
    candidate.document.assetSources,
  );
  const assets = await readPinnedAssetDescriptors(
    library.handle,
    candidate.document.assetSources ?? [],
    candidate.document.sceneAssets,
  );
  let heartbeats = 0;
  const heartbeat = setInterval(() => heartbeats++, 0);
  worker = new MapExportWorker();
  const exportWorker = worker;
  let baked;
  try {
    baked = await viewport.bakeMapAsync(
      candidate.document,
      assets,
      ({ stage, completed, total }) => {
        result.textContent = `RUNNING ${stage}${total ? `: ${Math.round((completed / total) * 100)}%` : ""}`;
      },
      (bounds, calibratedAssets) => {
        if (!calibratedAssets)
          throw new Error("Library bake requires calibrated asset definitions");
        return exportWorker.compile(candidate.document, bounds, calibratedAssets);
      },
    );
  } finally {
    clearInterval(heartbeat);
  }
  if (heartbeats < 2) throw new Error("Bake did not yield to browser input and progress updates");
  const { compiled, pixels, appearance } = baked;
  const roundtrip = parameters.get("roundtrip") === "1";
  const originalHashes = roundtrip
    ? await Promise.all([imageHash(pixels.color), imageHash(pixels.depth)])
    : undefined;
  result.textContent = "RUNNING encoding images and packaging ZIP";
  const archive = await worker.package(compiled, pixels, appearance);
  (window as unknown as { __bakeZip: Uint8Array }).__bakeZip = archive;
  if (roundtrip) {
    const files = unzipSync(archive);
    const readJson = (path: string) => {
      const bytes = files[path];
      if (!bytes) throw new Error(`ZIP omitted ${path}`);
      return JSON.parse(strFromU8(bytes));
    };
    check(
      JSON.stringify(readJson(`Data/Levels/${compiled.name}.level.json`)) ===
        JSON.stringify(compiled.descriptor),
      "ZIP changed compiled geometry",
    );
    const report = readJson("compile-report.json");
    check(
      JSON.stringify(report.warnings) === JSON.stringify(compiled.warnings),
      "ZIP lost warnings",
    );
    const prefix = `Data/Levels/Day/${compiled.name}`;
    const color = decode(files[`${prefix}.map.png`]!);
    const depth = decode(files[`${prefix}.occlusion-depth.png`]!);
    check(color.depth === 8 && color.channels === 4, "Invalid baked color format");
    check(depth.depth === 16 && depth.channels === 1, "Invalid baked depth format");
    check(
      [color, depth].every(
        (image) => image.width === compiled.bounds[2] && image.height === compiled.bounds[3],
      ),
      "ZIP changed baked image dimensions",
    );
    check(
      depth.data.some((value) => value !== depth.data[0]),
      "Rendered fixture has no depth variation",
    );
    check(
      (await imageHash(color.data)) === originalHashes![0] &&
        (await imageHash(depth.data)) === originalHashes![1],
      "ZIP PNGs changed baked pixels",
    );
    result.textContent = "RUNNING reopening ZIP editor document and pinned models";
    const stored = readJson(report.editor_document);
    const reopened = await prepareMapCandidate(
      name,
      library.handle,
      null,
      undefined,
      candidate.document.map,
      stored,
    );
    check(
      JSON.stringify(serializeStoredMap(reopened.document, assets)) === JSON.stringify(stored),
      "Reopened editor document changed authoring data",
    );
    candidate = reopened;
    viewport.replaceMap(
      candidate.asset,
      candidate.ground,
      candidate.sources,
      candidate.document.assetSources,
    );
    const rebaked = await viewport.bakeMapAsync(
      candidate.document,
      assets,
      ({ stage }) => {
        result.textContent = `RUNNING round trip: ${stage}`;
      },
      (bounds, calibratedAssets) =>
        exportWorker.compile(candidate.document, bounds, calibratedAssets!),
    );
    check(
      JSON.stringify(rebaked.compiled.descriptor) === JSON.stringify(compiled.descriptor),
      "Reopened map changed compiled gameplay",
    );
    check(
      (await imageHash(rebaked.pixels.color)) === originalHashes![0] &&
        (await imageHash(rebaked.pixels.depth)) === originalHashes![1],
      "Reopened map changed rendered color or depth",
    );
    check(
      JSON.stringify(rebaked.compiled.warnings) === JSON.stringify(compiled.warnings),
      "Rebake changed warnings",
    );
  }
  const obstacles =
    compiled.descriptor.asset_geometry?.sight_obstacles.length ??
    compiled.descriptor.volumes.length;
  result.textContent = `PASS ${name}: ${compiled.bounds[2]}x${compiled.bounds[3]}, ${obstacles} sight obstacles, ${archive.length} ZIP bytes${roundtrip ? "; reopened gameplay and rendered color/depth match" : ""}`;
} catch (error) {
  result.textContent = `FAIL ${error instanceof Error ? error.stack : String(error)}`;
} finally {
  worker?.dispose();
  viewport?.dispose();
}
