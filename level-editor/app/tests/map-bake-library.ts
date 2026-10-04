import { openHttpLibrary } from "../src/http-library.ts";
import { prepareMapCandidate } from "../src/map-candidate.ts";
import { EditorViewport } from "../src/editor-viewport.ts";
import { MapExportWorker } from "../src/map-export-client.ts";
import { readPinnedAssetDescriptors } from "../src/projection-library.ts";

const result = document.querySelector("#result")!;
let viewport: EditorViewport | undefined;
let worker: MapExportWorker | undefined;
try {
  const parameters = new URLSearchParams(location.search);
  const name = parameters.get("map") ?? "leicester";
  const library = await openHttpLibrary(parameters.get("library") ?? "/library/");
  const candidate = await prepareMapCandidate(name, library.handle, null, (count, total) => {
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
        if (!calibratedAssets) throw new Error("Library bake requires calibrated asset definitions");
        return exportWorker.compile(candidate.document, bounds, calibratedAssets);
      },
    );
  } finally {
    clearInterval(heartbeat);
  }
  if (heartbeats < 2) throw new Error("Bake did not yield to browser input and progress updates");
  const { compiled, pixels, appearance } = baked;
  result.textContent = "RUNNING encoding images and packaging ZIP";
  const archive = await worker.package(compiled, pixels, appearance);
  (window as unknown as { __bakeZip: Uint8Array }).__bakeZip = archive;
  const obstacles =
    compiled.descriptor.asset_geometry?.sight_obstacles.length ??
    compiled.descriptor.volumes.length;
  result.textContent = `PASS ${name}: ${compiled.bounds[2]}x${compiled.bounds[3]}, ${obstacles} sight obstacles, ${archive.length} ZIP bytes`;
} catch (error) {
  result.textContent = `FAIL ${error instanceof Error ? error.stack : String(error)}`;
} finally {
  worker?.dispose();
  viewport?.dispose();
}
