import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import { PopulationView } from "./population-view.ts";
import type { Population, PopulationSpriteCatalog } from "@rle/shared";
const camera = { kind: "oblique-orthographic" as const, elevation_deg: 35 };
test("missing legacy population files warn without preventing map loading", async () => {
  const saved = population();
  saved.spriteCatalog = "population/wychford/sprites.json";
  const before = structuredClone(saved);
  const root = {
    async getDirectoryHandle() {
      throw new DOMException(
        "Missing library file: population/wychford/sprites.json",
        "NotFoundError",
      );
    },
  } as unknown as FileSystemDirectoryHandle;
  const view = await PopulationView.load(root, saved, camera);
  assert.equal(view.count, 0);
  assert.match(view.warnings.join("\n"), /Legacy population preview unavailable.*wychford/);
  assert.deepEqual(saved, before);
  view.dispose();
});
const frame = {
  rect: [0, 0, 16, 32] as [number, number, number, number],
  offset: [-8, 32] as [number, number],
  duration: 0.12,
};
const sprite = {
  image: "guard.png",
  width: 16,
  height: 32,
  kind: "character" as const,
  idle: { "-1": [frame] },
  walk: { "-1": [frame] },
};
function population(): Population {
  return {
    version: 1,
    spriteCatalog: "sprites.json",
    actors: [
      {
        id: "one",
        name: "First guard",
        role: "soldier",
        sprite: "guard",
        position: [0, 0, 0],
        direction: 4,
        duty: "Patrol",
        route: "route",
        routeOffset: -10,
      },
      {
        id: "two",
        name: "Second guard",
        role: "soldier",
        sprite: "guard",
        position: [0, 0, 0],
        direction: 4,
        duty: "Patrol",
        route: "route",
        routeOffset: 10,
      },
    ],
    items: [],
    routes: [
      {
        id: "route",
        name: "Patrol",
        mode: "ping-pong",
        speed: 10,
        points: [
          { position: [0, 0, 0], wait: 2 },
          { position: [100, 0, 0], wait: 2 },
        ],
      },
    ],
  };
}
function directory(catalog: PopulationSpriteCatalog) {
  return {
    async getFileHandle(name: string) {
      return {
        getFile: async () =>
          new File([name === "sprites.json" ? JSON.stringify(catalog) : "png"], name),
      };
    },
  } as unknown as FileSystemDirectoryHandle;
}
test("paired actors stay separated at stops and on reversal; pausing freezes their poses", async (t) => {
  let now = 0;
  t.mock.method(performance, "now", () => now);
  let disposed = 0;
  const texture = new THREE.Texture();
  texture.addEventListener("dispose", () => disposed++);
  t.mock.method(THREE.TextureLoader.prototype, "loadAsync", async () => texture);
  const view = await PopulationView.load(
    directory({ version: 1, sprites: { guard: sprite } }),
    population(),
    camera,
  );
  const eye = new THREE.OrthographicCamera();
  eye.position.set(0, 100, 100);
  eye.lookAt(0, 0, 0);
  const actors = view.root.children.filter((c) => c.name.includes("guard"));
  for (now of [0, 7000, 13000, 19000]) {
    view.update(eye);
    assert.equal(actors[0].position.distanceTo(actors[1].position), 20);
  }
  view.setPlaying(false);
  const before = actors.map((a) => a.position.clone());
  now = 30000;
  view.update(eye);
  assert.deepEqual(
    actors.map((a) => a.position.toArray()),
    before.map((p) => p.toArray()),
  );
  view.setRoutesVisible(true);
  assert.equal(view.routesRoot.visible, true);
  view.dispose();
  assert.equal(disposed, 1);
  assert.equal(view.root.children.length, 0);
});
test("parallel sprite failure waits for late textures and disposes their resources", async (t) => {
  const data = population();
  data.actors[1].sprite = "missing";
  let disposed = 0;
  const texture = new THREE.Texture();
  texture.addEventListener("dispose", () => disposed++);
  t.mock.method(THREE.TextureLoader.prototype, "loadAsync", async () => {
    await new Promise((resolve) => setTimeout(resolve, 10));
    return texture;
  });
  await assert.rejects(
    PopulationView.load(directory({ version: 1, sprites: { guard: sprite } }), data, camera),
    /Missing population sprite/,
  );
  assert.equal(disposed, 1);
});
