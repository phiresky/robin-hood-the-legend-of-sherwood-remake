import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs/promises";
import { spawn } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

// Run from any directory against an already-running editor development server.
// Each browser finishes before the next starts to bound bake memory consumption.
const root = fileURLToPath(new URL("../", import.meta.url));
const arguments_ = process.argv.slice(2);
const selected = arguments_.filter((arg) => arg.startsWith("--map=")).map((arg) => arg.slice(6));
const urls = arguments_.filter((arg) => arg.startsWith("--url="));
assert.ok(urls.length <= 1, "Provide at most one --url");
assert.ok(
  arguments_.every((arg) => arg.startsWith("--map=") || arg.startsWith("--url=")),
  "Supported options: --map=NAME (repeatable), --url=URL",
);
const url = urls[0]?.slice(6) ?? "http://127.0.0.1:5181";
const suffix = ".rhlos-map.json";
const available = (await fs.readdir(path.join(root, "library/scenes")))
  .filter((name) => name.endsWith(suffix))
  .map((name) => name.slice(0, -suffix.length))
  .sort();
for (const map of selected) assert.ok(available.includes(map), `Unknown saved map: ${map}`);
const maps = selected.length ? [...new Set(selected)] : available;
assert.ok(maps.length > 0, "No saved maps found");
const output = await fs.mkdtemp(path.join(root, "work/needs-review/map-compile/browser-exports-"));
console.log(output);
const results = [];
const hash = async (file) =>
  createHash("sha256")
    .update(await fs.readFile(file))
    .digest("hex");
const report = async () => {
  const manifest = {
    scope: "map-only-browser-packaging-and-rebake-not-gameplay-or-visual-parity",
    url,
    maps,
    complete: results.length === maps.length && results.every((result) => result.passed),
    results,
  };
  const temporary = path.join(output, "diagnostics.json.tmp");
  await fs.writeFile(temporary, JSON.stringify(manifest, null, 2) + "\n");
  await fs.rename(temporary, path.join(output, "diagnostics.json"));
};
await report();
for (const map of maps) {
  const started = performance.now();
  const scene = path.join(root, "library/scenes", map + suffix);
  const sceneSha256 = await hash(scene);
  const zip = path.join(output, map + ".zip");
  const log = await fs.open(path.join(output, map + ".log"), "w");
  let exitCode;
  let failure;
  try {
    exitCode = await new Promise((resolve, reject) => {
      const child = spawn(process.execPath, ["app/tests/run-lifecycle.mjs", url], {
        cwd: root,
        env: {
          ...process.env,
          TEST_PAGE: "map-bake-library.html",
          TEST_QUERY: `?${new URLSearchParams({ map, roundtrip: "1", "map-only": "1" })}`,
          TEST_TIMEOUT: process.env.TEST_TIMEOUT ?? "1800000",
          TEST_EVALUATE_TIMEOUT: process.env.TEST_EVALUATE_TIMEOUT ?? "60000",
          TEST_PROGRESS: "1",
          TEST_BAKE_ZIP: zip,
        },
        stdio: ["ignore", log.fd, log.fd],
      });
      child.once("error", reject);
      child.once("exit", (code, signal) => {
        if (signal) reject(new Error(`Browser runner terminated by ${signal}`));
        else resolve(code);
      });
    });
    assert.equal(exitCode, 0, "Browser round trip failed; see map log");
    assert.equal(await hash(scene), sceneSha256, "Saved scene changed during browser verification");
    assert.ok((await fs.stat(zip)).size > 0, "Browser did not save its ZIP");
  } catch (error) {
    failure = String(error);
  } finally {
    await log.close();
  }
  results.push({
    map,
    sceneSha256,
    passed: failure === undefined,
    exitCode,
    elapsedMs: Math.round(performance.now() - started),
    ...(failure ? { error: failure } : { zip: map + ".zip", zipSha256: await hash(zip) }),
    log: map + ".log",
  });
  await report();
  console.log(`${map}: ${failure ?? "PASS"} (${results.at(-1).elapsedMs} ms)`);
}
if (results.some((result) => !result.passed)) process.exitCode = 1;
