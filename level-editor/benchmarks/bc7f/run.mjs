import http from "node:http";
import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawn } from "node:child_process";
import { chromeEndpoint, socketOpen } from "../../app/tests/cdp.mjs";
const dir = path.dirname(fileURLToPath(import.meta.url));
const work = path.resolve(dir, "../../work/bc7f-benchmark");
const artifacts = path.join(work, "results-" + (process.env.BENCH_VARIANT ?? "scalar"));
await fs.mkdir(artifacts, { recursive: true });
const rows = [];
const server = http.createServer(async (req, res) => {
  try {
    const url = new URL(req.url, "http://localhost");
    if (req.method === "POST") {
      const chunks = [];
      for await (const chunk of req) chunks.push(chunk);
      const data = Buffer.concat(chunks);
      if (url.pathname === "/row") {
        const row = JSON.parse(data);
        rows.push(row);
        if (!process.env.BENCH_MAP) console.log(JSON.stringify(row));
        else if (rows.length % 25 === 0) console.log("Encoded", rows.length, "textures");
        await fs.writeFile(path.join(artifacts, "rows.json"), JSON.stringify(rows, null, 2));
      } else if (/^\/artifact\/[\w-]+\.png$/.test(url.pathname))
        await fs.writeFile(path.join(artifacts, path.basename(url.pathname)), data);
      else throw new Error("Bad artifact path");
      res.end("ok");
      return;
    }
    let base = dir,
      relative = url.pathname.slice(1) || "index.html";
    if (relative.startsWith("basis/")) {
      base = path.join(work, "basis_universal");
      relative = relative.slice(6);
    }
    if (relative.startsWith("compiled/")) {
      base = work;
      relative = relative.slice(9);
      if (process.env.BENCH_BUILD === "scalar")
        relative = relative.replace(/^bc7f(?=\.)/, "bc7f-scalar");
    }
    if (relative.startsWith("inputs/")) {
      base = work;
      if (process.env.BENCH_MAP)
        relative = relative.replace("inputs/", "inputs-" + process.env.BENCH_MAP + "/");
    }
    const file = path.resolve(base, relative);
    if (!file.startsWith(base + path.sep)) throw new Error("Bad path");
    const data = await fs.readFile(file);
    res.setHeader(
      "Content-Type",
      file.endsWith(".js")
        ? "text/javascript"
        : file.endsWith(".wasm")
          ? "application/wasm"
          : file.endsWith(".html")
            ? "text/html"
            : "application/octet-stream",
    );
    res.end(data);
  } catch (error) {
    res.writeHead(404).end(String(error));
  }
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const profile = await fs.mkdtemp("/tmp/bc7f-chromium-");
const hardware = process.env.BENCH_GPU === "hardware";
const gpuFlags = hardware
  ? ["--use-gl=angle", "--use-angle=gl", "--disable-software-rasterizer"]
  : ["--enable-unsafe-swiftshader", "--use-angle=swiftshader"];
const chrome = spawn(
  "/usr/bin/chromium",
  [
    "--headless",
    "--no-sandbox",
    "--disable-dev-shm-usage",
    ...gpuFlags,
    "--remote-debugging-port=0",
    "--user-data-dir=" + profile,
    `http://127.0.0.1:${server.address().port}/?map=${process.env.BENCH_MAP ?? ""}`,
  ],
  { stdio: ["ignore", "ignore", "pipe"] },
);
const closed = new Promise((resolve) => chrome.once("close", resolve));
const sockets = [];
async function connect(url) {
  const socket = new WebSocket(url);
  sockets.push(socket);
  await socketOpen(socket);
  let id = 0;
  const pending = new Map();
  socket.onmessage = (event) => {
    const v = JSON.parse(event.data);
    if (v.id) {
      const p = pending.get(v.id);
      pending.delete(v.id);
      v.error ? p.reject(v.error) : p.resolve(v.result);
    }
  };
  return (method, params = {}) =>
    new Promise((resolve, reject) => {
      const key = ++id;
      pending.set(key, { resolve, reject });
      socket.send(JSON.stringify({ id: key, method, params }));
    });
}
const samples = [];
let timer;
try {
  const endpoint = await chromeEndpoint(chrome, { timeoutMs: 20000 });
  const browser = await connect(endpoint);
  const targets = await (await fetch(`http://${new URL(endpoint).host}/json/list`)).json();
  const page = await connect(targets.find((t) => t.type === "page").webSocketDebuggerUrl);
  const gpu = await browser("SystemInfo.getInfo");
  if (
    hardware &&
    gpu.gpu.devices.some((d) => /SwiftShader|llvmpipe|Software/i.test(d.deviceString))
  )
    throw new Error("Hardware run selected a software GPU: " + JSON.stringify(gpu.gpu.devices));
  await fs.writeFile(path.join(artifacts, "gpu.json"), JSON.stringify(gpu, null, 2));
  let phase = "init";
  let sampling = false;
  async function sample() {
    if (sampling) return;
    sampling = true;
    try {
      const procs = await browser("SystemInfo.getProcessInfo");
      const rss = [];
      for (const p of procs.processInfo) {
        try {
          const status = await fs.readFile(`/proc/${p.id}/status`, "utf8");
          const drm = [];
          if (p.type === "GPU") {
            const clients = new Set();
            for (const fd of await fs.readdir(`/proc/${p.id}/fdinfo`)) {
              try {
                const info = await fs.readFile(`/proc/${p.id}/fdinfo/${fd}`, "utf8");
                const client = info.match(/^drm-client-id:\s+(\d+)/m)?.[1];
                if (client && !clients.has(client)) {
                  clients.add(client);
                  drm.push(
                    info
                      .split("\n")
                      .filter((line) => /^drm-/.test(line))
                      .join("\n"),
                  );
                }
              } catch {}
            }
          }
          rss.push({
            type: p.type,
            pid: p.id,
            rssKiB: Number(status.match(/^VmRSS:\s+(\d+)/m)?.[1] ?? 0),
            ...(drm.length ? { drm } : {}),
          });
        } catch {}
      }
      samples.push({ time: Date.now(), phase, rss });
    } finally {
      sampling = false;
    }
  }
  timer = setInterval(() => sample().catch(console.error), 100);
  await sample();
  const deadline = Date.now() + 600000;
  while (Date.now() < deadline) {
    const r = await page("Runtime.evaluate", {
      expression:
        "({done:window.done,error:window.error,phase:window.phase,init:window.init,batchMs:window.batchMs})",
      returnByValue: true,
    });
    if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails));
    const v = r.result.value;
    phase = v.phase;
    if (v.done) {
      if (v.error) throw new Error(v.error);
      console.log("Complete", JSON.stringify({ init: v.init, batchMs: v.batchMs }));
      break;
    }
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  if (phase !== "done") throw new Error("Benchmark timed out");
  await page("HeapProfiler.collectGarbage");
  await sample();
  await fs.writeFile(
    path.join(artifacts, "heap.json"),
    JSON.stringify(await page("Runtime.getHeapUsage"), null, 2),
  );
} finally {
  clearInterval(timer);
  await fs.writeFile(path.join(artifacts, "memory.json"), JSON.stringify(samples, null, 2));
  for (const socket of sockets) socket.close();
  chrome.kill();
  await closed;
  server.close();
  await fs.rm(profile, { recursive: true, force: true });
}
