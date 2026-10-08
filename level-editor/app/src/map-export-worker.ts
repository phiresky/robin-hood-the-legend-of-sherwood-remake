import type { Level3D, ProjectionAssetDescriptor } from "@rle/shared";
import { compileMap, packageCompiledMap, type BakeBounds, type BakePixels } from "./map-compile.ts";
import type { BakedAppearanceRegion } from "./map-appearance.ts";
import type { SceneryResources } from "./scenery-resources.ts";

export type CompiledMap = ReturnType<typeof compileMap>;
export type ExportRequest =
  | {
      kind: "compile";
      document: Level3D;
      bounds: BakeBounds;
      assets: Map<string, ProjectionAssetDescriptor>;
    }
  | {
      kind: "package";
      compiled: CompiledMap;
      pixels: BakePixels;
      appearance: BakedAppearanceRegion[];
      scenery?: SceneryResources;
    };
export type ExportResponse =
  | { kind: "progress"; stage: string }
  | { kind: "compiled"; compiled: CompiledMap }
  | { kind: "packaged"; bytes: Uint8Array }
  | { kind: "error"; message: string };

globalThis.addEventListener("message", async (event: MessageEvent<ExportRequest>) => {
  try {
    const request = event.data;
    if (request.kind === "compile") {
      const compiled = compileMap(request.document, request.bounds, request.assets, {
        bestEffort: true,
        onProgress: (stage) =>
          globalThis.postMessage({ kind: "progress", stage } satisfies ExportResponse, {}),
      });
      globalThis.postMessage({ kind: "compiled", compiled } satisfies ExportResponse, {});
    } else {
      const bytes = await packageCompiledMap(
        request.compiled,
        request.pixels,
        request.appearance,
        request.scenery,
      );
      globalThis.postMessage({ kind: "packaged", bytes } satisfies ExportResponse, {
        transfer: [bytes.buffer],
      });
    }
  } catch (error) {
    globalThis.postMessage({ kind: "error", message: String(error) } satisfies ExportResponse, {});
  }
});
