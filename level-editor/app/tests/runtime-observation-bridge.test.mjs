import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, writeFile, rm, symlink } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { createHash } from "node:crypto";
import { createObservationBridge } from "../runtime-observation-bridge.mjs";
const encode = (v) => JSON.stringify(v),
  sha = (v) => createHash("sha256").update(v).digest("hex");
async function fixture(t) {
  const root = await mkdtemp(path.join(tmpdir(), "rle-observation-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const save = (n, v) => writeFile(path.join(root, n), encode(v));
  const session_id = "a".repeat(32),
    engine = encode({ control: { frame_counter: 0 } });
  await writeFile(path.join(root, "engine.json"), engine);
  const manifest = {
    kind: "live-state-observations",
    session_id,
    first_engine_frame: 0,
    assets: { file: "engine.json", sha256: sha(engine) },
    authority: { receipt: { file: "engine.json", sha256: sha(engine) } },
  };
  await save("session.json", manifest);
  await save("head.json", { session_id, count: 1, closed: false });
  await save("packet-0.json", {
    session_id,
    sequence: 0,
    engine_frame: 0,
    engine: { file: "engine.json", sha256: sha(engine) },
  });
  const handler = await createObservationBridge(root);
  async function get(url, method = "GET", headers = {}) {
    let status, body;
    const response = {
      writeHead(s) {
        status = s;
        return this;
      },
      end(b) {
        body = b;
        return this;
      },
    };
    await handler(
      {
        url: "/runtime-observations/" + url,
        method,
        headers: { host: "127.0.0.1:5180", ...headers },
      },
      response,
      () => {
        throw Error("unexpected next");
      },
    );
    return { status, body };
  }
  return { save, get, manifest, root, session_id };
}
test("bridge serves pinned bytes and pending sequence without game commands", async (t) => {
  const f = await fixture(t);
  assert.equal((await f.get("packet/0")).status, 200);
  assert.equal((await f.get("packet/1")).status, 204);
  assert.equal((await f.get("packet/0", "POST")).status, 405);
  assert.equal((await f.get("../session")).status, 404);
});
test("foreign browser requests are rejected without retiring valid session", async (t) => {
  const f = await fixture(t);
  assert.equal(
    (await f.get("session", "GET", { origin: "https://unrelated.example" })).status,
    403,
  );
  assert.equal((await f.get("session")).status, 200);
});
test("replaced producer session is permanently retired", async (t) => {
  const f = await fixture(t);
  await f.save("session.json", { ...f.manifest, session_id: "b".repeat(32) });
  assert.equal((await f.get("session")).status, 409);
  await f.save("session.json", f.manifest);
  assert.equal((await f.get("session")).status, 409);
});
test("missing committed packet does not become empty pending response", async (t) => {
  const f = await fixture(t);
  await f.save("head.json", { session_id: f.session_id, count: 2, closed: false });
  assert.equal((await f.get("packet/1")).status, 409);
});
test("frame gap rejected before serving payload", async (t) => {
  const f = await fixture(t);
  await f.save("packet-0.json", { session_id: f.session_id, sequence: 0, engine_frame: 5 });
  assert.equal((await f.get("packet/0")).status, 409);
});
test("spool file symlinks cannot escape bound directory", async (t) => {
  const f = await fixture(t);
  await rm(path.join(f.root, "engine.json"));
  await symlink("/etc/hosts", path.join(f.root, "engine.json"));
  assert.equal((await f.get("assets")).status, 409);
});

test("producer failure is surfaced rather than a normal closed stream", async (t) => {
  const f = await fixture(t);
  await f.save("head.json", {
    session_id: f.session_id,
    count: 1,
    closed: true,
    error: "native capture stopped",
  });
  const reply = await f.get("packet/1");
  assert.equal(reply.status, 409);
  assert.match(String(reply.body), /producer failed/);
});
