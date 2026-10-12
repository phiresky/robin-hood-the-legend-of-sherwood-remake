/** Run Vite on 5191 and serve test_foliage_export.py's GLB at FIXTURE_URL. */
import { spawn } from "node:child_process";
import { mkdtemp, rm, mkdir, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { chromeEndpoint, socketOpen, evaluate } from "./cdp.mjs";
const output = resolve(
  process.env.FOLIAGE_OUTPUT ?? "work/needs-review/leicester-refinement/editor-audit/foliage/browser",
);
await mkdir(output, { recursive: true });
const profile = await mkdtemp("/tmp/foliage-check-");
const url =
  (process.env.EDITOR_URL ?? "http://127.0.0.1:5191") +
  "/tests/foliage.html?fixture=" +
  encodeURIComponent(process.env.FIXTURE_URL ?? "/foliage-private-test/fixture.glb");
const chrome = spawn(
  "/usr/lib/chromium/chromium",
  [
    "--headless",
    "--window-size=900,400",
    "--no-sandbox",
    "--disable-dev-shm-usage",
    "--enable-unsafe-swiftshader",
    "--use-angle=swiftshader",
    "--remote-debugging-port=0",
    "--user-data-dir=" + profile,
    url,
  ],
  { stdio: ["ignore", "ignore", "pipe"], env: { ...process.env, TMPDIR: "/tmp" } },
);
const closed = new Promise((resolve) => chrome.on("close", resolve));
let ws,
  id = 0;
try {
  const endpoint = new URL(await chromeEndpoint(chrome));
  const pages = await (await fetch("http://" + endpoint.host + "/json/list")).json();
  ws = new WebSocket(pages.find((page) => page.type === "page").webSocketDebuggerUrl);
  await socketOpen(ws);
  let result;
  for (let i = 0; i < 150; i++) {
    result = await evaluate(ws, ++id, "window.__foliageCheck");
    if (result) break;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  const screenshot = await new Promise((resolve, reject) => {
    const request = ++id;
    const handler = (event) => {
      const message = JSON.parse(event.data);
      if (message.id === request) {
        ws.removeEventListener("message", handler);
        if (message.error) reject(message.error);
        else resolve(message.result.data);
      }
    };
    ws.addEventListener("message", handler);
    ws.send(
      JSON.stringify({ id: request, method: "Page.captureScreenshot", params: { format: "png" } }),
    );
  });
  await writeFile(output + "/views.png", Buffer.from(screenshot, "base64"));
  await writeFile(
    output + "/result.json",
    JSON.stringify(result ?? { status: "FAIL", error: "Timeout" }, null, 2),
  );
  console.log(JSON.stringify(result));
  if (result?.status !== "PASS") throw Error("Foliage browser regression failed");
} finally {
  ws?.close();
  chrome.kill("SIGTERM");
  await closed;
  await rm(profile, { recursive: true, force: true });
}
