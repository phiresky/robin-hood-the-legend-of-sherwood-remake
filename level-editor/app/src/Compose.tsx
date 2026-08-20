// Compose mode: canvas viewer for editing a MapDraft (place / select / move /
// delete library assets) plus the placements list panel.
import { For, Show, createEffect, onCleanup } from "solid-js";
import type { MapDraft } from "@rle/shared";
import type { LibraryAsset, LibraryIndex } from "./library";
import {
  DEFAULT_BACKGROUND,
  SAVE_MARGIN,
  assetBitmap,
  assetById,
  drawOrder,
  guideRect,
  hitAlpha,
} from "./compose";

export interface ComposeViewerProps {
  draft: () => MapDraft | null;
  /** increments on new/open — triggers a view refit */
  docId: () => number;
  library: () => LibraryIndex | null;
  /** asset being placed — ghost follows the cursor, click drops a placement */
  placing: () => LibraryAsset | null;
  onCancelPlace: () => void;
  selected: () => number | null;
  onSelect: (idx: number | null) => void;
  onPlace: (asset: LibraryAsset, pos: [number, number]) => void;
  onMove: (idx: number, pos: [number, number]) => void;
  onDelete: (idx: number) => void;
  onCursor?: (x: number, y: number) => void;
}

interface View {
  x: number; // world coord at canvas left
  y: number;
  zoom: number;
}

