import { loadHttpAssetCatalog, loadHttpModelParts } from "./http-asset-catalog.ts";

/** Retain only model routing, never a second copy of embedded asset descriptors. */
export function createHttpCatalogReader(base: string) {
  let modelShards: Awaited<ReturnType<typeof loadHttpAssetCatalog>>["model_shards"];
  let initialized = false;
  let initializing: Promise<void> | undefined;
  const read = async () => {
    const catalog = await loadHttpAssetCatalog(base);
    modelShards = catalog.model_shards;
    initialized = true;
    return catalog;
  };
  return {
    // Explicit index reads always refresh both descriptors and model routing.
    read,
    async readModel(path: string): Promise<Uint8Array<ArrayBuffer> | null> {
      if (!initialized) {
        // Direct model access also works before the first index read. Concurrent
        // model requests share routing initialization without retaining its catalog.
        initializing ??= read()
          .then(() => {})
          .finally(() => {
            initializing = undefined;
          });
        await initializing;
      }
      const parts = modelShards?.[path];
      return parts ? loadHttpModelParts(base, parts) : null;
    },
  };
}
