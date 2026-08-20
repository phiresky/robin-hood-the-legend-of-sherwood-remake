// Draft map document — the editor's own working format (not yet a game
// format; game export comes later via the .level.json path).
//
// A draft references library assets by id and places them in world/map pixel
// space. `pos` is where the asset's local pixel (0,0) lands, so recreating a
// source map means pos = asset.origin. Draw order is by world anchor Y
// (pos.y + anchor.y), matching the game's projected-Y sprite sort.

export interface Placement {
  /** library asset id */
  asset: string;
  /** world position of the asset's local (0,0) pixel */
  pos: [number, number];
  /** uniform scale factor, default 1 */
  scale?: number;
  /** horizontal mirror, default false */
  flip_x?: boolean;
}

export interface WallRun {
  /** library asset id of a spline-segment wall piece */
  asset: string;
  /** spline control points in world coords (piecewise linear for now) */
  points: [number, number][];
  /** stamp spacing as a fraction of the segment width, default 0.55 */
  spacing?: number;
}

export interface MapDraft {
  version: 1;
  name: string;
  /**
   * map dimensions in pixels. Editing happens on an infinite canvas; this is
   * computed (content bounds + margin, user-overridable) when the draft is
   * saved, and rendered only as a boundary guide when reopening.
   */
  size: [number, number];
  /** solid background fill until terrain painting exists */
  background_color?: string;
  placements: Placement[];
  /** wall runs stitched from spline-segment assets */
  walls?: WallRun[];
  notes?: string;
}
