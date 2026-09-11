import test from "node:test";
import assert from "node:assert/strict";
import { loadProtoLevel, scanDatadir } from "./datadir.ts";

test("connection indexes map names without reading unused missions or ambiance directories", async () => {
  const levels = {
    async *entries() {
      for (const name of [
        "York.rhp.json",
        "leicester.rhp.json",
        "broken.rhm.json",
      ])
        yield [name, { kind: "file" }];
      yield ["InaccessibleDay", { kind: "directory" }];
    },
    getDirectoryHandle: async () => {
      throw new Error("must not inspect ambiance directories");
    },
    getFileHandle: async () => {
      throw new Error(
        "must not read unused mission headers or unopened levels",
      );
    },
  } as unknown as FileSystemDirectoryHandle;
  const data = {
    getDirectoryHandle: async (name: string) => {
      assert.equal(name, "Levels");
      return levels;
    },
  } as unknown as FileSystemDirectoryHandle;
  const root = {
    getDirectoryHandle: async (name: string) => {
      assert.equal(name, "Data");
      return data;
    },
  } as unknown as FileSystemDirectoryHandle;
  const index = await scanDatadir(root);
  assert.deepEqual([...index.maps], ["York", "leicester"]);
  assert.equal(index.maps.size, 2);
  assert.equal(index.levelsDir, levels);
});

test("level opening preserves requested filename and still validates required data", async () => {
  const requested: string[] = [];
  const levelsDir = {
    getFileHandle: async (name: string) => {
      requested.push(name);
      return {
        getFile: async () => ({
          text: async () => JSON.stringify({ format: "Future" }),
        }),
      };
    },
  } as unknown as FileSystemDirectoryHandle;
  await assert.rejects(
    loadProtoLevel({ maps: new Set(["York"]), levelsDir }, "York"),
    /level.format/,
  );
  assert.deepEqual(requested, ["York.rhp.json"]);
});

test("missing or inaccessible required level directory still rejects connection", async () => {
  for (const errorName of ["NotFoundError", "NotAllowedError"]) {
    const root = {
      getDirectoryHandle: async () => {
        throw new DOMException("unavailable", errorName);
      },
    } as unknown as FileSystemDirectoryHandle;
    await assert.rejects(
      scanDatadir(root),
      errorName === "NotFoundError"
        ? /Data\/Levels\/ missing/
        : /Cannot open directory Data\/Levels/,
    );
  }
});
