// Asset-library browser: sidebar gallery + detail panel for extracted assets.
import { For, Show, createEffect, createSignal } from "solid-js";
import {
  filterAssets,
  groupAssets,
  loadAssetImage,
  type LibraryAsset,
  type LibraryIndex,
} from "./library";

export interface LibrarySectionProps {
  library: () => LibraryIndex | null;
  needsReconnect: () => boolean;
  onPick: () => void;
  onReconnect: () => void;
  onRefresh: () => void;
  selected: () => LibraryAsset | null;
  onSelect: (asset: LibraryAsset | null) => void;
}

export function LibrarySection(props: LibrarySectionProps) {
  const [open, setOpen] = createSignal(true);
  const [query, setQuery] = createSignal("");

  const groups = () => {
    const lib = props.library();
    if (!lib) return [];
    return groupAssets(filterAssets(lib.assets, query()));
  };

  const select = (asset: LibraryAsset) =>
    props.onSelect(props.selected() === asset ? null : asset);

  return (
    <section class="library">
      <h2 class="collapsible" onClick={() => setOpen(!open())}>
        <span class={`chev ${open() ? "open" : ""}`}>▸</span> Library
      </h2>
      <Show when={open()}>
        <Show
          when={props.library()}
          fallback={
            <div class="connect">
              <button onClick={props.onPick}>Open asset library…</button>
              <Show when={props.needsReconnect()}>
                <button onClick={props.onReconnect}>Reconnect library</button>
              </Show>
              <p class="hint">
                Pick the <code>level-editor/library</code> folder
              </p>
            </div>
          }
        >
          <div class="search-row">
            <input
              class="search"
              type="search"
              placeholder="filter by name or tag…"
              value={query()}
              onInput={(e) => setQuery(e.currentTarget.value)}
            />
            <button class="refresh" onClick={props.onRefresh} title="rescan library">
              ⟳
            </button>
          </div>
          <div class="gallery">
            <For each={groups()}>
              {([group, assets]) => (
                <div class="asset-group">
                  <Show when={assets.length > 1}>
                    <div class="group-name">{group}</div>
                  </Show>
                  <div class="thumbs">
                    <For each={assets}>
                      {(asset) => (
                        <div
                          class={`thumb ${props.selected() === asset ? "selected" : ""}`}
                          onClick={() => select(asset)}
                          title={`${asset.descriptor.name}\n${asset.descriptor.tags.join(", ")}`}
                        >
                          <img src={asset.dayUrl} alt={asset.descriptor.name} loading="lazy" />
                          <div class="thumb-name">{asset.descriptor.name}</div>
                          <div class="thumb-map">{asset.descriptor.source.map}</div>
                          <div class="thumb-meta">
                            <span class="scale-class">{asset.descriptor.scale_class}</span>
                            <For each={asset.descriptor.tags}>
                              {(tag) => <span class="tag">{tag}</span>}
                            </For>
                          </div>
                        </div>
                      )}
                    </For>
                  </div>
                </div>
              )}
            </For>
            <Show when={groups().length === 0}>
              <p class="hint">no assets match</p>
            </Show>
          </div>
        </Show>
      </Show>
    </section>
  );
}

/** trace the mask boundary (inside = value > 127) into a same-size overlay canvas */
function maskOutline(mask: ImageBitmap, color: [number, number, number]): HTMLCanvasElement {
  const w = mask.width;
  const h = mask.height;
  const src = document.createElement("canvas");
  src.width = w;
  src.height = h;
  const sctx = src.getContext("2d")!;
  sctx.drawImage(mask, 0, 0);
  const data = sctx.getImageData(0, 0, w, h).data;
  // mask.png is grayscale stored in RGB; read the red channel
  const inside = (x: number, y: number) =>
    x >= 0 && y >= 0 && x < w && y < h && data[(y * w + x) * 4]! > 127;

  const out = document.createElement("canvas");
  out.width = w;
  out.height = h;
  const octx = out.getContext("2d")!;
  const edge = octx.createImageData(w, h);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      if (!inside(x, y)) continue;
      if (inside(x - 1, y) && inside(x + 1, y) && inside(x, y - 1) && inside(x, y + 1))
        continue;
      const i = (y * w + x) * 4;
      edge.data[i] = color[0];
      edge.data[i + 1] = color[1];
      edge.data[i + 2] = color[2];
      edge.data[i + 3] = 255;
    }
  }
  octx.putImageData(edge, 0, 0);
  return out;
}

export interface AssetDetailProps {
  asset: () => LibraryAsset;
  onClose: () => void;
}

