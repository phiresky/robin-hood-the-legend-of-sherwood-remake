import * as THREE from "three";
import { NativePixelCompositor } from "../src/native-pixel-compositor.ts";
import { compositeNativePixels } from "../src/native-state-presentation.ts";
const renderer = new THREE.WebGLRenderer();
renderer.setSize(32, 32);
document.body.append(renderer.domElement);
const compositor = new NativePixelCompositor();
try {
  const width = 17,
    height = 13,
    data = new Uint8Array(width * height * 4);
  for (let i = 0; i < data.length; i++) data[i] = (i * 73 + 19) % 256;
  const background = { width, height, data };
  const results = [];
  for (const pixel_format of ["rgb555", "rgb565"] as const)
    for (const strength_percent of [0, 1, 27, 50, 99, 100])
      for (const [x, y] of [
        [0, 0],
        [-2, 3],
        [12, -1],
      ] as const) {
        const source = new Uint8Array(9 * 7 * 4);
        for (let i = 0; i < source.length; i += 4) {
          source.set(
            i % 12 === 0 ? [248, 0, 248, 255] : i % 12 === 4 ? [43, 219, 51, 127] : [3, 4, 5, 0],
            i,
          );
        }
        const pixels = { width: 9, height: 7, data: source },
          shadow = {
            rgb: [248, 0, 248] as [number, number, number],
            pixel_format,
            strength_percent,
          };
        const expected = { width, height, data: data.slice() };
        compositeNativePixels(expected, pixels, x, y, shadow);
        compositeNativePixels(expected, pixels, x + 1, y + 1, shadow);
        const actual = compositor.compose(renderer, background, [
          { pixels, x, y, shadow },
          { pixels, x: x + 1, y: y + 1, shadow },
        ]);
        const mismatches = [];
        for (let i = 0; i < data.length; i++)
          if (actual.data[i] !== expected.data[i])
            mismatches.push({ i, expected: expected.data[i], actual: actual.data[i] });
        if (mismatches.length)
          throw Error(
            JSON.stringify({
              pixel_format,
              strength_percent,
              x,
              y,
              mismatches: mismatches.slice(0, 8),
            }),
          );
        results.push({ pixel_format, strength_percent, x, y, bytes: data.length, exact: true });
      }
  const copied = compositor.compose(renderer, background, []);
  if (copied.data.some((v, i) => v !== data[i])) throw Error("empty copy");
  const borrowed = new THREE.WebGLRenderTarget(8, 8);
  renderer.setRenderTarget(borrowed);
  renderer.setViewport(1, 2, 3, 4);
  renderer.setScissor(2, 1, 4, 3);
  renderer.setScissorTest(true);
  renderer.autoClear = false;
  const before = {
    target: renderer.getRenderTarget() === borrowed,
    viewport: renderer.getViewport(new THREE.Vector4()).toArray(),
    scissor: renderer.getScissor(new THREE.Vector4()).toArray(),
    scissorTest: renderer.getScissorTest(),
    autoClear: renderer.autoClear,
  };
  const capture = () => ({
    target: renderer.getRenderTarget() === borrowed,
    viewport: renderer.getViewport(new THREE.Vector4()).toArray(),
    scissor: renderer.getScissor(new THREE.Vector4()).toArray(),
    scissorTest: renderer.getScissorTest(),
    autoClear: renderer.autoClear,
  });
  compositor.compose(renderer, background, []);
  if (JSON.stringify(before) !== JSON.stringify(capture())) throw Error("success restoration");
  const original = renderer.render;
  renderer.render = () => {
    throw Error("injected draw failure");
  };
  let failed = false;
  try {
    compositor.compose(renderer, background, []);
  } catch (e) {
    failed = String(e).includes("injected draw failure");
  } finally {
    renderer.render = original;
  }
  if (!failed || JSON.stringify(before) !== JSON.stringify(capture()))
    throw Error("failure restoration");
  renderer.setRenderTarget(null);
  borrowed.dispose();
  (window as any).proof = {
    ready: true,
    results,
    emptyCopy: true,
    restoration: { success: true, injectedFailure: true },
  };
} catch (error) {
  (window as any).proof = { error: String(error) };
} finally {
  compositor.dispose();
  renderer.dispose();
}
const result = document.querySelector("#result");
if (result)
  result.textContent = (window as any).proof.error
    ? "FAIL " + (window as any).proof.error
    : "PASS " + JSON.stringify((window as any).proof);
