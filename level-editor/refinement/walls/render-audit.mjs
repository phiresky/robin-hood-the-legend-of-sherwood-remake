import { spawn } from "node:child_process";
import { mkdtemp, readFile, writeFile, mkdir, rm, access } from "node:fs/promises";
import { chromeEndpoint, socketOpen, evaluate } from "../../app/tests/cdp.mjs";
const output = "work/needs-review/wall-presets";
const segments = process.argv.includes("--segments");
const folder = segments ? "segments" : "candidates";
const url =
  "http://127.0.0.1:5181/tests/wall-audit.html" +
  (segments
    ? "?splines&library=" +
      encodeURIComponent("/@fs/" + process.cwd() + "/work/needs-review/wall-presets/staging/")
    : "");
await mkdir(output + "/" + folder, { recursive: true });
const wanted = process.argv
  .find((arg) => arg.startsWith("--ids="))
  ?.slice(6)
  .split(",");
const rows = JSON.parse(
  await readFile(output + "/" + (wanted && !segments ? "inventory" : folder) + ".json", "utf8"),
).filter((row) => !wanted || wanted.includes(row.id));
const profile = await mkdtemp("/home/phire/.cache/wall-audit-");
const chrome = spawn(
  "chromium",
  [
    "--headless",
    "--disable-background-networking",
    "--no-sandbox",
    "--disable-dev-shm-usage",
    "--enable-unsafe-swiftshader",
    "--use-angle=swiftshader",
    "--remote-debugging-port=0",
    `--user-data-dir=${profile}`,
    url,
  ],
  { stdio: ["ignore", "ignore", "pipe"] },
);
const childClosed = new Promise((resolve) => chrome.once("close", resolve));
let socket;
try {
  const endpoint = new URL(await chromeEndpoint(chrome, { timeoutMs: 60000 }));
  const pages = await (await fetch(`http://${endpoint.host}/json/list`)).json();
  socket = new WebSocket(pages.find((p) => p.type === "page").webSocketDebuggerUrl);
  await socketOpen(socket);
  let n = 0;
  let failures = 0;
  for (let i = 0; i < 100; i++) {
    const status = await evaluate(socket, ++n, 'document.querySelector("#result")?.textContent');
    if (status === "READY") break;
    if (status?.startsWith("FAIL")) throw Error(status);
    await new Promise((r) => setTimeout(r, 200));
  }
  for (const [i, row] of rows.entries()) {
    const target = `${output}/${folder}/${row.id}.webp`;
    if (!process.argv.includes("--force")) {
      try {
        await access(target);
        continue;
      } catch {}
    }
    try {
      await evaluate(
        socket,
        ++n,
        `window.__auditResult = null; window.auditWallAsset(${JSON.stringify(row.id)}, ${segments ? JSON.stringify(row) : "undefined"}).then(value => { window.__auditResult = value; }).catch(error => { window.__auditResult = {error: String(error)}; }); undefined`,
      );
      let data;
      for (let wait = 0; wait < 600; wait++) {
        data = await evaluate(socket, ++n, "window.__auditResult", { timeoutMs: 60000 });
        if (data) break;
        await new Promise((r) => setTimeout(r, 100));
      }
      if (!data || data.error) throw Error(data?.error ?? "Audit timeout");
      await writeFile(target, Buffer.from(data.image.split(",")[1], "base64"));
      await writeFile(
        `${output}/${folder}/${row.id}.json`,
        JSON.stringify({ ...row, ...data, image: undefined }, null, 2),
      );
      await rm(`${output}/${folder}/${row.id}.error.txt`, { force: true });
      console.log(`${i + 1}/${rows.length} ${row.id}`);
    } catch (error) {
      failures++;
      console.log(`FAILED ${row.id}: ${error}`);
      await writeFile(`${output}/${folder}/${row.id}.error.txt`, String(error));
    }
  }
  if (failures) throw Error(`${failures} visual audit cases failed`);
} finally {
  socket?.close();
  chrome.kill("SIGTERM");
  await childClosed;
  await rm(profile, { recursive: true, force: true });
}
