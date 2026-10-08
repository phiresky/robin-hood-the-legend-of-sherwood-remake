import { render } from "@solidjs/web";
import { createSignal, Show } from "solid-js";
import AssetPreview, { AssetPreviewRenderer } from "../src/AssetPreview.tsx";
import { sceneryThumbnailFixture } from "./scenery-thumbnail-fixture.ts";

function directory(files: Map<string, Uint8Array>, prefix = ""): FileSystemDirectoryHandle {
  return {
    getDirectoryHandle: async (name: string) => directory(files, `${prefix}${name}/`),
    getFileHandle: async (name: string) => ({
      getFile: async () => {
        const bytes = files.get(prefix + name);
        if (!bytes) throw new Error(`Unexpected file read: ${prefix}${name}`);
        return new File([new Uint8Array(bytes)], name);
      },
    }),
  } as unknown as FileSystemDirectoryHandle;
}
const cleanups: (() => void)[] = [];
const renderer = new AssetPreviewRenderer();
const [visible, setVisible] = createSignal(true);
const [revision, setRevision] = createSignal(0);
try {
  const host = document.querySelector("#host")!;
  const cards: HTMLDivElement[] = [];
  for (const legacy of [true, false]) {
    const { descriptor, files } = await sceneryThumbnailFixture(legacy);
    const card = document.createElement("div");
    host.append(card);
    cards.push(card);
    const entry = {
      id: "fire",
      name: "Fire",
      source_map: "Authored",
      descriptor: "fire/asset.json",
      model: "fire/model.glb",
      editor: descriptor,
    };
    cleanups.push(
      render(
        () => <><Show when={visible()}><AssetPreview entry={entry} root={directory(files)} renderer={renderer} /></Show><span data-revision>{revision()}</span></>,
        card,
      ),
    );
  }
  const deadline = performance.now() + 10000;
  while (cards.some((card) => card.querySelector(".preview-status"))) {
    if (performance.now() > deadline)
      throw new Error(cards.map((card) => card.textContent).join("; "));
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  for (const [index, card] of cards.entries()) {
    const canvas = card.querySelector("canvas")!;
    const context = canvas.getContext("2d")!;
    const left = [...context.getImageData(50, 100, 1, 1).data];
    const right = [...context.getImageData(250, 100, 1, 1).data];
    if (right.join() !== "255,0,0,255") throw new Error(`Missing effect artwork: ${right}`);
    const expected = index === 0 ? "0,0,0,0" : "0,248,0,255";
    if (left.join() !== expected) throw new Error(`Wrong sprite color-key handling: ${left}`);
  }
  // Filtering cards disposes components within a reactive owner. A cleanup
  // write would halt updates, including unrelated controls that survive it.
  setVisible(false);
  await new Promise((resolve) => setTimeout(resolve, 20));
  setRevision(1);
  await new Promise((resolve) => setTimeout(resolve, 20));
  if (cards.some(card => card.querySelector("canvas") || card.querySelector("[data-revision]")?.textContent !== "1"))
    throw new Error("Filtering previews halted surviving reactive controls");
  document.querySelector("#result")!.textContent =
    "PASS scenery palette: pinned frames, legacy transparency, RGBA color, no model fallback, filtering cleanup preserves reactive controls";
} catch (error) {
  document.querySelector("#result")!.textContent = `FAIL ${String(error)}`;
} finally {
  for (const cleanup of cleanups) cleanup();
  renderer.dispose();
}
