import fs from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import { fileURLToPath } from "node:url";
import { spawn } from "node:child_process";
import { createRequire } from "node:module";
import { chromeEndpoint, socketOpen } from "../../app/tests/cdp.mjs";

const dir = path.dirname(fileURLToPath(import.meta.url));
const app = path.resolve(dir, "../../app");
const require = createRequire(path.join(app, "package.json"));
const { createServer } = await import(require.resolve("vite"));
const output = path.resolve(dir, "../../work/bc7f-benchmark/worker-pool");
await fs.mkdir(output, { recursive: true });
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function connect(url) {
  const socket = new WebSocket(url);
  await socketOpen(socket);
  let id = 0;
  const pending = new Map();
  socket.onmessage = ({ data }) => {
    const message = JSON.parse(data);
    if (!message.id) return;
    const request = pending.get(message.id);
    if (!request) return;
    pending.delete(message.id);
    clearTimeout(request.timer);
    if (message.error) request.reject(new Error(JSON.stringify(message.error)));
    else request.resolve(message.result);
  };
  return {
    close() {
      socket.close();
    },
    send(method, params = {}) {
      return new Promise((resolve, reject) => {
        const key = ++id;
        const timer = setTimeout(() => {
          pending.delete(key);
          reject(new Error(`CDP timeout: ${method}`));
        }, 30000);
        pending.set(key, { resolve, reject, timer });
        socket.send(JSON.stringify({ id: key, method, params }));
      });
    },
  };
}

async function memory(browser) {
  const { processInfo } = await browser.send("SystemInfo.getProcessInfo");
  const processes = [];
  let gpuResidentKiB = 0;
  for (const { id, type } of processInfo) {
    try {
      const status = await fs.readFile(`/proc/${id}/status`, "utf8");
      const rssKiB = Number(status.match(/^VmRSS:\s+(\d+)/m)?.[1] ?? 0);
      processes.push({ type, pid: id, rssKiB });
      if (type !== "GPU") continue;
      const clients = new Set();
      for (const fd of await fs.readdir(`/proc/${id}/fdinfo`)) {
        const text = await fs.readFile(`/proc/${id}/fdinfo/${fd}`, "utf8").catch(() => "");
        const client = text.match(/^drm-client-id:\s+(\d+)/m)?.[1];
        if (!client || clients.has(client)) continue;
        clients.add(client);
        for (const match of text.matchAll(/^drm-resident-(?:gtt|vram):\s+(\d+)\s+(KiB|MiB|GiB)/gm))
          gpuResidentKiB += Number(match[1]) * { KiB: 1, MiB: 1024, GiB: 1048576 }[match[2]];
      }
    } catch {
      /* A spare renderer can exit between enumeration and /proc reads. */
    }
  }
  return {
    processes,
    largestRendererRssKiB: Math.max(
      0,
      ...processes.filter((p) => p.type === "renderer").map((p) => p.rssKiB),
    ),
    processRssSumKiB: processes.reduce((total, p) => total + p.rssKiB, 0),
    gpuResidentKiB,
  };
}

