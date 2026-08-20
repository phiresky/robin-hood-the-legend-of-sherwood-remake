// Canvas overlay renderers for proto-level and mission data.
// All drawing happens in world/map pixel space; the canvas transform is set by
// the viewer, and LINE px widths are divided by zoom to stay ~constant on screen.
import type { Mission, Point, ProtoLevel } from "@rle/shared";

export interface OverlayToggles {
  motion: boolean;
  sightObstacles: boolean;
  masks: boolean;
  jumpZones: boolean;
  patches: boolean;
  animations: boolean;
  materials: boolean;
  lights: boolean;
  sounds: boolean;
  elevation: boolean;
  entities: boolean;
  paths: boolean;
  tactics: boolean;
}

export const DEFAULT_TOGGLES: OverlayToggles = {
  motion: true,
  sightObstacles: true,
  masks: false,
  jumpZones: false,
  patches: false,
  animations: false,
  materials: false,
  lights: false,
  sounds: false,
  elevation: false,
  entities: true,
  paths: false,
  tactics: false,
};

export const TOGGLE_LABELS: Record<keyof OverlayToggles, string> = {
  motion: "Walkability",
  sightObstacles: "3D volumes",
  masks: "Occlusion masks",
  jumpZones: "Jump zones",
  patches: "Patches",
  animations: "Ambient FX",
  materials: "Materials",
  lights: "Light sectors",
  sounds: "Sound sources",
  elevation: "Elevation lines",
  entities: "Mission entities",
  paths: "Patrol paths",
  tactics: "AI tactics",
};

const LAYER_COLORS = [
  "#3ddc55",
  "#38bdf8",
  "#f472b6",
  "#facc15",
  "#a78bfa",
  "#fb923c",
  "#2dd4bf",
  "#f87171",
];

function poly(ctx: CanvasRenderingContext2D, pts: readonly Point[], close = true) {
  if (pts.length === 0) return;
  ctx.beginPath();
  ctx.moveTo(pts[0]![0], pts[0]![1]);
  for (let i = 1; i < pts.length; i++) ctx.lineTo(pts[i]![0], pts[i]![1]);
  if (close) ctx.closePath();
}

function marker(ctx: CanvasRenderingContext2D, x: number, y: number, r: number, color: string) {
  ctx.beginPath();
  ctx.arc(x, y, r, 0, Math.PI * 2);
  ctx.fillStyle = color;
  ctx.fill();
}

function label(
  ctx: CanvasRenderingContext2D,
  text: string,
  x: number,
  y: number,
  zoom: number,
  color = "#fff",
) {
  if (zoom < 0.35) return;
  ctx.font = `${12 / zoom}px sans-serif`;
  ctx.fillStyle = "rgba(0,0,0,0.6)";
  const w = ctx.measureText(text).width;
  ctx.fillRect(x - 2 / zoom, y - 12 / zoom, w + 4 / zoom, 14 / zoom);
  ctx.fillStyle = color;
  ctx.fillText(text, x, y);
}

