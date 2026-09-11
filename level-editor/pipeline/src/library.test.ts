import test, { type TestContext } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { spawn } from "node:child_process";
import type { AssetDescriptor } from "@rle/shared";
import { AssetLibrary } from "./library.ts";

function descriptor(id: string): AssetDescriptor {
  return {
    id,
    name: id,
    tags: ["fixture"],
    scale_class: "unique",
    source: {
      map: "York",
      ambiance: "Day",
      bbox: [0, 0, 10, 20],
      extraction: { tool: "test" },
    },
    origin: [0, 0],
    anchor: [0, 0],
    images: { day: "day.png", mask: "mask.png" },
    volumes: { sight_obstacles: [] },
    motion: { obstacles: [], walkable: [] },
  };
}

async function fixture(t: TestContext) {
  const directory = await fs.mkdtemp(
    path.join(os.tmpdir(), "library-publication-"),
  );
  t.after(() => fs.rm(directory, { recursive: true, force: true }));
  const library = new AssetLibrary(directory);
  const index = path.join(directory, "index.json");
  const read = async () =>
    JSON.parse(await fs.readFile(index, "utf8")) as {
      id: string;
      [key: string]: unknown;
    }[];
  return { directory, library, index, read };
}

test("same-owner parallel publications preserve sorted entries and opaque existing metadata", async (t) => {
  const { directory, library, index, read } = await fixture(t);
  await Promise.all(
    ["z", "a", "m"].map((id) => library.writeAsset(descriptor(id), {})),
  );
  const entries = await read();
  assert.deepEqual(
    entries.map((e) => e.id),
    ["a", "m", "z"],
  );
  entries[0]!.extension = { retained: true };
  await fs.writeFile(index, JSON.stringify(entries));
  await library.writeAsset(descriptor("m"), { "day.png": Buffer.from("new") });
  assert.deepEqual((await read())[0]!.extension, { retained: true });
  assert.equal(
    await fs.readFile(path.join(directory, "m", "day.png"), "utf8"),
    "new",
  );
  assert.deepEqual((await fs.readdir(directory)).sort(), [
    "a",
    "index.json",
    "m",
    "z",
  ]);
});

test("malformed or structurally invalid existing index never becomes an empty library", async (t) => {
  const { directory, library, index } = await fixture(t);
  await library.writeAsset(descriptor("old"), {
    "day.png": Buffer.from("old bytes"),
  });
  const original = await fs.readFile(index, "utf8");
  for (const broken of [
    "{broken",
    "{}",
    "[null]",
    JSON.stringify([{ id: "x" }]),
    `[${original.slice(1, -1)},${original.slice(1, -1)}]`,
  ]) {
    await fs.writeFile(index, broken);
    await assert.rejects(
      library.writeAsset(descriptor("old"), {
        "day.png": Buffer.from("replacement"),
      }),
      /Invalid library index/,
    );
    assert.equal(await fs.readFile(index, "utf8"), broken);
    assert.equal(
      await fs.readFile(path.join(directory, "old", "day.png"), "utf8"),
      "old bytes",
    );
    assert.equal((await fs.readdir(directory)).includes(".index.lock"), false);
  }
  await fs.writeFile(index, original);
  await library.writeAsset(descriptor("new"), {}); // Failure does not poison this owner's queue.
});

test("index read errors fail before asset modification and release the acquired lease", async (t) => {
  const { directory, library, index } = await fixture(t);
  await library.writeAsset(descriptor("old"), {});
  const original = await fs.readFile(index, "utf8");
  const readFile = fs.readFile;
  const mocked = t.mock.method(
    fs,
    "readFile",
    async (...args: Parameters<typeof fs.readFile>) => {
      if (args[0] === index)
        throw Object.assign(new Error("permission denied"), { code: "EACCES" });
      return readFile(...args);
    },
  );
  await assert.rejects(
    library.writeAsset(descriptor("new"), {}),
    /Cannot read library index/,
  );
  mocked.mock.restore();
  assert.equal(await fs.readFile(index, "utf8"), original);
  assert.deepEqual((await fs.readdir(directory)).sort(), ["index.json", "old"]);
});

