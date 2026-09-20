import type { Vec3 } from "@rle/shared";
import { cylinderPixel } from "./entity-projection.ts";

export type SpriteKind = "character" | "pickup" | "scenery";
export type SpriteShape = "upright-character" | "prone-character" | "low-object" | "upright-scenery";
const PRONE_ACTIONS = new Set([45, 47, 48, 106, 108, 109, 113, 115, 116, 219]);

export function spriteShape(kind: SpriteKind, action: number): SpriteShape {
  if (kind === "pickup") return "low-object";
  if (kind === "scenery") return "upright-scenery";
  return PRONE_ACTIONS.has(action) ? "prone-character" : "upright-character";
}

export interface SpriteBounds { left: number; top: number; width: number; height: number }
/** Each profile preserves source pixels at the map camera. Prone bodies and
 * pickups occupy shallow ground volumes; scenery never acquires a human cap. */
export function projectSpritePixel(shape: SpriteShape, x: number, up: number, bounds: SpriteBounds, elevation: number): Vec3 {
  const sin = Math.sin(elevation), cos = Math.cos(elevation);
  if (shape === "upright-character") {
    return cylinderPixel(x, up, Math.max(3, bounds.width * 0.25), Math.max(1, bounds.top / cos), elevation);
  }
  if (shape === "upright-scenery") return [x, up / cos, 0];
  const u = (x - bounds.left) / bounds.width * 2 - 1;
  const v = (bounds.top - up) / bounds.height * 2 - 1;
  const thickness = shape === "prone-character" ? 9 : Math.min(16, bounds.height * 0.3);
  const height = 0.5 + thickness * Math.sqrt(Math.max(0, 1 - u * u - v * v));
  return [x, height, (height * cos - up) / sin];
}

export const BONUS_SPRITES: readonly (readonly [string, string])[] = [
  ["BONUS_Arrows", "BONUS Fleches"], ["BONUS_Stones", "BONUS Cailloux"],
  ["BONUS_Apples", "BONUS Pommes"], ["BONUS_Ale", "BONUS Ale"],
  ["BONUS_LegOfLamb", "BONUS Gigots"], ["BONUS_Plants", "BONUS Plantes"],
  ["BONUS_Nets", "BONUS Filets"], ["BONUS_WaspsNest", "BONUS Guepes"],
  ["BONUS_MoneyBag", "BONUS Bourses d'argent"], ["BONUS_GoldBagsRansom", "BONUS Sac d'or rancon"],
  ["BONUS_FourLeavedClover", "BONUS Trefle"], ["BONUS_Shield", "Shield"],
  ["RELIC_Ampulla", "Huile"], ["RELIC_Spoon", "Cuillere"],
  ["RELIC_Crown", "Couronne"], ["RELIC_Stamp", "Sceau"],
  ["RELIC_Sceptre", "Sceptre"], ["RELIC_Book", "Registre"], ["RELIC_Sword", "Epee"],
];
export function bonusSprite(type: number): readonly [string, string] {
  const asset = BONUS_SPRITES[type];
  if (!asset) throw new Error(`Unknown bonus type ${type}`);
  return asset;
}
export function sanitizedProfileName(name: string): string {
  return name.replace(/[\\/:*?"<>|\x00-\x1f]/g, "_").replace(/^\.+|\.+$/g, "");
}