export function AssetDetail(props: AssetDetailProps) {
  let canvas!: HTMLCanvasElement;
  const [ambiance, setAmbiance] = createSignal<"day" | "night">("day");
  const [showMask, setShowMask] = createSignal(true);

  // redraw the preview whenever asset / ambiance / mask toggle changes
  let drawToken = 0;
  createEffect(
    () => [props.asset(), ambiance(), showMask()] as const,
    ([asset, amb, mask]) => {
      const d = asset.descriptor;
      if (amb === "night" && !d.images.night) {
        setAmbiance("day");
        return;
      }
      const token = ++drawToken;
      void (async () => {
        const imgName = amb === "night" ? d.images.night! : d.images.day;
        const img = await loadAssetImage(asset, imgName);
        const outline = mask
          ? maskOutline(await loadAssetImage(asset, d.images.mask), [96, 165, 250])
          : null;
        if (token !== drawToken) return; // stale draw
        canvas.width = img.width;
        canvas.height = img.height;
        const ctx = canvas.getContext("2d")!;
        ctx.clearRect(0, 0, img.width, img.height);
        ctx.drawImage(img, 0, 0);
        if (outline) ctx.drawImage(outline, 0, 0);
      })();
    },
  );

  const d = () => props.asset().descriptor;
  const counts = () => {
    const desc = d();
    return [
      ["sight obstacles", desc.volumes.sight_obstacles.length],
      ["motion obstacles", desc.motion.obstacles.length],
      ["walkable fragments", desc.motion.walkable.length],
      ["jump zones", desc.jump_zones?.length ?? 0],
      ["jump line pairs", desc.jump_line_pairs?.length ?? 0],
      ["material sectors", desc.material_sectors?.length ?? 0],
      ["occlusion masks", desc.occlusion_masks?.length ?? 0],
    ] as const;
  };

  return (
    <aside class="detail">
      <div class="detail-head">
        <div>
          <h2>{d().name}</h2>
          <div class="detail-map">{d().source.map}</div>
        </div>
        <button class="close" onClick={props.onClose} title="close">
          ×
        </button>
      </div>
      <div class="row">
        <button class={ambiance() === "day" ? "selected" : ""} onClick={() => setAmbiance("day")}>
          Day
        </button>
        <button
          class={ambiance() === "night" ? "selected" : ""}
          disabled={!d().images.night}
          onClick={() => setAmbiance("night")}
        >
          Night
        </button>
        <label class="check inline">
          <input
            type="checkbox"
            checked={showMask()}
            onChange={() => setShowMask(!showMask())}
          />
          mask outline
        </label>
      </div>
      <canvas class="preview" ref={canvas} />
      <div class="detail-meta">
        <div class="meta-row">
          <span class="meta-key">id</span>
          <span>{d().id}</span>
        </div>
        <div class="meta-row">
          <span class="meta-key">scale class</span>
          <span>{d().scale_class}</span>
        </div>
        <Show when={d().variant_group}>
          <div class="meta-row">
            <span class="meta-key">variant group</span>
            <span>{d().variant_group}</span>
          </div>
        </Show>
        <div class="meta-row">
          <span class="meta-key">source</span>
          <span>
            {d().source.map} ({d().source.ambiance})
          </span>
        </div>
        <div class="meta-row">
          <span class="meta-key">bbox</span>
          <span>
            {d().source.bbox[0]},{d().source.bbox[1]} {d().source.bbox[2]}×{d().source.bbox[3]}
          </span>
        </div>
        <div class="meta-row">
          <span class="meta-key">tags</span>
          <span>
            <For each={d().tags}>{(tag) => <span class="tag">{tag}</span>}</For>
          </span>
        </div>
      </div>
      <section>
        <h2>Carried data</h2>
        <div class="detail-meta">
          <For each={counts()}>
            {([key, n]) => (
              <div class={`meta-row ${n === 0 ? "zero" : ""}`}>
                <span class="meta-key">{key}</span>
                <span>{n}</span>
              </div>
            )}
          </For>
        </div>
      </section>
      <section>
        <h2>Extraction</h2>
        <div class="detail-meta">
          <div class="meta-row">
            <span class="meta-key">tool</span>
            <span>{d().source.extraction.tool}</span>
          </div>
          <Show when={d().source.extraction.score !== undefined}>
            <div class="meta-row">
              <span class="meta-key">score</span>
              <span>{d().source.extraction.score!.toFixed(3)}</span>
            </div>
          </Show>
          <Show when={d().source.extraction.prompt}>
            <p class="prompt">“{d().source.extraction.prompt}”</p>
          </Show>
        </div>
      </section>
      <Show when={d().notes}>
        <section>
          <h2>Notes</h2>
          <p class="hint">{d().notes}</p>
        </section>
      </Show>
    </aside>
  );
}
