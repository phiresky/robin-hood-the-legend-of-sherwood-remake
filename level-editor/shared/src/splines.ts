/** Editable paths in game coordinates. Wall geometry is derived from pinned assets. */
export interface LevelSpline {
  id: string;
  name: string;
  kind: "river" | "road" | "wall";
  points: [number, number, number][];
  closed: boolean;
  /** False joins path controls with straight segments; omitted retains smooth curves. */
  curved?: boolean;
  width: number;
  /** Optional widths at control points; interpolated continuously along each section. */
  pointWidths?: number[];
  /** Road height adjustments above the sampled ground, in game pixels. */
  pointHeightOffsets?: number[];
  /** Material at each control point; sections blend their two endpoint designs. */
  pointMaterials?: string[];
  /** Interpolated control-point appearance retained when inserting a point in a transition. */
  pointMaterialMixes?: (Record<string, number> | null)[];
  /** Non-destructive riverbed modifier. Depth is in game pixels; slope is rise/run. */
  channel?: { enabled: boolean; bedDepth: number; bankSlope: number };
  repeatLength: number;
  /** Surface tile embedded in the document so save/reload needs no extra file grant. */
  texture?: string;
  /** Wall asset ID in Level3D.assetSources. */
  asset?: string;
  axis?: "x" | "y";
  sourceAngle?: number;
  /** Prepared straight strips retain their section shape, including rails and posts. */
  sourceStraight?: boolean;
  /** Reflect the cross-section so the parapet can face the exterior. */
  flipCrossSection?: boolean;
  /** Optional matching tower from the wall preset, placed at qualifying turns. */
  cornerAsset?: string;
  cornerMinAngle?: number;
  cornerScale?: number;
  cornerWidthScale?: number;
  cornerRotation?: number;
  /** Control point indices explicitly left as continuous wall. */
  cornerDisabled?: number[];
  /** Retained interval along the source model, useful for trimming fixed end caps. */
  sourceStart?: number;
  sourceEnd?: number;
}