for (const failure of ["write", "rename"] as const) {
  test(`failed temporary index ${failure} retains the complete previous index`, async (t) => {
    const { directory, library, index } = await fixture(t);
    await library.writeAsset(descriptor("old"), {});
    const original = await fs.readFile(index, "utf8");
    const writeFile = fs.writeFile;
    const rename = fs.rename;
    if (failure === "write") {
      t.mock.method(
        fs,
        "writeFile",
        async (...args: Parameters<typeof fs.writeFile>) => {
          if (String(args[0]).includes(".index-publish-"))
            throw Object.assign(new Error("index write failed"), {
              code: "ENOSPC",
            });
          return writeFile(...args);
        },
      );
    } else {
      t.mock.method(
        fs,
        "rename",
        async (...args: Parameters<typeof fs.rename>) => {
          if (args[1] === index)
            throw Object.assign(new Error("index rename failed"), {
              code: "EIO",
            });
          return rename(...args);
        },
      );
    }
    await assert.rejects(
      library.writeAsset(descriptor("new"), {}),
      /index .* failed/,
    );
    assert.equal(await fs.readFile(index, "utf8"), original);
    assert.equal(
      (await fs.readdir(directory)).some((name) => name.startsWith(".index")),
      false,
    );
    // The index is atomic, not the asset directory: successfully written but
    // unindexed files remain available for inspection/retry after this failure.
    assert.ok(await fs.stat(path.join(directory, "new", "asset.json")));
  });
}

test("a crash-left lease gives actionable recovery guidance and is never stolen", async (t) => {
  const { directory, library } = await fixture(t);
  const lock = path.join(directory, ".index.lock");
  await fs.mkdir(lock);
  await assert.rejects(library.writeAsset(descriptor("new"), {}), (error) => {
    assert.ok(error instanceof Error);
    assert.ok(error.message.includes(lock));
    assert.match(error.message, /verify that no writer is active/);
    return true;
  });
  assert.deepEqual(await fs.readdir(directory), [".index.lock"]);
});

test("asset and image names cannot escape or replace publication metadata", async (t) => {
  const { directory, library } = await fixture(t);
  for (const id of [
    "../outside",
    ".index.lock",
    "index.json",
    ".index-publish-reserved",
  ])
    await assert.rejects(
      library.writeAsset(descriptor(id), {}),
      /Invalid asset ID/,
    );
  await assert.rejects(
    library.writeAsset(descriptor("safe"), {
      "../index.json": Buffer.from("overwrite"),
    }),
    /Invalid image name/,
  );
  assert.deepEqual(await fs.readdir(directory), []);
});

test(
  "independent processes cannot overwrite one another's index or retire an active lease",
  { timeout: 15000 },
  async (t) => {
    const { directory, library, read } = await fixture(t);
    await library.writeAsset(descriptor("old"), {});
    function writer(id: string, pause: boolean) {
      const source = `
      import fs from 'node:fs/promises';
      import { AssetLibrary } from ${JSON.stringify(new URL("./library.ts", import.meta.url).href)};
      const original = fs.writeFile;
      if (${pause}) fs.writeFile = async (...args) => {
        if (String(args[0]).endsWith('/asset.json')) {
          process.stdout.write('LEASED\\n');
          await new Promise(resolve => process.stdin.once('data', resolve));
          process.stdin.pause();
        }
        return original(...args);
      };
      try {
        await new AssetLibrary(${JSON.stringify(directory)}).writeAsset(${JSON.stringify(descriptor(id))}, {});
      } catch (error) { process.stdout.write(error.message); process.exitCode = 2; }
    `;
      const child = spawn(
        process.execPath,
        ["--input-type=module", "-e", source],
        { stdio: ["pipe", "pipe", "pipe"] },
      );
      let stdout = "";
      let stderr = "";
      child.stdout.on("data", (bytes) => {
        stdout += String(bytes);
      });
      child.stderr.on("data", (bytes) => {
        stderr += String(bytes);
      });
      const done = new Promise<number | null>((resolve, reject) => {
        child.once("error", reject);
        child.once("close", resolve);
      });
      t.after(async () => {
        if (child.exitCode === null) child.kill("SIGKILL");
        await done;
      });
      return { child, done, stdout: () => stdout, stderr: () => stderr };
    }
    const first = writer("first", true);
    await new Promise<void>((resolve, reject) => {
      const received = () => {
        if (first.stdout().includes("LEASED")) resolve();
      };
      first.child.stdout.on("data", received);
      first.done.then(
        () =>
          reject(
            new Error(
              `writer exited before acquiring lease: ${first.stderr()}`,
            ),
          ),
        reject,
      );
    });
    const second = writer("second", false);
    assert.equal(await second.done, 2, second.stderr());
    assert.match(second.stdout(), /Library publication is locked/);
    assert.deepEqual(
      (await read()).map((e) => e.id),
      ["old"],
    );
    assert.ok(await fs.stat(path.join(directory, ".index.lock")));
    await assert.rejects(fs.stat(path.join(directory, "second")), {
      code: "ENOENT",
    });
    first.child.stdin.end("continue");
    assert.equal(await first.done, 0, first.stderr());
    const retry = writer("second", false);
    assert.equal(await retry.done, 0, retry.stderr());
    assert.deepEqual(
      (await read()).map((e) => e.id),
      ["first", "old", "second"],
    );
  },
);
