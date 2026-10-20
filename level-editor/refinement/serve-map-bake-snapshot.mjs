// Freeze test code before long browser bakes so development reloads cannot restart them.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath, pathToFileURL } from "node:url";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";

const root = fileURLToPath(new URL("../", import.meta.url));
const app = path.join(root, "app");
const require = createRequire(path.join(app, "package.json"));
const { build, preview } = await import(pathToFileURL(require.resolve("vite")).href);
const port = Number(process.argv[2] ?? 5184);
assert.ok(Number.isInteger(port) && port >= 1024 && port <= 65535);
const output = await fs.mkdtemp(path.join(root, "work/needs-review/map-compile/browser-build-"));
const outDir = path.join(output, "dist");
const configFile = path.join(app, "vite.config.ts");
const git = (...args) => execFileSync("git", args, { cwd: root, encoding: "utf8" });
const head = git("rev-parse", "HEAD").trim();
const diff = git("diff", "HEAD", "--", "app", "shared", "pipeline");
await fs.writeFile(path.join(output, "source.patch"), diff);
await build({
  root: app,
  configFile,
  build: {
    outDir,
    rollupOptions: { input: path.join(app, "tests/map-bake-library.html") },
  },
});
assert.equal(git("rev-parse", "HEAD").trim(), head, "HEAD changed while building");
assert.equal(
  git("diff", "HEAD", "--", "app", "shared", "pipeline"),
  diff,
  "Editor sources changed while building",
);
const files = [];
async function inventory(directory) {
  for (const entry of await fs.readdir(directory, { withFileTypes: true })) {
    const file = path.join(directory, entry.name);
    if (entry.isDirectory()) await inventory(file);
    else {
      const bytes = await fs.readFile(file);
      files.push({
        file: path.relative(outDir, file),
        bytes: bytes.length,
        sha256: createHash("sha256").update(bytes).digest("hex"),
      });
    }
  }
}
await inventory(outDir);
await fs.writeFile(
  path.join(output, "snapshot.json"),
  JSON.stringify(
    {
      scope: "frozen-browser-test-code-with-live-library",
      head,
      sourcePatchSha256: createHash("sha256").update(diff).digest("hex"),
      files,
    },
    null,
    2,
  ),
);
const server = await preview({
  root: app,
  configFile,
  build: { outDir },
  preview: { host: "127.0.0.1", port, strictPort: true },
});
console.log(`Snapshot ${output}`);
console.log(`Browser tests: http://127.0.0.1:${port}`);
for (const signal of ["SIGINT", "SIGTERM"])
  process.once(signal, () => server.httpServer.close(() => process.exit(0)));
