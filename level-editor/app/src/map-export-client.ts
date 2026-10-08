import type { Level3D, ProjectionAssetDescriptor } from "@rle/shared";
import type { BakeBounds, BakePixels } from "./map-compile.ts";
import type { BakedAppearanceRegion } from "./map-appearance.ts";
import type { CompiledMap, ExportRequest, ExportResponse } from "./map-export-worker.ts";
import type { SceneryResources } from "./scenery-resources.ts";

/** One export owns one worker, including CPU-heavy PNG encoding and ZIP packaging. */
export class MapExportWorker {
  private readonly worker = new Worker(new URL("./map-export-worker.ts", import.meta.url), {
    type: "module",
  });
  private pending:
    | { resolve: (response: ExportResponse) => void; reject: (error: Error) => void }
    | undefined;
  private disposed = false;
  constructor(onProgress?: (stage: string) => void) {
    this.worker.onmessage = (event: MessageEvent<ExportResponse>) => {
      if (event.data.kind === "progress") {
        if (this.pending && !this.disposed) {
          try {
            onProgress?.(event.data.stage);
          } catch (error) {
            this.pending?.reject(error instanceof Error ? error : new Error(String(error)));
            this.pending = undefined;
            this.dispose();
          }
        }
        return;
      }
      const pending = this.pending;
      this.pending = undefined;
      if (event.data.kind === "error") pending?.reject(new Error(event.data.message));
      else pending?.resolve(event.data);
    };
    this.worker.onerror = (event) => {
      this.pending?.reject(new Error(event.message || "Map export worker failed."));
      this.pending = undefined;
      this.dispose();
    };
    this.worker.onmessageerror = () => {
      this.pending?.reject(new Error("The map export worker returned unreadable data."));
      this.pending = undefined;
      this.dispose();
    };
  }
  private request(request: ExportRequest, transfer: Transferable[] = []): Promise<ExportResponse> {
    if (this.disposed) return Promise.reject(new Error("Map export was cancelled."));
    if (this.pending) return Promise.reject(new Error("Map export worker is already busy."));
    return new Promise((resolve, reject) => {
      this.pending = { resolve, reject };
      try {
        this.worker.postMessage(request, transfer);
      } catch (error) {
        this.pending = undefined;
        reject(error);
      }
    });
  }
  async compile(
    document: Level3D,
    bounds: BakeBounds,
    assets: ReadonlyMap<string, ProjectionAssetDescriptor>,
  ) {
    const response = await this.request({
      kind: "compile",
      document,
      bounds,
      assets: new Map(assets),
    });
    if (response.kind !== "compiled") throw new Error("Unexpected map compilation response.");
    return response.compiled;
  }
  async package(
    compiled: CompiledMap,
    pixels: BakePixels,
    appearance: BakedAppearanceRegion[],
    scenery: SceneryResources = {},
  ) {
    // Packaging takes ownership of the bake buffers; exporting must not retain a second copy.
    const buffers = new Set<ArrayBuffer>();
    for (const state of [pixels, ...appearance.flatMap((region) => region.states)]) {
      for (const view of [state.color, state.depth]) {
        if (!(view.buffer instanceof ArrayBuffer))
          throw new Error("Export pixels require transferable buffers.");
        buffers.add(view.buffer);
      }
    }
    for (const bytes of Object.values(scenery)) {
      if (!(bytes.buffer instanceof ArrayBuffer))
        throw new Error("Scenery requires transferable buffers.");
      buffers.add(bytes.buffer);
    }
    const response = await this.request(
      { kind: "package", compiled, pixels, appearance, scenery },
      [...buffers],
    );
    if (response.kind !== "packaged") throw new Error("Unexpected map packaging response.");
    return response.bytes;
  }
  dispose() {
    this.disposed = true;
    this.worker.terminate();
    this.pending?.reject(new Error("Map export was cancelled."));
    this.pending = undefined;
  }
}