export function ComposeViewer(props: ComposeViewerProps) {
  let canvas!: HTMLCanvasElement;
  let container!: HTMLDivElement;
  const view: View = { x: 0, y: 0, zoom: 0.25 };
  let raf = 0;
  // pointer interaction state
  let panning = false;
  let moving: { idx: number; dx: number; dy: number } | null = null;
  let lastX = 0;
  let lastY = 0;
  let ghost: [number, number] | null = null; // cursor world pos while placing

  const scheduleDraw = () => {
    if (!raf) raf = requestAnimationFrame(draw);
  };

  const placementBitmap = (assetId: string) => {
    const asset = assetById(props.library(), assetId);
    return asset ? assetBitmap(asset, scheduleDraw) : null;
  };

  const draw = () => {
    raf = 0;
    const ctx = canvas.getContext("2d")!;
    const dpr = window.devicePixelRatio || 1;
    const w = container.clientWidth;
    const h = container.clientHeight;
    if (canvas.width !== w * dpr || canvas.height !== h * dpr) {
      canvas.width = w * dpr;
      canvas.height = h * dpr;
      canvas.style.width = `${w}px`;
      canvas.style.height = `${h}px`;
    }
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    const draft = props.draft();
    // infinite canvas: the background fill covers the whole viewport
    ctx.fillStyle = draft ? (draft.background_color ?? DEFAULT_BACKGROUND) : "#181818";
    ctx.fillRect(0, 0, w, h);
    if (!draft) return;
    ctx.scale(view.zoom, view.zoom);
    ctx.translate(-view.x, -view.y);
    ctx.imageSmoothingEnabled = view.zoom < 1;

    const lib = props.library();

    // boundary guide: what save would write (content bounds + margin), or the
    // stored size for an opened draft without placements — never clamps
    const guide = guideRect(draft, lib);
    if (guide) {
      ctx.strokeStyle = "#555";
      ctx.lineWidth = 1 / view.zoom;
      ctx.setLineDash([6 / view.zoom, 4 / view.zoom]);
      ctx.strokeRect(guide[0], guide[1], guide[2], guide[3]);
      ctx.setLineDash([]);
    }
    for (const idx of drawOrder(draft, lib)) {
      const p = draft.placements[idx]!;
      const bmp = placementBitmap(p.asset);
      if (!bmp) continue; // still loading, or asset missing from the library
      drawPlacementImage(ctx, bmp, p.pos, p.scale ?? 1, p.flip_x ?? false);
    }

    const sel = props.selected();
    if (sel !== null) {
      const p = draft.placements[sel];
      const bmp = p ? placementBitmap(p.asset) : null;
      if (p && bmp) {
        const s = p.scale ?? 1;
        ctx.strokeStyle = "#60a5fa";
        ctx.lineWidth = 2 / view.zoom;
        ctx.setLineDash([8 / view.zoom, 5 / view.zoom]);
        ctx.strokeRect(p.pos[0], p.pos[1], bmp.width * s, bmp.height * s);
        ctx.setLineDash([]);
      }
    }

    const placing = props.placing();
    if (placing && ghost) {
      const bmp = assetBitmap(placing, scheduleDraw);
      if (bmp) {
        const pos = ghostPos(placing, ghost);
        ctx.globalAlpha = 0.5;
        drawPlacementImage(ctx, bmp, pos, 1, false);
        ctx.globalAlpha = 1;
      }
    }
  };

  const drawPlacementImage = (
    ctx: CanvasRenderingContext2D,
    bmp: ImageBitmap,
    pos: readonly [number, number],
    scale: number,
    flipX: boolean,
  ) => {
    ctx.save();
    ctx.translate(pos[0], pos[1]);
    ctx.scale(scale, scale);
    if (flipX) {
      ctx.translate(bmp.width, 0);
      ctx.scale(-1, 1);
    }
    ctx.drawImage(bmp, 0, 0);
    ctx.restore();
  };

  /** placement pos so the asset's anchor lands under the cursor */
  const ghostPos = (asset: LibraryAsset, cursor: [number, number]): [number, number] => [
    Math.round(cursor[0] - asset.descriptor.anchor[0]),
    Math.round(cursor[1] - asset.descriptor.anchor[1]),
  ];

  const toWorld = (clientX: number, clientY: number): [number, number] => {
    const rect = canvas.getBoundingClientRect();
    return [
      view.x + (clientX - rect.left) / view.zoom,
      view.y + (clientY - rect.top) / view.zoom,
    ];
  };

  /** topmost placement under the world point, by draw order, alpha-aware */
  const hitTest = (wx: number, wy: number): number | null => {
    const draft = props.draft();
    if (!draft) return null;
    const order = drawOrder(draft, props.library());
    for (let i = order.length - 1; i >= 0; i--) {
      const idx = order[i]!;
      const p = draft.placements[idx]!;
      const bmp = placementBitmap(p.asset);
      if (!bmp) continue;
      const s = p.scale ?? 1;
      let lx = (wx - p.pos[0]) / s;
      const ly = (wy - p.pos[1]) / s;
      if (lx < 0 || ly < 0 || lx >= bmp.width || ly >= bmp.height) continue;
      if (p.flip_x) lx = bmp.width - 1 - lx;
      if (hitAlpha(p.asset, bmp, lx, ly)) return idx;
    }
    return null;
  };

  const onWheel = (e: WheelEvent) => {
    e.preventDefault();
    const [wx, wy] = toWorld(e.clientX, e.clientY);
    const factor = Math.exp(-e.deltaY * 0.0015);
    view.zoom = Math.min(8, Math.max(0.02, view.zoom * factor));
    const rect = canvas.getBoundingClientRect();
    view.x = wx - (e.clientX - rect.left) / view.zoom;
    view.y = wy - (e.clientY - rect.top) / view.zoom;
    scheduleDraw();
  };

  const onPointerDown = (e: PointerEvent) => {
    canvas.setPointerCapture(e.pointerId);
    lastX = e.clientX;
    lastY = e.clientY;
    if (e.button === 1) {
      panning = true;
      return;
    }
    if (e.button !== 0) return;
    const [wx, wy] = toWorld(e.clientX, e.clientY);
    const placing = props.placing();
    if (placing) {
      props.onPlace(placing, ghostPos(placing, [wx, wy]));
      return;
    }
    const hit = hitTest(wx, wy);
    props.onSelect(hit);
    if (hit !== null) {
      const p = props.draft()!.placements[hit]!;
      moving = { idx: hit, dx: wx - p.pos[0], dy: wy - p.pos[1] };
    } else {
      panning = true;
    }
  };

  const onPointerMove = (e: PointerEvent) => {
    const [wx, wy] = toWorld(e.clientX, e.clientY);
    props.onCursor?.(wx, wy);
    if (props.placing()) {
      ghost = [wx, wy];
      scheduleDraw();
    }
    if (moving) {
      props.onMove(moving.idx, [Math.round(wx - moving.dx), Math.round(wy - moving.dy)]);
      return;
    }
    if (!panning) return;
    view.x -= (e.clientX - lastX) / view.zoom;
    view.y -= (e.clientY - lastY) / view.zoom;
    lastX = e.clientX;
    lastY = e.clientY;
    scheduleDraw();
  };

  const onPointerUp = () => {
    panning = false;
    moving = null;
  };

  const onPointerLeave = () => {
    if (ghost) {
      ghost = null;
      scheduleDraw();
    }
  };

  const onKeyDown = (e: KeyboardEvent) => {
    const t = e.target as HTMLElement;
    if (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT") return;
    if (e.key === "Escape") {
      if (props.placing()) props.onCancelPlace();
      else props.onSelect(null);
      return;
    }
    const sel = props.selected();
    if (sel === null) return;
    if (e.key === "Delete" || e.key === "Backspace") {
      e.preventDefault();
      props.onDelete(sel);
      return;
    }
    const step = e.shiftKey ? 10 : 1;
    const nudge: Record<string, [number, number]> = {
      ArrowLeft: [-step, 0],
      ArrowRight: [step, 0],
      ArrowUp: [0, -step],
      ArrowDown: [0, step],
    };
    const d = nudge[e.key];
    if (!d) return;
    e.preventDefault();
    const p = props.draft()?.placements[sel];
    if (p) props.onMove(sel, [p.pos[0] + d[0], p.pos[1] + d[1]]);
  };

  // one-shot setup after mount
  createEffect(
    () => undefined,
    () => {
      const ro = new ResizeObserver(scheduleDraw);
      ro.observe(container);
      onCleanup(() => ro.disconnect());
      canvas.addEventListener("wheel", onWheel, { passive: false });
      onCleanup(() => canvas.removeEventListener("wheel", onWheel));
      window.addEventListener("keydown", onKeyDown);
      onCleanup(() => window.removeEventListener("keydown", onKeyDown));
    },
  );

  // fit view when a different draft document is loaded (docId bumps on
  // new/open, not on placement edits)
  createEffect(
    () => props.docId(),
    () => {
      const draft = props.draft(); // untracked read — edits must not refit
      if (!draft) return;
      const w = container.clientWidth || 1;
      const h = container.clientHeight || 1;
      const guide = guideRect(draft, props.library());
      if (guide) {
        view.zoom = Math.min(8, Math.min(w / guide[2], h / guide[3]) * 0.95);
        view.x = guide[0] - (w / view.zoom - guide[2]) / 2;
        view.y = guide[1] - (h / view.zoom - guide[3]) / 2;
      } else {
        // empty draft: center the world origin
        view.zoom = 0.5;
        view.x = -w / view.zoom / 2;
        view.y = -h / view.zoom / 2;
      }
      scheduleDraw();
    },
  );

  // redraw on any data change
  createEffect(
    () => [props.draft(), props.library(), props.selected(), props.placing()],
    () => scheduleDraw(),
  );

  return (
    <div class="viewer" ref={container}>
      <canvas
        ref={canvas}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerLeave={onPointerLeave}
      />
    </div>
  );
}

export interface PlacementsPanelProps {
  draft: () => MapDraft;
  library: () => LibraryIndex | null;
  selected: () => number | null;
  onSelect: (idx: number | null) => void;
  onDelete: (idx: number) => void;
}

export function PlacementsPanel(props: PlacementsPanelProps) {
  let list!: HTMLDivElement;

  // keep the selected row visible when selection comes from the canvas
  createEffect(
    () => props.selected(),
    (sel) => {
      if (sel === null) return;
      list.querySelector(`[data-idx="${sel}"]`)?.scrollIntoView({ block: "nearest" });
    },
  );

  return (
    <aside class="detail placements">
      <div class="detail-head">
        <h2>Placements ({props.draft().placements.length})</h2>
      </div>
      <div class="placement-list" ref={list}>
        <For each={props.draft().placements}>
          {(p, idx) => {
            const asset = () => assetById(props.library(), p.asset);
            return (
              <div
                class={`placement-row ${props.selected() === idx() ? "selected" : ""}`}
                data-idx={idx()}
                onClick={() => props.onSelect(idx())}
              >
                <span class={`placement-name ${asset() ? "" : "missing"}`}>
                  {asset()?.descriptor.name ?? `${p.asset} (missing)`}
                </span>
                <span class="placement-pos">
                  {p.pos[0]},{p.pos[1]}
                </span>
                <button
                  class="row-delete"
                  title="delete placement"
                  onClick={(e) => {
                    e.stopPropagation();
                    props.onDelete(idx());
                  }}
                >
                  ×
                </button>
              </div>
            );
          }}
        </For>
        <Show when={props.draft().placements.length === 0}>
          <p class="hint">
            No placements yet. Select a library asset, then click on the canvas to
            place it.
          </p>
        </Show>
      </div>
    </aside>
  );
}
