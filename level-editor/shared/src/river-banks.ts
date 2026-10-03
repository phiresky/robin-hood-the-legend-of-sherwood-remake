import type { LevelSpline } from "./splines.ts";
import { splineSectionAt } from "./spline-sampling.ts";

export const riverBankStyles = {
  none: "No bank decoration",
  plain: "Plain",
  small_stones: "Small stones",
  big_stones: "Big stones",
  mixed_stones: "Mixed stones",
  stones_plants: "Small stones with plants",
  vegetation: "Vegetation",
} as const;
export type RiverBankStyle = keyof typeof riverBankStyles;
export type RiverBankSide = "left" | "right";
export interface RiverBankSetting {
  /** Absolute scene width, independent of the water ribbon. */
  width: number;
  mix: Partial<Record<RiverBankStyle, number>>;
}
export interface RiverBankPoint {
  left: RiverBankSetting;
  right: RiverBankSetting;
}
export function riverBankPoint(path: LevelSpline, index: number): RiverBankPoint {
  return (
    path.pointBanks?.[index] ?? {
      left: { width: 32, mix: { none: 1 } },
      right: { width: 32, mix: { none: 1 } },
    }
  );
}
export function riverBankAt(path: LevelSpline, parameter: number): RiverBankPoint {
  const { section, next, fraction } = splineSectionAt(path, parameter);
  const a = riverBankPoint(path, section),
    b = riverBankPoint(path, next);
  const blend = (side: RiverBankSide): RiverBankSetting => ({
    width: a[side].width + (b[side].width - a[side].width) * fraction,
    mix: Object.fromEntries(
      Object.keys(riverBankStyles)
        .map((key) => {
          const id = key as RiverBankStyle;
          return [id, (a[side].mix[id] ?? 0) * (1 - fraction) + (b[side].mix[id] ?? 0) * fraction];
        })
        .filter(([, value]) => Number(value) > 0),
    ),
  });
  return { left: blend("left"), right: blend("right") };
}

/** Sections set both endpoints so the chosen reach is uniform, with soft adjoining transitions. */
export function editRiverBanks(
  path: LevelSpline,
  indices: number[],
  side: RiverBankSide | "both",
  patch: Partial<RiverBankSetting>,
): RiverBankPoint[] {
  return path.points.map((_, index) => {
    const bank = riverBankPoint(path, index);
    if (!indices.includes(index)) return bank;
    return {
      left: side === "right" ? bank.left : { ...bank.left, ...patch },
      right: side === "left" ? bank.right : { ...bank.right, ...patch },
    };
  });
}
