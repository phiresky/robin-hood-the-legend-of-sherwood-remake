import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import { prepareGpuTextures } from "./prepare-gpu-textures.ts";
function fixture() {
  const a = new THREE.Texture(),
    b = a.clone(),
    root = new THREE.Group();
  root.add(
    new THREE.Mesh(
      new THREE.BoxGeometry(),
      new THREE.MeshStandardMaterial({ map: a, emissiveMap: a }),
    ),
  );
  root.add(
    new THREE.Mesh(
      new THREE.BoxGeometry(),
      new THREE.ShaderMaterial({ uniforms: { maps: { value: [a, b] } } }),
    ),
  );
  return { root, a, b };
}
test("upload queue yields between distinct unchanged texture identities", async () => {
  const { root, a, b } = fixture();
  const events: unknown[] = [];
  const versions = [a.version, b.version];
  await prepareGpuTextures(
    [root, root],
    { initTexture: (t) => events.push(t) },
    () => {},
    (done, total) => events.push([done, total]),
    async () => {
      events.push("yield");
    },
  );
  assert.deepEqual(events, [[0, 2], "yield", a, [1, 2], "yield", b, [2, 2], "yield"]);
  assert.deepEqual([a.version, b.version], versions);
});
test("abort in a real queued task prevents remaining uploads without disposing resources", async () => {
  const { root, a } = fixture();
  const abort = new AbortController();
  let uploads = 0,
    disposals = 0;
  a.addEventListener("dispose", () => disposals++);
  await assert.rejects(
    prepareGpuTextures(
      [root],
      {
        initTexture: () => {
          uploads++;
          setTimeout(() => abort.abort(), 0);
        },
      },
      () => abort.signal.throwIfAborted(),
      () => {},
    ),
    { name: "AbortError" },
  );
  assert.equal(uploads, 1);
  assert.equal(disposals, 0);
});
test("superseded renderer at initial yield cannot upload", async () => {
  const { root } = fixture();
  let current = true,
    uploads = 0;
  await assert.rejects(
    prepareGpuTextures(
      [root],
      { initTexture: () => uploads++ },
      () => {
        if (!current) throw new Error("superseded");
      },
      () => {},
      async () => {
        current = false;
      },
    ),
    /superseded/,
  );
  assert.equal(uploads, 0);
});
test("upload failure rejects before later textures", async () => {
  const { root } = fixture();
  let uploads = 0;
  await assert.rejects(
    prepareGpuTextures(
      [root],
      {
        initTexture: () => {
          uploads++;
          throw new Error("upload failed");
        },
      },
      () => {},
      () => {},
      async () => {},
    ),
    /upload failed/,
  );
  assert.equal(uploads, 1);
});
