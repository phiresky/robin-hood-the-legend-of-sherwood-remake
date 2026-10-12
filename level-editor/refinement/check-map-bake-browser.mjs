// Run the real browser bake/reopen assertions and preserve the resulting archive.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs/promises";
import path from "node:path";
import { chromeEndpoint, evaluate, socketOpen } from "../app/tests/cdp.mjs";

const [url, directory, filename, timeoutText = "3600000"] = process.argv.slice(2);
assert.ok(url && directory && filename, "Supply bake URL, output directory and ZIP filename");
assert.equal(new URL(url).searchParams.get("roundtrip"), "1", "Require ZIP reopening and rebaking");
assert.equal(path.basename(filename), filename);
const timeout = Number(timeoutText);
assert.ok(Number.isSafeInteger(timeout) && timeout > 0);
const output = path.resolve(directory);
const profile = await fs.mkdtemp(path.join(output, "chrome-"));
const chrome = spawn(
  "/usr/lib/chromium/chromium",
  [
    "--headless",
    "--no-sandbox",
    "--disable-dev-shm-usage",
    "--disable-background-networking",
    "--enable-unsafe-swiftshader",
    "--use-angle=swiftshader",
    "--remote-debugging-port=0",
    "--user-data-dir=" + profile,
    url,
  ],
  { stdio: ["ignore", "ignore", "pipe"] },
);
let exited = false;
const closed = new Promise((resolve) =>
  chrome.on("close", () => {
    exited = true;
    resolve();
  }),
);
let socket;
let request = 0;
try {
  const endpoint = new URL(await chromeEndpoint(chrome, { timeoutMs: 30000 }));
  const pages = await (await fetch(`http://${endpoint.host}/json/list`)).json();
  socket = new WebSocket(pages.find((page) => page.type === "page").webSocketDebuggerUrl);
  await socketOpen(socket);
  const deadline = Date.now() + timeout;
  let result = "";
  let previous = "";
  while (Date.now() < deadline) {
    assert.ok(!exited, "Chromium exited before completing the bake");
    try {
      result =
        (await evaluate(socket, ++request, "document.querySelector('#result')?.textContent", {
          timeoutMs: 45000,
        })) ?? "";
    } catch (error) {
      if (error instanceof Error && error.message === "CDP evaluation timed out") {
        console.log("Browser status pending; bake remains running");
        continue;
      }
      throw error;
    }
    if (result !== previous) console.log(result);
    previous = result;
    assert.ok(!result.startsWith("FAIL"), result);
    if (result.startsWith("PASS")) break;
    await new Promise((resolve) => setTimeout(resolve, 1000));
  }
  assert.ok(result.startsWith("PASS"), `Bake exceeded ${timeout}ms: ${result}`);
  assert.ok(result.includes("reopened gameplay and rendered color/depth match"), result);
  const size = await evaluate(socket, ++request, "window.__bakeZip.length");
  assert.ok(Number.isSafeInteger(size) && size > 0, "Browser omitted ZIP bytes");
  const file = await fs.open(path.join(output, filename), "wx");
  const hash = createHash("sha256");
  try {
    for (let offset = 0; offset < size; offset += 262144) {
      const data = await evaluate(
        socket,
        ++request,
        `btoa(Array.from(window.__bakeZip.subarray(${offset},${offset + 262144}),b=>String.fromCharCode(b)).join(''))`,
        { timeoutMs: 45000 },
      );
      const bytes = Buffer.from(data, "base64");
      hash.update(bytes);
      await file.writeFile(bytes);
    }
    assert.equal((await file.stat()).size, size, "Archive transfer changed its length");
  } finally {
    await file.close();
  }
  const report = {
    scope: "browser-rendered-export-and-reopen-not-in-game-render-proof",
    browserRoundtrip: result,
    bytes: size,
    zipSha256: hash.digest("hex"),
  };
  await fs.writeFile(path.join(output, "browser-roundtrip-evidence.json"), JSON.stringify(report));
  console.log(JSON.stringify(report));
} finally {
  socket?.close();
  chrome.kill("SIGTERM");
  await closed;
  await fs.rm(profile, { recursive: true, force: true });
}
