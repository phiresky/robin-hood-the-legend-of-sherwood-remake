import test from "node:test";
import assert from "node:assert/strict";
import { subdir, readJson } from "./fs.ts";

test("only missing optional directories are absence", async () => {
  const root = (name: string) =>
    ({
      getDirectoryHandle: async () => {
        throw new DOMException("failed", name);
      },
    }) as unknown as FileSystemDirectoryHandle;
  assert.equal(await subdir(root("NotFoundError"), ["scenes"]), null);
  await assert.rejects(subdir(root("NotAllowedError"), ["scenes"]), /Cannot open directory scenes/);
});
test("malformed JSON carries filename instead of returning defaults", async () => {
  const dir = {
    getFileHandle: async () => ({
      getFile: async () => ({ text: async () => "{" }),
    }),
  } as unknown as FileSystemDirectoryHandle;
  await assert.rejects(readJson(dir, "map.rhlos-map.json"), /Invalid JSON in map.rhlos-map.json/);
});

test("virtual JSON reads return parsed values without making a file or retaining an old value", async () => {
  const { setJsonReader } = await import("./fs.ts");
  const directory = {
    getFileHandle: async () => {
      throw new Error("No file serialization expected");
    },
  } as unknown as FileSystemDirectoryHandle;
  let value = { version: 1, assets: [{ id: "first" }] };
  setJsonReader(directory, "index.json", async () => value);
  assert.equal(await readJson(directory, "index.json"), value);
  value = { version: 1, assets: [{ id: "second" }] };
  assert.equal(await readJson(directory, "index.json"), value);
  await assert.rejects(readJson(directory, "other.json"), /No file serialization expected/);
});
