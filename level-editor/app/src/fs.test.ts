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
  await assert.rejects(
    subdir(root("NotAllowedError"), ["scenes"]),
    /Cannot open directory scenes/,
  );
});
test("malformed JSON carries filename instead of returning defaults", async () => {
  const dir = {
    getFileHandle: async () => ({
      getFile: async () => ({ text: async () => "{" }),
    }),
  } as unknown as FileSystemDirectoryHandle;
  await assert.rejects(
    readJson(dir, "map.level3d.json"),
    /Invalid JSON in map.level3d.json/,
  );
});
