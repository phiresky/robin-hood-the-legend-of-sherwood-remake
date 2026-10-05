import ClipperLib from "clipper-lib";
import type { MultiPolygon } from "polygon-clipping";

/** Close cracks within the deformation precision before rounding navigation to whole pixels. */
export function closeNavigationSeams(regions: MultiPolygon, reach = 1 / 512): MultiPolygon {
  const scale = 1048576;
  const paths = regions.flatMap((region) =>
    region.map((ring, index) => {
      const path = ring.map(([x, y]) => ({ X: Math.round(x * scale), Y: Math.round(y * scale) }));
      if (ClipperLib.Clipper.Orientation(path) !== (index === 0)) path.reverse();
      return path;
    }),
  );
  const expand = new ClipperLib.ClipperOffset();
  expand.AddPaths(paths, ClipperLib.JoinType.jtMiter, ClipperLib.EndType.etClosedPolygon);
  const expanded: ClipperLib.Paths = [];
  expand.Execute(expanded, scale * reach);
  const contract = new ClipperLib.ClipperOffset();
  contract.AddPaths(expanded, ClipperLib.JoinType.jtMiter, ClipperLib.EndType.etClosedPolygon);
  const tree = new ClipperLib.PolyTree();
  contract.Execute(tree, -scale * reach);
  const ring = (node: ClipperLib.PolyNode) => {
    const points = node.Contour().map(({ X, Y }): [number, number] => [X / scale, Y / scale]);
    return [...points, points[0]!];
  };
  const result: MultiPolygon = [];
  const visit = (parent: ClipperLib.PolyNode) => {
    for (const node of parent.Childs()) {
      if (!node.IsHole())
        result.push([
          ring(node),
          ...node
            .Childs()
            .filter((n) => n.IsHole())
            .map(ring),
        ]);
      visit(node);
    }
  };
  visit(tree);
  return result;
}