async function run(map, pool, trial) {
  const workers = Number(pool.split("-")[0]);
  const sizeAware = pool.endsWith("-small");
  // Change only this browser's served encoder limit, without editing application files.
  const server = await createServer({
    root: app,
    configFile: path.join(app, "vite.config.ts"),
    logLevel: "error",
    server: { host: "127.0.0.1", port: 0, strictPort: false, hmr: false, watch: null },
    plugins: [
      {
        name: "benchmark-worker-limit",
        enforce: "pre",
        transform(code, id) {
          if (id.split("?")[0] !== path.join(app, "src/texture-compression.ts")) return;
          const expression = /Math\.min\(\d+, navigator\.hardwareConcurrency \|\| \d+\)/g;
          if ([...code.matchAll(expression)].length !== 1)
            throw new Error("Worker limit source changed");
          if (sizeAware) {
            const original = "const worker = this.idle.pop()!,\n        job = this.queue.shift()!;";
            if (!code.includes(original)) throw new Error("Queue scheduling source changed");
            code = code.replace(
              original,
              `
              // Keep large allocations on one worker for its entire lifetime.
              const small = job => {
                const image = textureImage(job.texture);
                return image.width * image.height <= 2048 * 2048;
              };
              const idleIndex = this.idle.findIndex(worker =>
                worker === this.workers[0] || this.queue.some(small));
              if (idleIndex < 0) break;
              const worker = this.idle.splice(idleIndex, 1)[0];
              let jobIndex = worker === this.workers[0]
                ? this.queue.findIndex(job => !small(job))
                : this.queue.findIndex(small);
              if (jobIndex < 0) jobIndex = 0;
              const job = this.queue.splice(jobIndex, 1)[0];
            `,
            );
          }
          return (
            code.replace(
              expression,
              `Math.min(${workers}, navigator.hardwareConcurrency || ${workers})`,
            ) + `\nglobalThis.__bc7WorkerLimit = ${workers};\n`
          );
        },
      },
    ],
  });
  const profile = await fs.mkdtemp(path.join(os.tmpdir(), "bc7f-pool-"));
  let chrome,
    closed,
    browser,
    page,
    sampling,
    stopped = false;
  const samples = [];
  try {
    await server.listen();
    const origin = `http://127.0.0.1:${server.httpServer.address().port}`;
    chrome = spawn(
      process.env.CHROME ?? "chromium",
      [
        "--headless",
        "--no-sandbox",
        "--disable-dev-shm-usage",
        "--disable-background-networking",
        "--use-gl=angle",
        "--use-angle=gl",
        "--disable-software-rasterizer",
        "--remote-debugging-port=0",
        `--user-data-dir=${profile}`,
        "about:blank",
      ],
      { stdio: ["ignore", "ignore", "pipe"] },
    );
    closed = new Promise((resolve) => chrome.once("close", resolve));
    const endpoint = await chromeEndpoint(chrome, { timeoutMs: 20000 });
    browser = await connect(endpoint);
    const gpu = await browser.send("SystemInfo.getInfo");
    if (/swiftshader|llvmpipe|software rasterizer/i.test(JSON.stringify(gpu.gpu.devices)))
      throw new Error("Hardware GPU required");
    const targets = await (await fetch(`http://${new URL(endpoint).host}/json/list`)).json();
    page = await connect(targets.find((target) => target.type === "page").webSocketDebuggerUrl);
    const evaluate = async (expression) => {
      const result = await page.send("Runtime.evaluate", {
        expression,
        awaitPromise: true,
        returnByValue: true,
      });
      if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails));
      return result.result.value;
    };
    await page.send("Page.navigate", { url: origin });
    let ready = false;
    for (let i = 0; i < 600; i++) {
      ready = await evaluate(
        `location.origin === ${JSON.stringify(origin)} && !![...document.querySelectorAll('[data-map]')].find(b => b.dataset.map.toLowerCase() === ${JSON.stringify(map.toLowerCase())})`,
      ).catch(() => false);
      if (ready) break;
      await pause(100);
    }
    if (!ready) throw new Error("Map chooser unavailable");
    const limit = await evaluate("globalThis.__bc7WorkerLimit");
    if (limit !== workers) throw new Error("Benchmark worker limit was not applied");
    const started = Date.now();
    sampling = (async () => {
      while (!stopped) {
        samples.push({ ms: Date.now() - started, ...(await memory(browser)) });
        await pause(100);
      }
    })();
    await evaluate(`(() => {
      const start = performance.now();
      window.__poolBenchmark = { start, phases: [], elapsed: null, activePeak: 0 };
      const observer = new MutationObserver(() => {
        const b = window.__poolBenchmark;
        const phase = document.querySelector('.map-load-progress')?.textContent;
        if (phase && b.phases.at(-1)?.text !== phase) b.phases.push({ ms: performance.now() - start, text: phase });
        b.activePeak = Math.max(b.activePeak, Number(phase?.match(/encoding textures \\((\\d+) active/)?.[1] ?? 0));
        if (document.querySelector('[aria-label="Close map"]') && !document.querySelector('.map-load-dialog')) {
          b.elapsed = performance.now() - start; observer.disconnect();
        }
      });
      observer.observe(document.body, { subtree: true, childList: true, characterData: true });
      [...document.querySelectorAll('[data-map]')].find(b => b.dataset.map.toLowerCase() === ${JSON.stringify(map.toLowerCase())}).click();
    })()`);
    let result;
    for (let i = 0; i < 1800; i++) {
      result = await evaluate("window.__poolBenchmark");
      if (result.elapsed !== null) break;
      await pause(100);
    }
    if (result.elapsed === null) throw new Error("Map load timed out");
    if (!sizeAware && result.activePeak !== workers)
      throw new Error(`Observed ${result.activePeak} active workers, expected ${workers}`);
    const atLoad = await memory(browser);
    stopped = true;
    await sampling;
    await pause(2000);
    const settled = await memory(browser);
    const heap = await page.send("Runtime.getHeapUsage");
    const peak = Object.fromEntries(
      ["largestRendererRssKiB", "processRssSumKiB", "gpuResidentKiB"].map((key) => [
        key,
        Math.max(atLoad[key], ...samples.map((sample) => sample[key])),
      ]),
    );
    const record = {
      map,
      workers,
      pool,
      trial,
      ...result,
      peak,
      atLoad,
      settled,
      heap,
      gpu: gpu.gpu.devices,
      samples,
    };
    await fs.writeFile(
      path.join(output, `${map.toLowerCase()}-${pool}-${trial}.json`),
      JSON.stringify(record, null, 2),
    );
    console.log(JSON.stringify({ map, pool, workers, trial, elapsed: result.elapsed, peak }));
    return {
      map,
      workers,
      pool,
      trial,
      elapsed: result.elapsed,
      activePeak: result.activePeak,
      peak,
      atLoad,
      settled,
      heap,
      gpu: gpu.gpu.devices,
    };
  } finally {
    stopped = true;
    await sampling?.catch(() => {});
    page?.close();
    browser?.close();
    if (chrome) {
      chrome.kill();
      await closed;
    }
    await server.close();
    await fs.rm(profile, { recursive: true, force: true });
  }
}

const results = [];
const pools = (process.env.BENCH_POOLS ?? "2,4").split(",");
for (
  let trial = Number(process.env.BENCH_START_TRIAL ?? 1);
  trial <= Number(process.env.BENCH_TRIALS ?? 3);
  trial++
)
  for (const map of (process.env.BENCH_MAPS ?? "York,Wychford").split(","))
    for (const pool of trial % 2 ? pools : [...pools].reverse()) {
      results.push(await run(map, pool, trial));
      await fs.writeFile(
        path.join(output, `summary-${pools.join("_")}.json`),
        JSON.stringify(results, null, 2),
      );
    }
