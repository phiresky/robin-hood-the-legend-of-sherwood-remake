// Compose mode: canvas viewer for editing a MapDraft (place / select / move /
// delete library assets, draw wall runs) plus the placements list panel.
import { For, Show, createEffect, onCleanup } from "solid-js";
import type { MapDraft } from "@rle/shared";
import { expandWallRun } from "@rle/shared";
import type { LibraryAsset, LibraryIndex } from "./library";
import {
  DEFAULT_BACKGROUND,
  SAVE_MARGIN,
  assetBitmap,
  assetById,
  drawItems,
  guideRect,
  hitAlpha,
  selectionEquals,
  wallStamps,
  type DraftSelection,
} from "./compose";

export interface ComposeViewerProps {
  draft: () => MapDraft | null;
  /** increments on new/open — triggers a view refit */
  docId: () => number;
  library: () => LibraryIndex | null;
  /** asset being placed — ghost follows the cursor, click drops a placement */
  placing: () => LibraryAsset | null;
  onCancelPlace: () => void;
  /** wall-draw mode: clicks add control points for a run of this asset */
  wallAsset: () => LibraryAsset | null;
  onCommitWall: (asset: LibraryAsset, points: [number, number][]) => void;
  onCancelWall: () => void;
  selected: () => DraftSelection;
  onSelect: (sel: DraftSelection) => void;
  onPlace: (asset: LibraryAsset, pos: [number, number]) => void;
  onMove: (idx: number, pos: [number, number]) => void;
  onDelete: (sel: NonNullable<DraftSelection>) => void;
  onMoveWallPoint: (wall: number, pt: number, pos: [number, number]) => void;
  onInsertWallPoint: (wall: number, after: number, pos: [number, number]) => void;
  onRemoveWallPoint: (wall: number, pt: number) => void;
  onCursor?: (x: number, y: number) => void;
}

interface View {
  x: number; // world coord at canvas left
  y: number;
  zoom: number;
}

/** screen-pixel radius for grabbing wall control-point handles */
const HANDLE_R = 7;

