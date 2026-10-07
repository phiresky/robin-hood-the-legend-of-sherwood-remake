// Diagnostic point-mask compositing with real asset/sprite pixels, not native gameplay certification.
import * as THREE from "three";
import { GLTFLoader } from "three/addons/loaders/GLTFLoader.js";
import { partMatrix } from "../../shared/src/level3d.ts";
import { gameToScene } from "../../shared/src/scene.ts";
import { sceneToGame, applyAffineMatrix } from "../../shared/src/geometry.ts";
import { rasterizeMaskGeometry } from "../../shared/src/compile-mask-geometry.ts";
import { decodeRecoveryMask } from "../../pipeline/src/recover-mask-bitmap.ts";
import { TextureDisplay } from "../src/texture-display.ts";
import { decodeSpritePixels } from "../src/entity-projection.ts";
import { maskReviewMesh } from "./mask-review-mesh.mjs";

const result = document.querySelector("#result");
const json = async (url) => {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`Missing review input: ${url}`);
  return response.json();
};
const image = async (url) => createImageBitmap(await (await fetch(url)).blob());
try {
  const stage = new URLSearchParams(location.search).get("stage");
  if (!stage) throw new Error("Provide a staged fern export URL");
  const edits = await json(`${stage}/edits.json`);
  const rotations = [0, 90, 180, 270];
  const manifest = await json("/library/game-data/Data/Characters/RobinTown.rhs.d/manifest.json");
  const profile = manifest.profiles.find((profile) => profile.name === "Robin des bois");
  const frame = profile.rows.find((row) => row.action_id === 3 && row.direction === 8).frames[0];
  const atlas = await image(`/library/game-data/Data/Characters/RobinTown.rhs.d/${manifest.atlas}`);
  const sprite = document.createElement("canvas");
  sprite.width = frame.rect[2];
  sprite.height = frame.rect[3];
  const spriteContext = sprite.getContext("2d");
  spriteContext.drawImage(atlas, ...frame.rect, 0, 0, sprite.width, sprite.height);
  const rgba = spriteContext.getImageData(0, 0, sprite.width, sprite.height);
  decodeSpritePixels(rgba.data, manifest.pixel_format !== "rgba");
  spriteContext.putImageData(rgba, 0, 0);
  const staged = new Map();
  let halfWidth = 90;
  let halfHeight = 75;
  for (const { asset: id } of edits)
    for (const rotation of rotations) {
      const file = `${id}-0-${rotation}.level.json`;
      const descriptor = await json(`${stage}/${file}`);
      staged.set(file, descriptor);
      for (const mask of descriptor.asset_geometry.masks) {
        const [x, y] = mask.box_top_left;
        halfWidth = Math.max(halfWidth, Math.abs(x - 500), Math.abs(x + mask.box_size[0] - 500));
        halfHeight = Math.max(halfHeight, Math.abs(y - 500), Math.abs(y + mask.box_size[1] - 500));
      }
    }
  const width = Math.ceil(halfWidth + 16) * 2,
    height = Math.ceil(halfHeight + 16) * 2;
  const renderer = new THREE.WebGLRenderer({
    alpha: true,
    antialias: false,
    preserveDrawingBuffer: true,
  });
  renderer.setPixelRatio(1);
  renderer.setSize(width, height);
  renderer.outputColorSpace = THREE.SRGBColorSpace;
  const sheet = document.createElement("canvas");
  sheet.width = width * 4 * 2;
  sheet.height = (height + 20) * edits.length * rotations.length * 2;
  const context = sheet.getContext("2d");
  context.scale(2, 2);
  context.imageSmoothingEnabled = false;
  const camera = new THREE.OrthographicCamera(
    -width / 2,
    width / 2,
    height / 2,
    -height / 2,
    1,
    4000,
  );
  const conversion = new THREE.Matrix4().makeRotationX(Math.PI / 2);
  const loader = new GLTFLoader();
  const above = (line, x, y) => {
    if (x < line[0][0] || x > line.at(-1)[0]) return false;
    for (let i = 1; i < line.length; i++) {
      const a = line[i - 1],
        b = line[i];
      if (b[0] < x || a[0] === b[0]) continue;
      return a[1] + ((b[1] - a[1]) * (x - a[0])) / (b[0] - a[0]) > y;
    }
    return false;
  };
  const cases = [];
  let row = 0;
  for (const { asset: id } of edits)
    for (const rotation of rotations) {
      result.textContent = `RUNNING ${id} ${rotation}`;
      const file = `${id}-0-${rotation}.level.json`;
      const descriptor = staged.get(file);
      const document3d = await json(`${stage}/${file}.scene.json`);
      const reference = document3d.assetSources[0];
      const bytes = await (await fetch(`/library/${reference.model}`)).arrayBuffer();
      const digest = [...new Uint8Array(await crypto.subtle.digest("SHA-256", bytes))]
        .map((v) => v.toString(16).padStart(2, "0"))
        .join("");
      if (digest !== reference.model_sha256) throw new Error(`Review model changed: ${id}`);
      const gltf = await loader.parseAsync(
        bytes,
        `/library/${reference.model.slice(0, reference.model.lastIndexOf("/") + 1)}`,
      );
      const wrapper = new THREE.Group();
      wrapper.matrixAutoUpdate = false;
      wrapper.matrix
        .copy(conversion)
        .invert()
        .multiply(
          new THREE.Matrix4().fromArray(
            partMatrix(document3d.camera, document3d, document3d.objects[0]),
          ),
        )
        .multiply(conversion);
      wrapper.add(gltf.scene);
      const display = new TextureDisplay();
      display.smooth = false;
      display.apply(wrapper);
      const scene = new THREE.Scene();
      scene.add(wrapper, new THREE.AmbientLight(0xffffff, 2));
      const target = new THREE.Vector3(...gameToScene(document3d.camera, 500, 500, 0)).applyMatrix4(
        conversion.clone().invert(),
      );
      const angle = THREE.MathUtils.degToRad(35);
      camera.position
        .copy(target)
        .add(new THREE.Vector3(0, Math.sin(angle), Math.cos(angle)).multiplyScalar(2000));
      camera.lookAt(target);
      renderer.render(scene, camera);
      const plant = document.createElement("canvas");
      plant.width = width;
      plant.height = height;
      plant.getContext("2d").drawImage(renderer.domElement, 0, 0);
      const masks = descriptor.asset_geometry.masks;
      const coverage = new Set();
      for (const mask of masks)
        for (const [i, pixel] of decodeRecoveryMask(mask).entries()) {
          if (!pixel) continue;
          const x = mask.box_top_left[0] + (i % mask.box_size[0]) - 500 + width / 2;
          const y = mask.box_top_left[1] + Math.floor(i / mask.box_size[0]) - 500 + height / 2;
          if (x >= 0 && y >= 0 && x < width && y < height) coverage.add(y * width + x);
        }
      const rendered = plant.getContext("2d").getImageData(0, 0, width, height);
      // Render authored coverage, including compact alpha, through the GPU path.
      // This separates extraction errors from CPU/GPU rasterization differences.
      const authored = edits.find((entry) => entry.asset === id).gameplay.masks[0];
      const authoredMesh = maskReviewMesh(authored, (point) => {
        const [x, y, z] = gameToScene(document3d.camera, ...point);
        return [x, z, -y];
      });
      const clippedMesh = authoredMesh.root;
      clippedMesh.matrixAutoUpdate = false;
      clippedMesh.matrix.copy(wrapper.matrix);
      const clippedScene = new THREE.Scene();
      clippedScene.add(clippedMesh);
      renderer.render(clippedScene, camera);
      const clippedCanvas = document.createElement("canvas");
      clippedCanvas.width = width;
      clippedCanvas.height = height;
      const clippedContext = clippedCanvas.getContext("2d");
      clippedContext.drawImage(renderer.domElement, 0, 0);
      const clippedPixels = clippedContext.getImageData(0, 0, width, height).data;
      let clippedGpuVsMask = 0;
      let clippedGpuVsTexture = 0;
      for (let i = 0; i < width * height; i++) {
        const covered = Boolean(clippedPixels[i * 4 + 3]);
        if (covered !== coverage.has(i)) clippedGpuVsMask++;
        if (covered !== Boolean(rendered.data[i * 4 + 3])) clippedGpuVsTexture++;
      }
      authoredMesh.dispose();
      const matrix = partMatrix(document3d.camera, document3d, document3d.objects[0]);
      const projected = authored.triangles.map((triangle) =>
        triangle.map((point) => {
          const [x, y, z] = sceneToGame(
            document3d.camera,
            applyAffineMatrix(matrix, gameToScene(document3d.camera, ...point)),
          );
          return [x, y - z, 0];
        }),
      );
      const subpixelComparisons = [];
      for (const precision of [0, 16, 256, 65536]) {
        const triangles = precision
          ? projected.map((triangle) =>
              triangle.map((point) => point.map((n) => Math.round(n * precision) / precision)),
            )
          : projected;
        const sampled = new Set();
        for (const mask of rasterizeMaskGeometry(
          triangles,
          masks[0],
          authored.cullBackfaces,
          authored.alphaCoverage,
        ))
          for (const [i, pixel] of decodeRecoveryMask(mask).entries()) {
            if (!pixel) continue;
            const x = mask.box_top_left[0] + (i % mask.box_size[0]) - 500 + width / 2;
            const y = mask.box_top_left[1] + Math.floor(i / mask.box_size[0]) - 500 + height / 2;
            if (x >= 0 && y >= 0 && x < width && y < height) sampled.add(y * width + x);
          }
        let gpuDifferences = 0;
        for (let i = 0; i < width * height; i++)
          if (sampled.has(i) !== Boolean(clippedPixels[i * 4 + 3])) gpuDifferences++;
        subpixelComparisons.push({ precision, gpuDifferences });
      }
      let renderedLeafWithoutMask = 0;
      let mismatchesTouchCoverage = true;
      for (let i = 0; i < width * height; i++)
        if (Boolean(rendered.data[i * 4 + 3]) !== coverage.has(i)) {
          if (rendered.data[i * 4 + 3]) renderedLeafWithoutMask++;
          const x = i % width,
            y = Math.floor(i / width);
          let adjacent = false;
          for (const dx of [-1, 0, 1])
            for (const dy of [-1, 0, 1]) {
              if (x + dx < 0 || y + dy < 0 || x + dx >= width || y + dy >= height) continue;
              const other = (y + dy) * width + x + dx;
              adjacent ||= coverage.has(i)
                ? Boolean(rendered.data[other * 4 + 3])
                : coverage.has(other);
            }
          mismatchesTouchCoverage &&= adjacent;
        }
      for (const [column, dy] of [-15, 0, 15, 30].entries()) {
        const feet = [500.25, 500.75 + dy];
        const panel = document.createElement("canvas");
        panel.width = width;
        panel.height = height;
        const ctx = panel.getContext("2d");
        ctx.fillStyle = "#4d5542";
        ctx.fillRect(0, 0, width, height);
        ctx.drawImage(plant, 0, 0);
        const background = ctx.getImageData(0, 0, width, height);
        ctx.drawImage(
          sprite,
          feet[0] - 500 + width / 2 + frame.offset_x - profile.center_x,
          feet[1] - 500 + height / 2 - profile.center_y + frame.offset_y,
        );
        const combined = ctx.getImageData(0, 0, width, height);
        const underlying = plant.getContext("2d").getImageData(0, 0, width, height);
        let applied = false;
        let maskWithoutRenderedLeaf = 0;
        for (const mask of masks)
          if (above(mask.character_polyline, ...feet)) {
            applied = true;
            for (const [index, pixel] of decodeRecoveryMask(mask).entries()) {
              if (!pixel) continue;
              const x = mask.box_top_left[0] + (index % mask.box_size[0]);
              const y = mask.box_top_left[1] + Math.floor(index / mask.box_size[0]);
              const px = x - 500 + width / 2,
                py = y - 500 + height / 2;
              if (px < 0 || py < 0 || px >= width || py >= height) continue;
              const offset = (py * width + px) * 4;
              if (!underlying.data[offset + 3]) maskWithoutRenderedLeaf++;
              for (let c = 0; c < 4; c++) combined.data[offset + c] = background.data[offset + c];
            }
          }
        ctx.putImageData(combined, 0, 0);
        const x = column * width,
          y = row * (height + 20);
        context.drawImage(panel, x, y);
        context.fillStyle = "#eee";
        context.font = "9px sans-serif";
        context.fillText(
          `${id.slice(-7)} ${rotation}deg dy=${dy} mask=${applied}`,
          x + 4,
          y + height + 12,
        );
        cases.push({
          id,
          rotation,
          dy,
          applied,
          maskWithoutRenderedLeaf,
          renderedLeafWithoutMask,
          mismatchesTouchCoverage,
          clippedGpuVsMask,
          clippedGpuVsTexture,
          subpixelComparisons,
        });
      }
      row++;
    }
  Object.assign(window, {
    __migrationImages: { after: sheet.toDataURL("image/png") },
    __fernReview: cases,
  });
  document.body.append(sheet);
  renderer.dispose();
  atlas.close();
  const status = cases.every(
    (entry) =>
      entry.mismatchesTouchCoverage &&
      (entry.rotation !== 0 || entry.maskWithoutRenderedLeaf === 0),
  )
    ? "PASS"
    : "FAIL";
  result.textContent = `${status} ${cases.length} point-mask compositing diagnostics; discrepancies beyond a one-pixel edge require review: ${JSON.stringify(cases)}`;
} catch (error) {
  result.textContent = `FAIL ${error.stack ?? error}`;
}
