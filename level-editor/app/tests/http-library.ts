import { openHttpLibrary, downloadMap } from "../src/http-library";
import { readJson, writeText, listFiles } from "../src/fs";
import { createNewMap } from "../src/new-map";

/** Real OPFS, with HTTP responses isolated from the published library. */
export async function checkHttpLibrary() {
  const root = await navigator.storage.getDirectory();
  const storage = await root.getDirectoryHandle("http-library-test", { create: true });
  const originalFetch = window.fetch;
  const originalPicker = window.showDirectoryPicker;
  const remote = new Map([
    ["/library/3d-assets/index.json", JSON.stringify({ version: 1, assets: [] })],
    ["/library/scenes/index.json", JSON.stringify(["York.rhlos-map.json"])],
    ["/library/scenes/York.rhlos-map.json", JSON.stringify({ map: "York", revision: "published" })],
    ["/library/3d-assets/york/tower/model.glb", "model bytes"],
  ]);
  const assert = (value: unknown, message: string) => {
    if (!value) throw new Error(message);
  };
  window.showDirectoryPicker = async () => {
    throw new Error("HTTP library requested a filesystem picker");
  };
  window.fetch = async (url, options) => {
    assert(!options?.method || options.method === "GET", "HTTP library attempted a write");
    const data = remote.get(String(url));
    return new Response(data ?? "", { status: data === undefined ? 404 : 200 });
  };
  try {
    const workspace = await storage.getDirectoryHandle("sherwood-level-editor", { create: true });
    const storedMaps = await workspace.getDirectoryHandle("maps", { create: true });
    await writeText(
      storedMaps,
      "Old forest.level3d.json",
      JSON.stringify({ map: "Old forest", revision: "legacy" }),
    );
    const assets = [{ id: "frame", editor: { gameplay: { surfaces: [] } } }];
    const shard = JSON.stringify({ version: 1, assets });
    const hash = Array.from(
      new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(shard))),
      (byte) => byte.toString(16).padStart(2, "0"),
    ).join("");
    const path = `3d-assets/catalog-${hash}.json`;
    remote.set("/library/" + path, shard);
    const model = "chunked model bytes";
    const digest = async (value: string) =>
      Array.from(
        new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(value))),
        (byte) => byte.toString(16).padStart(2, "0"),
      ).join("");
    const parts = [];
    for (const chunk of [model.slice(0, 7), model.slice(7)]) {
      const sha256 = await digest(chunk);
      const path = `3d-assets/model-chunk-${sha256}.bin`;
      remote.set("/library/" + path, chunk);
      parts.push({ path, sha256 });
    }
    remote.set(
      "/library/3d-assets/index.json",
      JSON.stringify({
        version: 1,
        assets: [],
        asset_shards: [{ path, sha256: hash }],
        model_shards: {
          "3d-assets/chunked.glb": { bytes: model.length, sha256: await digest(model), parts },
        },
      }),
    );
    let connection = await openHttpLibrary("/library/", storage);
    let library = connection.handle;
    const assetDirectory = await library.getDirectoryHandle("3d-assets");
    assert(
      (await (await (await assetDirectory.getFileHandle("chunked.glb")).getFile()).text()) ===
        model,
      "Chunked runtime model bytes changed",
    );
    const hydrated = await readJson<{ assets: unknown[] }>(assetDirectory, "index.json");
    assert(
      JSON.stringify(hydrated.assets) === JSON.stringify(assets),
      "Catalog shards lost embedded gameplay",
    );
    const nextAssets = [{ ...assets[0], descriptor_sha256: "a".repeat(64) }];
    const nextShard = JSON.stringify({ version: 1, assets: nextAssets });
    const nextHash = await digest(nextShard);
    const nextPath = `3d-assets/catalog-${nextHash}.json`;
    remote.set("/library/" + nextPath, nextShard);
    remote.set(
      "/library/3d-assets/index.json",
      JSON.stringify({
        version: 1,
        assets: [],
        asset_shards: [{ path: nextPath, sha256: nextHash }],
      }),
    );
    const refreshed = await readJson<{ assets: unknown[] }>(assetDirectory, "index.json");
    assert(
      JSON.stringify(refreshed.assets) === JSON.stringify(nextAssets),
      "Open HTTP library retained stale asset descriptor pins after a catalog update",
    );
    assert(
      (await readJson<{ revision: string }>(storedMaps, "Old forest.rhlos-map.json")).revision ===
        "legacy",
      "Existing browser save was not migrated",
    );
    assert(
      !(await listFiles(storedMaps)).includes("Old forest.level3d.json"),
      "Migrated browser save retained its old filename",
    );
    await storedMaps.removeEntry("Old forest.rhlos-map.json");
    assert(
      (await connection.mapLabels()).get("York") === "York (WIP)",
      "Published map incorrectly marked modified",
    );
    let maps = await library.getDirectoryHandle("scenes");
    assert(
      (await readJson<{ revision: string }>(maps, "York.rhlos-map.json")).revision === "published",
      "Published map did not load",
    );
    await writeText(
      maps,
      "York.rhlos-map.json",
      JSON.stringify({ map: "York", revision: "edited" }),
    );
    assert(
      (await connection.mapLabels()).get("York (Modified)") === "York (WIP) (Modified)",
      "Saving did not expose the modified copy",
    );
    await writeText(
      storedMaps,
      "York.level3d.json",
      JSON.stringify({ map: "York", revision: "older save" }),
    );
    connection = await openHttpLibrary("/library/", storage);
    library = connection.handle;
    assert(
      (await connection.mapLabels()).get("York (Modified)") === "York (WIP) (Modified)",
      "Modified label did not survive reopening",
    );
    maps = await library.getDirectoryHandle("scenes");
    assert(
      (await readJson<{ revision: string }>(maps, "York (Modified).rhlos-map.json")).revision ===
        "edited",
      "Browser save did not survive reopening",
    );
    assert(
      (await readJson<{ revision: string }>(storedMaps, "York.level3d.json")).revision ===
        "older save",
      "Conflicting legacy save must remain recoverable",
    );
    assert(
      (await readJson<{ revision: string }>(maps, "York.rhlos-map.json")).revision === "published",
      "Original is no longer independently loadable",
    );
    assert(
      connection.documentMap("York (Modified)") === "York",
      "Modified map lost its document identity",
    );
    assert(
      connection.savedMapName("York") === "York (Modified)" &&
        connection.savedMapName("York (Modified)") === "York (Modified)",
      "Repeated saves must use the same copy",
    );
    await writeText(
      maps,
      "York (Modified).rhlos-map.json",
      JSON.stringify({ map: "York", revision: "edited again" }),
    );
    assert(
      (await readJson<{ revision: string }>(maps, "York (Modified).rhlos-map.json")).revision ===
        "edited again",
      "Saving modified copy failed",
    );
    assert(
      JSON.parse(remote.get("/library/scenes/York.rhlos-map.json")!).revision === "published",
      "Published map changed",
    );
    await createNewMap(library, "New forest");
    assert(
      !(await connection.mapLabels()).has("New forest"),
      "Custom map was given a published-map label",
    );
    assert(
      (await listFiles(maps)).sort().join(",") ===
        "New forest.rhlos-map.json,York (Modified).rhlos-map.json,York.rhlos-map.json",
      "Original and modified maps must be separate entries",
    );
    let rejected = false;
    try {
      await createNewMap(library, "York");
    } catch {
      rejected = true;
    }
    assert(rejected, "New map overwrote published map");
    const assetDir = await (
      await (await library.getDirectoryHandle("3d-assets")).getDirectoryHandle("york")
    ).getDirectoryHandle("tower");
    assert(
      (await (await (await assetDir.getFileHandle("model.glb")).getFile()).text()) ===
        "model bytes",
      "Nested HTTP asset did not load",
    );
    rejected = false;
    try {
      await assetDir.getFileHandle("model.glb", { create: true });
    } catch {
      rejected = true;
    }
    assert(rejected, "Asset write was allowed");
    const document = await readJson(maps, "New forest.rhlos-map.json");
    let exported: Promise<unknown> | undefined;
    const click = HTMLAnchorElement.prototype.click;
    HTMLAnchorElement.prototype.click = function () {
      assert(
        /^New forest_\d{4}-\d{2}-\d{2}T\d{2}-\d{2}-\d{2}\.rhlos-map\.json$/.test(this.download),
        "Download filename must include a local timestamp to the second and the .rhlos-map.json extension",
      );
      exported = originalFetch(this.href).then((response) => response.json());
    };
    try {
      downloadMap("New forest", document);
    } finally {
      HTMLAnchorElement.prototype.click = click;
    }
    assert(
      JSON.stringify(await exported) === JSON.stringify(document),
      "Downloaded map differs from saved map",
    );
  } finally {
    window.fetch = originalFetch;
    window.showDirectoryPicker = originalPicker;
    await root.removeEntry("http-library-test", { recursive: true });
  }
}