export function ComposeViewer(props: ComposeViewerProps) {
  let canvas!: HTMLCanvasElement;
  let container!: HTMLDivElement;
  const view: View = { x: 0, y: 0, zoom: 0.25 };
  let raf = 0;
  // pointer interaction state
  let panning = false;
  let moving: { idx: number; dx: number; dy: number } | null = null;
  let draggingPoint: { wall: number; pt: number } | null = null;
  let lastX = 0;
  let lastY = 0;
  let ghost: [number, number] | null = null; // cursor world pos while placing/drawing
  // in-progress wall run control points (world coords), not yet in the draft
  let wallPoints: [number, number][] = [];

  const scheduleDraw = () => {
    if (!raf) raf = requestAnimationFrame(draw);
  };

  const placementBitmap = (assetId: string) => {
    const asset = assetById(props.library(), assetId);
    return asset ? assetBitmap(asset, scheduleDraw) : null;
  };

  const wallRunAsset = (wall: number): LibraryAsset | null => {
    const run = props.draft()?.walls?.[wall];
    return run ? assetById(props.library(), run.asset) : null;
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
    for (const item of drawItems(draft, lib)) {
      if (item.kind === "placement") {
        const p = draft.placements[item.idx]!;
        const bmp = placementBitmap(p.asset);
        if (!bmp) continue; // still loading, or asset missing from the library
        drawPlacementImage(ctx, bmp, p.pos, p.scale ?? 1, p.flip_x ?? false);
      } else {
        const run = draft.walls![item.wall]!;
        const bmp = placementBitmap(run.asset);
        if (!bmp) continue;
        drawPlacementImage(ctx, bmp, item.pos, 1, false);
      }
    }

    const sel = props.selected();
    if (sel?.kind === "placement") {
      const p = draft.placements[sel.idx];
      const bmp = p ? placementBitmap(p.asset) : null;
      if (p && bmp) {
        const s = p.scale ?? 1;
        ctx.strokeStyle = "#60a5fa";
        ctx.lineWidth = 2 / view.zoom;
        ctx.setLineDash([8 / view.zoom, 5 / view.zoom]);
        ctx.strokeRect(p.pos[0], p.pos[1], bmp.width * s, bmp.height * s);
        ctx.setLineDash([]);
      }
    } else if (sel?.kind === "wall") {
      const run = draft.walls?.[sel.idx];
      if (run) drawWallHandles(ctx, run.points);
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

    // live preview of the in-progress wall run: committed points + cursor
    const wallAsset = props.wallAsset();
    if (wallAsset && (wallPoints.length > 0 || ghost)) {
      const pts = ghost ? [...wallPoints, ghost] : wallPoints;
      const bmp = assetBitmap(wallAsset, scheduleDraw);
      if (bmp && pts.length >= 2) {
        const d = wallAsset.descriptor;
        const [, , aw, ah] = d.source.bbox;
        ctx.globalAlpha = 0.6;
        for (const s of expandWallRun(pts, [aw, ah], d.anchor)) {
          drawPlacementImage(ctx, bmp, s.pos, 1, false);
        }
        ctx.globalAlpha = 1;
      }
      drawWallHandles(ctx, pts, "#facc15");
    }
  };

  const drawWallHandles = (
    ctx: CanvasRenderingContext2D,
    points: readonly [number, number][],
    color = "#60a5fa",
  ) => {
    if (points.length === 0) return;
    ctx.strokeStyle = color;
    ctx.lineWidth = 1.5 / view.zoom;
    if (points.length > 1) {
      ctx.beginPath();
      ctx.moveTo(points[0]![0], points[0]![1]);
      for (const [x, y] of points.slice(1)) ctx.lineTo(x, y);
      ctx.stroke();
    }
    const r = HANDLE_R / view.zoom;
    ctx.fillStyle = "#1e2a44";
    for (const [x, y] of points) {
      ctx.fillRect(x - r / 2, y - r / 2, r, r);
      ctx.strokeRect(x - r / 2, y - r / 2, r, r);
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

  /** topmost placement or wall stamp under the world point, alpha-aware */
  const hitTest = (wx: number, wy: number): DraftSelection => {
    const draft = props.draft();
    if (!draft) return null;
    const items = drawItems(draft, props.library());
    for (let i = items.length - 1; i >= 0; i--) {
      const item = items[i]!;
      if (item.kind === "placement") {
        const p = draft.placements[item.idx]!;
        const bmp = placementBitmap(p.asset);
        if (!bmp) continue;
        const s = p.scale ?? 1;
        let lx = (wx - p.pos[0]) / s;
        const ly = (wy - p.pos[1]) / s;
        if (lx < 0 || ly < 0 || lx >= bmp.width || ly >= bmp.height) continue;
        if (p.flip_x) lx = bmp.width - 1 - lx;
        if (hitAlpha(p.asset, bmp, lx, ly)) return { kind: "placement", idx: item.idx };
      } else {
        const run = draft.walls![item.wall]!;
        const bmp = placementBitmap(run.asset);
        if (!bmp) continue;
        const lx = wx - item.pos[0];
        const ly = wy - item.pos[1];
        if (lx < 0 || ly < 0 || lx >= bmp.width || ly >= bmp.height) continue;
        if (hitAlpha(run.asset, bmp, lx, ly)) return { kind: "wall", idx: item.wall };
      }
    }
    return null;
  };

  /** control-point index of the selected wall near the world point, if any */
  const hitWallHandle = (wx: number, wy: number): { wall: number; pt: number } | null => {
    const sel = props.selected();
    if (sel?.kind !== "wall") return null;
    const run = props.draft()?.walls?.[sel.idx];
    if (!run) return null;
    const r = HANDLE_R / view.zoom;
    for (let i = 0; i < run.points.length; i++) {
      const [x, y] = run.points[i]!;
      if (Math.abs(wx - x) <= r && Math.abs(wy - y) <= r) return { wall: sel.idx, pt: i };
    }
    return null;
  };

  /** segment index of the selected wall whose midpoint is near the world point */
  const hitWallMidpoint = (
    wx: number,
    wy: number,
  ): { wall: number; after: number; pos: [number, number] } | null => {
    const sel = props.selected();
    if (sel?.kind !== "wall") return null;
    const run = props.draft()?.walls?.[sel.idx];
    if (!run) return null;
    const r = (HANDLE_R * 2) / view.zoom;
    for (let i = 0; i + 1 < run.points.length; i++) {
      const [x1, y1] = run.points[i]!;
      const [x2, y2] = run.points[i + 1]!;
      const mx = (x1 + x2) / 2;
      const my = (y1 + y2) / 2;
      if (Math.abs(wx - mx) <= r && Math.abs(wy - my) <= r)
        return { wall: sel.idx, after: i, pos: [Math.round(mx), Math.round(my)] };
    }
    return null;
  };

  const commitWall = () => {
    const asset = props.wallAsset();
    if (!asset) return;
    // drop consecutive near-duplicate points (double-click adds the last one twice)
    const pts: [number, number][] = [];
    for (const p of wallPoints) {
      const prev = pts[pts.length - 1];
      if (prev && Math.hypot(p[0] - prev[0], p[1] - prev[1]) < 3) continue;
      pts.push(p);
    }
    wallPoints = [];
    if (pts.length >= 2) props.onCommitWall(asset, pts);
    scheduleDraw();
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
    const [wx, wy] = toWorld(e.clientX, e.clientY);
    if (props.wallAsset()) {
      if (e.button === 0) {
        wallPoints.push([Math.round(wx), Math.round(wy)]);
        scheduleDraw();
      }
      return;
    }
    // Alt-click / right-click a segment midpoint of the selected wall inserts
    // a control point there
    if (e.button === 2 || (e.button === 0 && e.altKey)) {
      const mid = hitWallMidpoint(wx, wy);
      if (mid) {
        props.onInsertWallPoint(mid.wall, mid.after, mid.pos);
        draggingPoint = { wall: mid.wall, pt: mid.after + 1 };
      }
      return;
    }
    if (e.button !== 0) return;
    const placing = props.placing();
    if (placing) {
      props.onPlace(placing, ghostPos(placing, [wx, wy]));
      return;
    }
    const handle = hitWallHandle(wx, wy);
    if (handle) {
      draggingPoint = handle;
      return;
    }
    const hit = hitTest(wx, wy);
    if (!selectionEquals(hit, props.selected())) props.onSelect(hit);
    if (hit?.kind === "placement") {
      const p = props.draft()!.placements[hit.idx]!;
      moving = { idx: hit.idx, dx: wx - p.pos[0], dy: wy - p.pos[1] };
    } else if (hit === null) {
      panning = true;
    }
  };

  const onPointerMove = (e: PointerEvent) => {
    const [wx, wy] = toWorld(e.clientX, e.clientY);
    props.onCursor?.(wx, wy);
    if (props.placing() || props.wallAsset()) {
      ghost = [wx, wy];
      scheduleDraw();
    }
    if (draggingPoint) {
      props.onMoveWallPoint(draggingPoint.wall, draggingPoint.pt, [
        Math.round(wx),
        Math.round(wy),
      ]);
      return;
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
    draggingPoint = null;
  };

  const onPointerLeave = () => {
    if (ghost) {
      ghost = null;
      scheduleDraw();
    }
  };

  const onDblClick = (e: MouseEvent) => {
    if (props.wallAsset()) {
      commitWall();
      return;
    }
    // double-click a handle of the selected wall removes that point (min 2)
    const [wx, wy] = toWorld(e.clientX, e.clientY);
    const handle = hitWallHandle(wx, wy);
    if (handle) {
      const run = props.draft()?.walls?.[handle.wall];
      if (run && run.points.length > 2) props.onRemoveWallPoint(handle.wall, handle.pt);
    }
  };

  const onKeyDown = (e: KeyboardEvent) => {
    const t = e.target as HTMLElement;
    if (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT") return;
    if (props.wallAsset()) {
      if (e.key === "Enter") {
        e.preventDefault();
        commitWall();
      } else if (e.key === "Escape") {
        wallPoints = [];
        props.onCancelWall();
        scheduleDraw();
      }
      return;
    }
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
    if (sel.kind === "placement") {
      const p = props.draft()?.placements[sel.idx];
      if (p) props.onMove(sel.idx, [p.pos[0] + d[0], p.pos[1] + d[1]]);
    } else {
      const run = props.draft()?.walls?.[sel.idx];
      if (run)
        run.points.forEach((pt, i) =>
          props.onMoveWallPoint(sel.idx, i, [pt[0] + d[0], pt[1] + d[1]]),
        );
    }
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
      wallPoints = [];
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

  // leaving wall-draw mode (toggle off, asset change) drops in-progress points
  createEffect(
    () => props.wallAsset(),
    (asset) => {
      if (!asset && wallPoints.length > 0) {
        wallPoints = [];
        scheduleDraw();
      }
    },
  );

  // redraw on any data change
  createEffect(
    () => [
      props.draft(),
      props.library(),
      props.selected(),
      props.placing(),
      props.wallAsset(),
    ],
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
        onDblClick={onDblClick}
        onContextMenu={(e) => e.preventDefault()}
      />
    </div>
  );
}

export interface PlacementsPanelProps {
  draft: () => MapDraft;
  library: () => LibraryIndex | null;
  selected: () => DraftSelection;
  onSelect: (sel: DraftSelection) => void;
  onDelete: (sel: NonNullable<DraftSelection>) => void;
}

export function PlacementsPanel(props: PlacementsPanelProps) {
  let list!: HTMLDivElement;

  const selKey = () => {
    const sel = props.selected();
    return sel ? `${sel.kind}-${sel.idx}` : null;
  };

  // keep the selected row visible when selection comes from the canvas
  createEffect(
    () => selKey(),
    (key) => {
      if (key === null) return;
      list.querySelector(`[data-key="${key}"]`)?.scrollIntoView({ block: "nearest" });
    },
  );

  const count = () =>
    props.draft().placements.length + (props.draft().walls?.length ?? 0);

  return (
    <aside class="detail placements">
      <div class="detail-head">
        <h2>Placements ({count()})</h2>
      </div>
      <div class="placement-list" ref={list}>
        <For each={props.draft().placements}>
          {(p, idx) => {
            const asset = () => assetById(props.library(), p.asset);
            return (
              <div
                class={`placement-row ${selKey() === `placement-${idx()}` ? "selected" : ""}`}
                data-key={`placement-${idx()}`}
                onClick={() => props.onSelect({ kind: "placement", idx: idx() })}
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
                    props.onDelete({ kind: "placement", idx: idx() });
                  }}
                >
                  ×
                </button>
              </div>
            );
          }}
        </For>
        <For each={props.draft().walls ?? []}>
          {(run, idx) => {
            const asset = () => assetById(props.library(), run.asset);
            return (
              <div
                class={`placement-row ${selKey() === `wall-${idx()}` ? "selected" : ""}`}
                data-key={`wall-${idx()}`}
                onClick={() => props.onSelect({ kind: "wall", idx: idx() })}
              >
                <span class={`placement-name ${asset() ? "" : "missing"}`}>
                  wall: {asset()?.descriptor.name ?? `${run.asset} (missing)`}
                </span>
                <span class="placement-pos">{run.points.length} pts</span>
                <button
                  class="row-delete"
                  title="delete wall"
                  onClick={(e) => {
                    e.stopPropagation();
                    props.onDelete({ kind: "wall", idx: idx() });
                  }}
                >
                  ×
                </button>
              </div>
            );
          }}
        </For>
        <Show when={count() === 0}>
          <p class="hint">
            No placements yet. Select a library asset, then click on the canvas to
            place it.
          </p>
        </Show>
      </div>
    </aside>
  );
}
