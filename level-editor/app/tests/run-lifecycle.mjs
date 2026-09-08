import { spawn } from "node:child_process";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { chromeEndpoint, socketOpen, evaluate } from "./cdp.mjs";

// Build/serve the lifecycle fixture separately. Only a fresh temporary profile is used.
const profile = await mkdtemp(join(tmpdir(), "editor-lifecycle-"));
const lifetime = new AbortController();
const chrome = spawn(
  process.env.CHROME ?? "google-chrome",
  [
    "--headless",
    "--no-sandbox",
    "--disable-dev-shm-usage",
    "--disable-background-networking",
    "--enable-unsafe-swiftshader",
    "--use-angle=swiftshader",
    "--remote-debugging-port=0",
    `--user-data-dir=${profile}`,
    `${process.argv[2] ?? "http://localhost:5181"}/tests/lifecycle.html`,
  ],
  { stdio: ["ignore", "ignore", "pipe"] },
);
let closed = false;
const childClosed = new Promise((resolve) =>
  chrome.once("close", () => {
    closed = true;
    resolve();
  }),
);
chrome.on("error", (error) => lifetime.abort(error));
chrome.on("exit", (code, signal) =>
  lifetime.abort(new Error(`Chromium exited (${code ?? signal})`)),
);
const interrupted = () =>
  lifetime.abort(new Error("Lifecycle runner interrupted"));
process.on("SIGTERM", interrupted);
process.on("SIGINT", interrupted);
let socket;
async function waitForChild(milliseconds) {
  let timer;
  try {
    return await Promise.race([
      childClosed.then(() => true),
      new Promise((resolve) => {
        timer = setTimeout(() => resolve(false), milliseconds);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}
try {
  const endpoint = await chromeEndpoint(chrome, {
    signal: lifetime.signal,
    timeoutMs: 15000,
  });
  const address = new URL(endpoint);
  const response = await fetch(`http://${address.host}/json/list`, {
    signal: AbortSignal.any([lifetime.signal, AbortSignal.timeout(5000)]),
  });
  if (!response.ok)
    throw new Error(`CDP page discovery returned HTTP ${response.status}`);
  const pages = await response.json();
  const page = pages.find((page) => page.type === "page");
  if (!page) throw new Error("Chromium exposed no page target");
  socket = new WebSocket(page.webSocketDebuggerUrl);
  await socketOpen(socket, { signal: lifetime.signal, timeoutMs: 5000 });
  let id = 0;
  let outcome;
  const deadline = Date.now() + 60000;
  while (Date.now() < deadline) {
    outcome = await evaluate(
      socket,
      ++id,
      "document.querySelector('#result')?.textContent",
      {
        signal: lifetime.signal,
        timeoutMs: Math.min(5000, deadline - Date.now()),
      },
    );
    if (outcome?.startsWith("PASS") || outcome?.startsWith("FAIL")) break;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  if (!outcome?.startsWith("PASS"))
    throw new Error(
      outcome?.startsWith("FAIL")
        ? outcome
        : "Lifecycle acceptance timed out without a result",
    );
  console.log(outcome);
} catch (error) {
  console.error(error.stack ?? error);
  process.exitCode = 1;
} finally {
  socket?.close();
  if (!closed) chrome.kill("SIGTERM");
  if (!(await waitForChild(3000))) {
    chrome.kill("SIGKILL");
    if (!(await waitForChild(3000))) {
      console.error(
        `Chromium did not exit; retaining its temporary profile ${profile}`,
      );
      process.exitCode = 1;
    }
  }
  // Never remove a live browser's profile or any path not allocated above.
  if (closed)
    await rm(profile, { recursive: true, force: true, maxRetries: 3 });
  process.removeListener("SIGTERM", interrupted);
  process.removeListener("SIGINT", interrupted);
}
