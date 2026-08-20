import { createEffect, onCleanup } from "solid-js";
import type { Mission, ProtoLevel } from "@rle/shared";
import {
  drawMissionOverlays,
  drawProtoOverlays,
  drawSelectionBbox,
  type OverlayToggles,
} from "./overlays";

export interface ViewerProps {
  image: () => ImageBitmap | null;
  level: () => ProtoLevel | null;
  mission: () => Mission | null;
  toggles: () => OverlayToggles;
  layerFilter: () => number | null;
  /** source bbox of the selected library asset, when it lies on the loaded map */
  selection: () => readonly [number, number, number, number] | null;
  onCursor?: (x: number, y: number) => void;
}

interface View {
  x: number; // world coord at canvas left
  y: number;
  zoom: number;
}

export default function Viewer(props: ViewerProps) {
  let canvas!: HTMLCanvasElement;
  let container!: HTMLDivElement;
  const view: View = { x: 0, y: 0, zoom: 0.25 };
  let dragging = false;
  let lastX = 0;
  let lastY = 0;
  let raf = 0;

  const scheduleDraw = () => {
    if (!raf) raf = requestAnimationFrame(draw);
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
    ctx.fillStyle = "#181818";
    ctx.fillRect(0, 0, w, h);
    ctx.scale(view.zoom, view.zoom);
    ctx.translate(-view.x, -view.y);

    const img = props.image();
    if (img) {
      ctx.imageSmoothingEnabled = view.zoom < 1;
      ctx.drawImage(img, 0, 0);
    }
    const level = props.level();
    if (level) drawProtoOverlays(ctx, level, props.toggles(), view.zoom, props.layerFilter());
    const mission = props.mission();
    if (mission) drawMissionOverlays(ctx, mission, props.toggles(), view.zoom);
    const sel = props.selection();
    if (sel) drawSelectionBbox(ctx, sel, view.zoom);
  };

  const toWorld = (clientX: number, clientY: number): [number, number] => {
    const rect = canvas.getBoundingClientRect();
    return [
      view.x + (clientX - rect.left) / view.zoom,
      view.y + (clientY - rect.top) / view.zoom,
    ];
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
    dragging = true;
    lastX = e.clientX;
    lastY = e.clientY;
    canvas.setPointerCapture(e.pointerId);
  };
  const onPointerMove = (e: PointerEvent) => {
    const [wx, wy] = toWorld(e.clientX, e.clientY);
    props.onCursor?.(wx, wy);
    if (!dragging) return;
    view.x -= (e.clientX - lastX) / view.zoom;
    view.y -= (e.clientY - lastY) / view.zoom;
    lastX = e.clientX;
    lastY = e.clientY;
    scheduleDraw();
  };
  const onPointerUp = () => {
    dragging = false;
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
    },
  );

  // fit view when a new map image arrives
  createEffect(
    () => props.image(),
    (img) => {
      if (!img) return;
      const w = container.clientWidth || 1;
      const h = container.clientHeight || 1;
      view.zoom = Math.min(w / img.width, h / img.height);
      view.x = -(w / view.zoom - img.width) / 2;
      view.y = -(h / view.zoom - img.height) / 2;
      scheduleDraw();
    },
  );

  // redraw on any overlay/data change
  createEffect(
    () => [
      props.level(),
      props.mission(),
      props.layerFilter(),
      props.selection(),
      { ...props.toggles() },
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
      />
    </div>
  );
}
