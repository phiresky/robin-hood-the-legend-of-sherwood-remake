// Use the editor's embedded catalog reader for headless compilation checks.
import fs from "node:fs/promises";
import path from "node:path";
import assert from "node:assert/strict";
import { readPinnedAssetDescriptors } from "../app/src/projection-library.ts";
import { expandStoredMap, parseStoredMap } from "../shared/src/index.ts";

const libraries = new Map();
function directory(base) {
  return {
    async getDirectoryHandle(name) {
      const child = path.join(base, name);
      assert.ok((await fs.stat(child)).isDirectory(), `Not a directory: ${child}`);
      return directory(child);
    },
    async getFileHandle(name) {
      const file = path.join(base, name);
      assert.ok((await fs.stat(file)).isFile(), `Not a file: ${file}`);
      return {
        async getFile() {
          return { text: () => fs.readFile(file, "utf8") };
        },
      };
    },
  };
}

export async function readIndexedMap(file, library) {
  const root = path.resolve(library);
  if (!libraries.has(root)) libraries.set(root, directory(root));
  const raw = JSON.parse(await fs.readFile(file, "utf8"));
  const expanded = expandStoredMap(raw);
  const warnings = [];
  const assets = await readPinnedAssetDescriptors(
    libraries.get(root),
    expanded.assetSources ?? [],
    expanded.sceneAssets ?? [],
    (warning) => warnings.push(warning),
  );
  assert.deepEqual(warnings, [], "Indexed verification requires current scene pins");
  return { document: parseStoredMap(raw, assets), assets };
}