export function drawProtoOverlays(
  ctx: CanvasRenderingContext2D,
  level: ProtoLevel,
  t: OverlayToggles,
  zoom: number,
  layerFilter: number | null,
) {
  const lw = (n: number) => n / zoom;
  const layerOk = (layer: number) => layerFilter === null || layer === layerFilter;

  if (t.materials) {
    for (const ms of level.material_sectors) {
      poly(ctx, ms.polygon.points);
      ctx.fillStyle = `hsla(${(ms.material * 47) % 360}, 70%, 50%, 0.15)`;
      ctx.fill();
      ctx.strokeStyle = `hsla(${(ms.material * 47) % 360}, 70%, 60%, 0.6)`;
      ctx.lineWidth = lw(1);
      ctx.stroke();
    }
  }

  if (t.lights) {
    for (const ls of level.light_sectors) {
      if (!layerOk(ls.layer)) continue;
      poly(ctx, ls.polygon.points);
      ctx.fillStyle = "rgba(255, 240, 150, 0.12)";
      ctx.fill();
      ctx.strokeStyle = "rgba(255, 240, 150, 0.5)";
      ctx.lineWidth = lw(1);
      ctx.stroke();
    }
  }

  if (t.motion) {
    level.motion_data.layers.forEach((areas, layerIdx) => {
      if (!layerOk(layerIdx)) return;
      const color = LAYER_COLORS[layerIdx % LAYER_COLORS.length]!;
      for (const area of areas) {
        poly(ctx, area.polygon.points);
        ctx.strokeStyle = color;
        ctx.lineWidth = lw(area.is_lift ? 3 : 2);
        ctx.setLineDash(area.is_lift ? [8 / zoom, 4 / zoom] : []);
        ctx.stroke();
        ctx.setLineDash([]);
        for (const obs of area.obstacles) {
          poly(ctx, obs.polygon.points);
          ctx.fillStyle = "rgba(248, 60, 60, 0.18)";
          ctx.fill();
          ctx.strokeStyle = "rgba(248, 60, 60, 0.7)";
          ctx.lineWidth = lw(1);
          ctx.stroke();
        }
      }
    });
  }

  if (t.sightObstacles) {
    for (const so of level.sight_obstacles) {
      const bottom: Point[] = so.points.map((p) => [p.x, p.y - p.z_bottom]);
      const top: Point[] = so.points.map((p) => [p.x, p.y - p.z_top]);
      poly(ctx, bottom);
      ctx.fillStyle = so.solid ? "rgba(56, 130, 248, 0.15)" : "rgba(56, 130, 248, 0.05)";
      ctx.fill();
      ctx.strokeStyle = "rgba(56, 130, 248, 0.8)";
      ctx.lineWidth = lw(1);
      ctx.stroke();
      poly(ctx, top);
      ctx.strokeStyle = "rgba(120, 200, 255, 0.8)";
      ctx.stroke();
      ctx.beginPath();
      for (let i = 0; i < so.points.length; i++) {
        ctx.moveTo(bottom[i]![0], bottom[i]![1]);
        ctx.lineTo(top[i]![0], top[i]![1]);
      }
      ctx.strokeStyle = "rgba(120, 200, 255, 0.4)";
      ctx.stroke();
      const zt = so.points[0]?.z_top ?? 0;
      if (zt !== 0) label(ctx, `z ${zt.toFixed(0)}`, top[0]![0], top[0]![1], zoom, "#9cf");
    }
  }

  if (t.masks) {
    for (const m of level.masks) {
      if (!layerOk(m.layer)) continue;
      ctx.strokeStyle = "rgba(255, 160, 40, 0.8)";
      ctx.lineWidth = lw(2);
      poly(ctx, m.character_polyline, false);
      ctx.stroke();
      ctx.strokeStyle = "rgba(255, 160, 40, 0.3)";
      ctx.lineWidth = lw(1);
      ctx.strokeRect(m.box_top_left[0], m.box_top_left[1], m.box_size[0], m.box_size[1]);
    }
  }

  if (t.elevation) {
    ctx.strokeStyle = "rgba(200, 120, 255, 0.8)";
    ctx.lineWidth = lw(1.5);
    ctx.beginPath();
    for (const el of level.elevation_lines) {
      if (!layerOk(el.layer)) continue;
      ctx.moveTo(el.point_a[0], el.point_a[1]);
      ctx.lineTo(el.point_b[0], el.point_b[1]);
    }
    ctx.stroke();
  }

  if (t.jumpZones) {
    for (const jz of level.jump_zones) {
      if (!layerOk(jz.layer)) continue;
      poly(ctx, jz.polygon.points);
      ctx.fillStyle = "rgba(190, 90, 255, 0.15)";
      ctx.fill();
      ctx.strokeStyle = "rgba(190, 90, 255, 0.8)";
      ctx.lineWidth = lw(1.5);
      ctx.stroke();
    }
    ctx.strokeStyle = "rgba(255, 90, 255, 0.9)";
    ctx.lineWidth = lw(2);
    for (const pair of level.jump_line_pairs) {
      for (const line of [pair.line1, pair.line2]) {
        ctx.beginPath();
        ctx.moveTo(line.point_a[0], line.point_a[1]);
        ctx.lineTo(line.point_b[0], line.point_b[1]);
        ctx.stroke();
      }
    }
  }

  if (t.patches) {
    for (const p of level.patches) {
      poly(ctx, p.apply_sector.points);
      ctx.fillStyle = "rgba(255, 220, 60, 0.12)";
      ctx.fill();
      ctx.strokeStyle = "rgba(255, 220, 60, 0.8)";
      ctx.lineWidth = lw(1.5);
      ctx.stroke();
      const [wx, wy] = p.waypoint;
      marker(ctx, wx, wy, 5 / zoom, "#fd0");
      label(ctx, p.element_fx.sprite.profile_name, wx + 8 / zoom, wy, zoom, "#fd0");
    }
  }

  if (t.animations) {
    for (const fx of level.animations) {
      const { position_x: x, position_y: y, profile_name } = fx.sprite;
      marker(ctx, x, y, 4 / zoom, fx.active ? "#4fd" : "#666");
      label(ctx, profile_name, x + 7 / zoom, y, zoom, "#4fd");
    }
  }

  if (t.sounds) {
    for (const s of level.sound_sources) {
      for (const [x, y] of s.polyline) {
        marker(ctx, x, y, 4 / zoom, "#fa4");
        ctx.beginPath();
        ctx.arc(x, y, s.inner_distance, 0, Math.PI * 2);
        ctx.strokeStyle = "rgba(255, 170, 60, 0.4)";
        ctx.lineWidth = lw(1);
        ctx.stroke();
        ctx.beginPath();
        ctx.arc(x, y, s.outer_distance, 0, Math.PI * 2);
        ctx.strokeStyle = "rgba(255, 170, 60, 0.15)";
        ctx.stroke();
      }
    }
  }
}

