/** Hydrate immutable catalog batches used when the published library exceeds one file. */
async function sha256(bytes: ArrayBuffer | Uint8Array<ArrayBuffer>) {
  return Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)), (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
}

type ModelParts = { bytes: number; sha256: string; parts: { path: string; sha256: string }[] };

/** Reassemble the exact runtime GLB when its bytes exceed the host's per-file limit. */
export async function loadHttpModelParts(base: string, model: ModelParts) {
  if (!Number.isSafeInteger(model.bytes) || model.bytes < 0 || !Array.isArray(model.parts))
    throw new Error("Invalid library model parts");
  const bytes = new Uint8Array(model.bytes);
  let offset = 0;
  for (const part of model.parts) {
    if (
      !part ||
      !/^[a-f0-9]{64}$/.test(part.sha256) ||
      part.path !== `3d-assets/model-chunk-${part.sha256}.bin`
    )
      throw new Error("Invalid library model part");
    const response = await fetch(base + part.path, { cache: "no-cache" });
    if (!response.ok) throw new Error(`Cannot load library model part (${response.status})`);
    const data = await response.arrayBuffer();
    if ((await sha256(data)) !== part.sha256) throw new Error("Library model part hash mismatch");
    if (offset + data.byteLength > bytes.length) throw new Error("Library model size mismatch");
    bytes.set(new Uint8Array(data), offset);
    offset += data.byteLength;
  }
  if (offset !== bytes.length || (await sha256(bytes)) !== model.sha256)
    throw new Error("Library model hash or size mismatch");
  return bytes;
}

export async function loadHttpAssetCatalog(base: string) {
  const response = await fetch(base + "3d-assets/index.json", { cache: "no-store" });
  if (!response.ok) throw new Error(`Cannot load library catalog (${response.status})`);
  const catalog = await response.json();
  if (!catalog || !Array.isArray(catalog.assets)) throw new Error("Invalid asset library index");
  if (catalog.asset_shards !== undefined) {
    if (!Array.isArray(catalog.asset_shards) || catalog.assets.length)
      throw new Error("Invalid asset library shards");
    const seen = new Set<string>();
    for (const shard of catalog.asset_shards) {
      if (
        !shard ||
        typeof shard.sha256 !== "string" ||
        !/^[a-f0-9]{64}$/.test(shard.sha256) ||
        shard.path !== `3d-assets/catalog-${shard.sha256}.json` ||
        seen.has(shard.path)
      )
        throw new Error("Invalid asset library shard");
      seen.add(shard.path);
      const result = await fetch(base + shard.path, { cache: "no-cache" });
      if (!result.ok) throw new Error(`Cannot load library shard (${result.status})`);
      const bytes = await result.arrayBuffer();
      const hash = await sha256(bytes);
      if (hash !== shard.sha256) throw new Error("Library shard hash mismatch");
      const data = JSON.parse(new TextDecoder().decode(bytes));
      if (data.version !== catalog.version || !Array.isArray(data.assets))
        throw new Error("Invalid asset library shard data");
      catalog.assets.push(...data.assets);
    }
    delete catalog.asset_shards;
  }
  return catalog;
}
