import { spawn } from "node:child_process";
import { mkdtemp, rm, writeFile, mkdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import assert from "node:assert/strict";
import { chromeEndpoint, socketOpen, evaluate } from "./cdp.mjs";
const profile = await mkdtemp(join(tmpdir(), "terrain-controls-"));
const chrome = spawn(
  process.env.CHROME ?? "chromium",
  [
    "--headless",
    "--no-sandbox",
    "--disable-dev-shm-usage",
    "--window-size=1440,1000",
    "--enable-unsafe-swiftshader",
    "--use-angle=swiftshader",
    "--remote-debugging-port=0",
    `--user-data-dir=${profile}`,
    `${process.argv[2] ?? "http://127.0.0.1:5197"}/tests/terrain-workflow.html?controls`,
  ],
  { stdio: ["ignore", "ignore", "pipe"] },
);
let socket,
  id = 0;
const sleep = () => new Promise((r) => setTimeout(r, 100));
function command(method, params = {}) {
  return new Promise((resolve, reject) => {
    const request = ++id;
    const timer = setTimeout(() => {
      socket.removeEventListener("message", listen);
      reject(new Error(`Timeout ${method}`));
    }, 10000);
    const listen = (event) => {
      const result = JSON.parse(String(event.data));
      if (result.id !== request) return;
      clearTimeout(timer);
      socket.removeEventListener("message", listen);
      if (result.error) reject(new Error(JSON.stringify(result.error)));
      else resolve(result.result);
    };
    socket.addEventListener("message", listen);
    socket.send(JSON.stringify({ id: request, method, params }));
  });
}
const evalJS = (source) => evaluate(socket, ++id, source, { timeoutMs: 10000 });
async function mouse(type, p, modifiers = 0, button = "left", clickCount = 1) {
  await command("Input.dispatchMouseEvent", {
    type,
    x: p.x,
    y: p.y,
    button,
    buttons: type === "mouseReleased" ? 0 : button === "right" ? 2 : 1,
    clickCount,
    modifiers,
  });
  await sleep();
}
async function drag(from, to, cancel = false, modifiers = 0, button = "left") {
  await mouse("mouseMoved", from, modifiers, button);
  await mouse("mousePressed", from, modifiers, button);
  await mouse("mouseMoved", to, modifiers, button);
  if (cancel)
    await command("Input.dispatchKeyEvent", {
      type: "keyDown",
      key: "Escape",
      code: "Escape",
      windowsVirtualKeyCode: 27,
    });
  await mouse("mouseReleased", to, modifiers, button);
  await sleep();
}
try {
  const endpoint = new URL(await chromeEndpoint(chrome, { timeoutMs: 15000 }));
  let page;
  for (let i = 0; i < 50 && !page; i++) {
    page = (await (await fetch(`http://${endpoint.host}/json/list`)).json()).find(
      (p) => p.type === "page",
    );
    if (!page) await sleep();
  }
  assert.ok(page);
  socket = new WebSocket(page.webSocketDebuggerUrl);
  await socketOpen(socket);
  let ready;
  for (let i = 0; i < 150; i++) {
    try {
      ready = await evalJS("document.querySelector('#result')?.textContent");
    } catch {}
    if (ready === "READY" || ready?.startsWith("FAIL")) break;
    await sleep();
  }
  assert.equal(ready, "READY");
  let before = await evalJS("terrainTest.state()"),
    vertex = await evalJS("terrainTest.point(9)");
  const nearVertex = { x: vertex.x + 8, y: vertex.y + 3 };
  await mouse("mouseMoved", nearVertex);
  assert.deepEqual(await evalJS("terrainTest.state().hover"), [
    before.document.terrain.vertices[9].id,
  ]);
  await mouse("mousePressed", nearVertex);
  await mouse("mouseReleased", nearVertex);
  assert.deepEqual((await evalJS("terrainTest.state()")).selected, [
    before.document.terrain.vertices[9].id,
  ]);
  await drag(vertex, { x: vertex.x, y: vertex.y - 25 });
  let after = await evalJS("terrainTest.state()");
  assert.equal(after.commits, before.commits + 1);
  assert.notEqual(
    after.document.terrain.vertices[9].position[2],
    before.document.terrain.vertices[9].position[2],
  );
  before = after;
  vertex = await evalJS("terrainTest.point(9)");
  await drag(vertex, { x: vertex.x, y: vertex.y - 20 }, true);
  after = await evalJS("terrainTest.state()");
  assert.equal(after.commits, before.commits);
  assert.deepEqual(after.document, before.document);
  vertex = await evalJS("terrainTest.point(9)");
  await drag(vertex, { x: vertex.x + 15, y: vertex.y + 10 }, false, 1);
  after = await evalJS("terrainTest.state()");
  assert.equal(after.commits, before.commits + 1);
  assert.notDeepEqual(
    after.document.terrain.vertices[9].position.slice(0, 2),
    before.document.terrain.vertices[9].position.slice(0, 2),
  );
  assert.equal(
    after.document.terrain.vertices[9].position[2],
    before.document.terrain.vertices[9].position[2],
  );
  before = after;
  const number = await evalJS(
    `(()=>{const r=document.querySelector('[aria-label="Vertex Z"]').closest('.meta-row').querySelector('.meta-key').getBoundingClientRect();return {x:r.left+10,y:r.top+r.height/2};})()`,
  );
  await drag(number, { x: number.x + 25, y: number.y });
  after = await evalJS("terrainTest.state()");
  assert.equal(after.commits, before.commits + 1);
  assert.equal(
    after.document.terrain.vertices[9].position[2],
    Math.round((before.document.terrain.vertices[9].position[2] + 25) * 100) / 100,
  );
  before = after;
  await drag(number, { x: number.x + 25, y: number.y }, true);
  after = await evalJS("terrainTest.state()");
  assert.equal(after.commits, before.commits);
  assert.deepEqual(after.document, before.document);
  // Shift-click changes only selection, then dragging either selected vertex moves both.
  await evalJS("terrainTest.selectVertex(9)");
  const added = await evalJS("terrainTest.point(10)");
  await mouse("mousePressed", added, 8);
  await mouse("mouseReleased", added, 8);
  after = await evalJS("terrainTest.state()");
  const ids = [9, 10].map((i) => after.document.terrain.vertices[i].id);
  assert.deepEqual(
    [...after.selected].sort((a, b) => a.localeCompare(b)),
    [...ids].sort((a, b) => a.localeCompare(b)),
  );
  assert.equal(after.commits, before.commits);
  vertex = await evalJS("terrainTest.point(9)");
  await mouse("mouseMoved", vertex);
  assert.deepEqual(
    (await evalJS("terrainTest.state().hover")).sort((a, b) => a.localeCompare(b)),
    [...ids].sort((a, b) => a.localeCompare(b)),
  );
  before = after;
  await drag(vertex, { x: vertex.x, y: vertex.y - 18 });
  after = await evalJS("terrainTest.state()");
  const delta =
    after.document.terrain.vertices[9].position[2] -
    before.document.terrain.vertices[9].position[2];
  assert.ok(delta > 0);
  assert.ok(
    Math.abs(
      after.document.terrain.vertices[10].position[2] -
        before.document.terrain.vertices[10].position[2] -
        delta,
    ) < 1e-8,
  );
  assert.equal(after.commits, before.commits + 1);
  assert.deepEqual(after.document.terrain.vertices[11], before.document.terrain.vertices[11]);
  // Flatten changes only the heights of the selected vertices in one edit.
  before = after;
  const averageHeight =
    (before.document.terrain.vertices[9].position[2] +
      before.document.terrain.vertices[10].position[2]) /
    2;
  await evalJS(
    `Array.from(document.querySelectorAll('button')).find(b => b.textContent === 'Flatten').click()`,
  );
  await sleep();
  after = await evalJS("terrainTest.state()");
  assert.equal(after.commits, before.commits + 1);
  for (const index of [9, 10]) {
    assert.equal(after.document.terrain.vertices[index].position[2], averageHeight);
    assert.deepEqual(
      after.document.terrain.vertices[index].position.slice(0, 2),
      before.document.terrain.vertices[index].position.slice(0, 2),
    );
  }
  assert.deepEqual(after.document.terrain.vertices[11], before.document.terrain.vertices[11]);
  // Removing a member is a click, not a geometry edit.
  const removed = await evalJS("terrainTest.point(10)");
  await mouse("mousePressed", removed, 8);
  await mouse("mouseReleased", removed, 8);
  assert.deepEqual((await evalJS("terrainTest.state()")).selected, [ids[0]]);
  // Edge hit: two endpoints, with no cell interior involved.
  const endpoints = await evalJS("[terrainTest.point(11), terrainTest.point(12)]");
  const edge = {
    x: (endpoints[0].x + endpoints[1].x) / 2,
    y: (endpoints[0].y + endpoints[1].y) / 2,
  };
  const edgeIds = [11, 12].map((i) => after.document.terrain.vertices[i].id);
  await mouse("mouseMoved", edge);
  assert.deepEqual(
    (await evalJS("terrainTest.state().hover")).sort((a, b) => a.localeCompare(b)),
    [...edgeIds].sort((a, b) => a.localeCompare(b)),
  );
  before = await evalJS("terrainTest.state()");
  await drag(edge, { x: edge.x, y: edge.y - 16 });
  after = await evalJS("terrainTest.state()");
  assert.deepEqual(
    [...after.selected].sort((a, b) => a.localeCompare(b)),
    [...edgeIds].sort((a, b) => a.localeCompare(b)),
  );
  for (const i of [11, 12])
    assert.ok(
      after.document.terrain.vertices[i].position[2] >
        before.document.terrain.vertices[i].position[2],
    );
  assert.equal(after.commits, before.commits + 1);
  // Picking an edge does not turn its endpoints into a Shift-built group.
  const edgeEndpoint = await evalJS("terrainTest.point(11)");
  await mouse("mouseMoved", edgeEndpoint);
  assert.deepEqual(await evalJS("terrainTest.state().hover"), [edgeIds[0]]);
  before = await evalJS("terrainTest.state()");
  await drag(edgeEndpoint, { x: edgeEndpoint.x, y: edgeEndpoint.y - 16 });
  after = await evalJS("terrainTest.state()");
  assert.deepEqual(after.selected, [edgeIds[0]]);
  assert.ok(
    after.document.terrain.vertices[11].position[2] >
      before.document.terrain.vertices[11].position[2],
  );
  assert.deepEqual(after.document.terrain.vertices[12], before.document.terrain.vertices[12]);
  assert.equal(after.commits, before.commits + 1);
  // Cell center hit: the complete perimeter, not the rendering triangle beneath it.
  const cell = after.document.terrain.cells[10];
  const corners = await evalJS(`(${JSON.stringify(cell.vertices)}).map(i => terrainTest.point(i))`);
  const area = {
    x: corners.reduce((a, p) => a + p.x, 0) / corners.length,
    y: corners.reduce((a, p) => a + p.y, 0) / corners.length,
  };
  const cornerIds = cell.vertices.map((i) => after.document.terrain.vertices[i].id);
  await mouse("mouseMoved", area);
  assert.deepEqual(
    (await evalJS("terrainTest.state().hover")).sort((a, b) => a.localeCompare(b)),
    [...cornerIds].sort((a, b) => a.localeCompare(b)),
  );
  before = after;
  await drag(area, { x: area.x, y: area.y - 16 });
  after = await evalJS("terrainTest.state()");
  assert.deepEqual(
    [...after.selected].sort((a, b) => a.localeCompare(b)),
    [...cornerIds].sort((a, b) => a.localeCompare(b)),
  );
  assert.deepEqual(after.cells, [cell.id]);
  assert.equal(after.commits, before.commits + 1);
  const areaDelta =
    after.document.terrain.vertices[cell.vertices[0]].position[2] -
    before.document.terrain.vertices[cell.vertices[0]].position[2];
  for (const i of cell.vertices)
    assert.ok(
      Math.abs(
        after.document.terrain.vertices[i].position[2] -
          before.document.terrain.vertices[i].position[2] -
          areaDelta,
      ) < 1e-8,
    );
  // Shift marquee uses projected CSS pixels without rotating the camera.
  await evalJS("terrainTest.clearSelection(); terrainTest.top()");
  await new Promise((resolve) => setTimeout(resolve, 800));
  const screenPoints = await evalJS(
    "terrainTest.state().document.terrain.vertices.map((_,i)=>terrainTest.point(i))",
  );
  const boxPoints = [screenPoints[9], screenPoints[10]];
  const start = {
    x: Math.min(...boxPoints.map((p) => p.x)) - 5,
    y: Math.min(...boxPoints.map((p) => p.y)) - 5,
  };
  const end = {
    x: Math.max(...boxPoints.map((p) => p.x)) + 5,
    y: Math.max(...boxPoints.map((p) => p.y)) + 5,
  };
  const expected = screenPoints.flatMap((p, i) =>
    p.x >= start.x && p.x <= end.x && p.y >= start.y && p.y <= end.y
      ? [after.document.terrain.vertices[i].id]
      : [],
  );
  assert.ok(expected.length >= 2);
  before = after;
  await drag(start, end, false, 8);
  after = await evalJS("terrainTest.state()");
  assert.deepEqual(
    [...after.selected].sort((a, b) => a.localeCompare(b)),
    expected.sort((a, b) => a.localeCompare(b)),
  );
  assert.equal(after.commits, before.commits);
  assert.deepEqual(await evalJS("terrainTest.point(9)"), screenPoints[9]);
  const extra = screenPoints[26];
  await drag({ x: extra.x - 4, y: extra.y - 4 }, { x: extra.x + 4, y: extra.y + 4 }, false, 8);
  after = await evalJS("terrainTest.state()");
  assert.deepEqual(
    [...after.selected].sort((a, b) => a.localeCompare(b)),
    [...new Set([...expected, after.document.terrain.vertices[26].id])].sort((a, b) =>
      a.localeCompare(b),
    ),
  );
  before = after;
  await evalJS("terrainTest.selectCell(0)");
  await sleep();
  await evalJS(
    `Array.from(document.querySelectorAll('button')).find(b=>b.textContent==='Subdivide selected cell').click()`,
  );
  await sleep();
  after = await evalJS("terrainTest.state()");
  assert.ok(after.document.terrain.cells.length > before.document.terrain.cells.length);
  assert.equal(after.commits, before.commits + 1);
  before = after;
  const doubleCorners = await evalJS(
    `(${JSON.stringify(cell.vertices)}).map(i => terrainTest.point(i))`,
  );
  const doublePoint = {
    x: doubleCorners.reduce((sum, p) => sum + p.x, 0) / doubleCorners.length,
    y: doubleCorners.reduce((sum, p) => sum + p.y, 0) / doubleCorners.length,
  };
  await mouse("mousePressed", doublePoint);
  await mouse("mouseReleased", doublePoint);
  await mouse("mousePressed", doublePoint, 0, "left", 2);
  await mouse("mouseReleased", doublePoint, 0, "left", 2);
  after = await evalJS("terrainTest.state()");
  assert.equal(after.commits, before.commits + 1, "Double click creates one subdivision edit");
  assert.ok(!after.document.terrain.cells.some((c) => c.id === cell.id));
  assert.deepEqual(
    after.document.terrain.cells.at(-1),
    before.document.terrain.cells.at(-1),
    "Double click keeps distant cells unchanged",
  );
  before = after;
  await evalJS("terrainTest.selectVertex(9)");
  await sleep();
  const deleted = before.document.terrain.vertices[9];
  await command("Input.dispatchKeyEvent", {
    type: "keyDown",
    key: "Delete",
    code: "Delete",
    windowsVirtualKeyCode: 46,
  });
  await sleep();
  after = await evalJS("terrainTest.state()");
  assert.equal(after.commits, before.commits + 1, "Delete creates one terrain edit");
  assert.equal(after.document.terrain.vertices.length, before.document.terrain.vertices.length - 1);
  assert.ok(!after.document.terrain.vertices.some((vertex) => vertex.id === deleted.id));
  assert.ok(
    Number.isFinite(
      await evalJS(`terrainTest.height(${deleted.position[0]}, ${deleted.position[1]})`),
    ),
    "Deleted interior vertex leaves connected ground",
  );
  for (const vertex of after.document.terrain.vertices)
    assert.deepEqual(
      vertex,
      before.document.terrain.vertices.find((old) => old.id === vertex.id),
    );
  await evalJS("terrainTest.roundtrip()");
  assert.deepEqual((await evalJS("terrainTest.state()")).document.terrain, after.document.terrain);
  const beforeOrbit = await evalJS("terrainTest.point(9)");
  await drag(doublePoint, { x: doublePoint.x + 50, y: doublePoint.y + 20 }, false, 0, "right");
  assert.notDeepEqual(
    await evalJS("terrainTest.point(9)"),
    beforeOrbit,
    "Right drag rotates terrain view",
  );
  assert.equal((await evalJS("terrainTest.state()")).commits, after.commits);
  assert.deepEqual(after.errors, []);
  if (process.env.TEST_ARTIFACT_DIR) {
    await mkdir(process.env.TEST_ARTIFACT_DIR, { recursive: true });
    const image = await command("Page.captureScreenshot");
    await writeFile(
      join(process.env.TEST_ARTIFACT_DIR, "terrain-controls.png"),
      Buffer.from(image.data, "base64"),
    );
  }
  console.log(
    "PASS real pointer vertex elevation, Alt horizontal movement, one commit per drag, Escape cancellation, number scrubbing, Shift selection, hover, edges/cells, Shift-drag marquee, right orbit, flatten, double-click subdivision and Delete with connected ground",
  );
} catch (error) {
  console.error(error);
  if (socket && process.env.TEST_ARTIFACT_DIR) {
    await mkdir(process.env.TEST_ARTIFACT_DIR, { recursive: true });
    const image = await command("Page.captureScreenshot");
    await writeFile(
      join(process.env.TEST_ARTIFACT_DIR, "terrain-controls-failure.png"),
      Buffer.from(image.data, "base64"),
    );
  }
  throw error;
} finally {
  socket?.close();
  const exited = new Promise((r) => chrome.once("close", r));
  chrome.kill("SIGTERM");
  await Promise.race([exited, new Promise((r) => setTimeout(r, 3000))]);
  if (chrome.exitCode === null) chrome.kill("SIGKILL");
  await rm(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
}