/** highlight rectangle for a selected library asset's source region */
export function drawSelectionBbox(
  ctx: CanvasRenderingContext2D,
  bbox: readonly [number, number, number, number],
  zoom: number,
) {
  const [x, y, w, h] = bbox;
  ctx.fillStyle = "rgba(37, 99, 235, 0.12)";
  ctx.fillRect(x, y, w, h);
  ctx.strokeStyle = "#60a5fa";
  ctx.lineWidth = 2 / zoom;
  ctx.setLineDash([8 / zoom, 5 / zoom]);
  ctx.strokeRect(x, y, w, h);
  ctx.setLineDash([]);
}

function entityDot(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  dir: number,
  zoom: number,
  color: string,
) {
  marker(ctx, x, y, 5 / zoom, color);
  // direction: 0..255-style angle in the original; draw a small heading tick
  const a = (dir / 256) * Math.PI * 2;
  ctx.beginPath();
  ctx.moveTo(x, y);
  ctx.lineTo(x + (Math.cos(a) * 12) / zoom, y + (Math.sin(a) * 12) / zoom);
  ctx.strokeStyle = color;
  ctx.lineWidth = 2 / zoom;
  ctx.stroke();
}

export function drawMissionOverlays(
  ctx: CanvasRenderingContext2D,
  mission: Mission,
  t: OverlayToggles,
  zoom: number,
) {
  if (t.paths) {
    mission.hiking_paths.forEach((path, i) => {
      const wps = Array.isArray(path.waypoints) ? path.waypoints : [];
      const pts: Point[] = [];
      for (const wp of wps) {
        const pos = (wp as { position?: Point }).position;
        if (pos) pts.push(pos);
      }
      if (pts.length < 2) return;
      poly(ctx, pts, false);
      ctx.strokeStyle = `hsla(${(i * 41) % 360}, 80%, 65%, 0.7)`;
      ctx.lineWidth = 1.5 / zoom;
      ctx.stroke();
      marker(ctx, pts[0]![0], pts[0]![1], 3 / zoom, `hsl(${(i * 41) % 360}, 80%, 65%)`);
      label(ctx, `path ${i}`, pts[0]![0] + 6 / zoom, pts[0]![1], zoom);
    });
  }

  if (t.entities) {
    for (const b of mission.beam_mes) {
      const [x, y] = b.position;
      marker(ctx, x, y, 7 / zoom, "#3f6");
      label(ctx, `spawn ${b.index}`, x + 9 / zoom, y, zoom, "#3f6");
    }
    for (const s of mission.soldiers)
      entityDot(ctx, s.position_x, s.position_y, s.direction, zoom, "#f44");
    for (const c of mission.civilians)
      entityDot(ctx, c.position_x, c.position_y, c.direction, zoom, "#fd6");
    for (const tt of mission.targets) marker(ctx, tt.position_x, tt.position_y, 5 / zoom, "#f0f");
    for (const bn of mission.bonuses) marker(ctx, bn.position_x, bn.position_y, 4 / zoom, "#6cf");
    for (const sc of mission.scrolls) marker(ctx, sc.position_x, sc.position_y, 4 / zoom, "#fff");
    for (const pc of mission.pcs_to_rescue)
      marker(ctx, pc.position_x, pc.position_y, 6 / zoom, "#0f0");
  }

  if (t.tactics) {
    const td = mission.tactic_data;
    const pt = (o: unknown): Point | null => {
      const anyo = o as Record<string, unknown>;
      if (typeof anyo.position_x === "number") return [anyo.position_x as number, anyo.position_y as number];
      if (Array.isArray(anyo.position)) return anyo.position as Point;
      return null;
    };
    for (const [list, color] of [
      [td.reinforcement_points, "#f80"],
      [td.ambush_points, "#f08"],
      [td.seek_points, "#08f"],
    ] as const) {
      for (const o of list) {
        const p = pt(o);
        if (p) marker(ctx, p[0], p[1], 3 / zoom, color);
      }
    }
  }
}
