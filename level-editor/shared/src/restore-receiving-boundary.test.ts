import test from "node:test";
import assert from "node:assert/strict";
import clipping, { type MultiPolygon } from "polygon-clipping";
import type { Point } from "./level.ts";
import { restoreReceivingBoundary } from "./restore-receiving-boundary.ts";

test("half-grid endpoint candidates use the interior of their rounding cell", () => {
  const source: Point[] = [
    [910.2813222477323, 1346.5067106699871],
    [976.4406082501519, 1332.5480393251266],
    [981.8100804105238, 1357.997484731754],
    [945.4592418670654, 1366.1088275909424],
    [929.4592714309692, 1343.1088275909424],
    [921.227822303772, 1344.2542276382446],
    [941.125862121582, 1372.4422283172607],
    [982.9398568617387, 1363.352235302785],
    [992.9558872647501, 1410.8247635735902],
    [926.7966012623306, 1424.7834349184504],
  ];
  const boundary: Point[] = [
    [929, 1343],
    [976, 1333],
    [982, 1358],
    [945, 1366],
  ];
  const restored = restoreReceivingBoundary(boundary, [[source]]);
  assert.ok(restored);
  assert.deepEqual(clipping.difference([restored], [source]), []);
  assert.deepEqual(
    clipping.xor([restored.map(([x, y]): Point => [Math.round(x), Math.round(y)])], [boundary]),
    [],
  );
  assert.ok(restored.some((point) => point[0] === source[1]![0] && point[1] === source[1]![1]));
});

test("split movement regions retain source edges without filling collision cuts", () => {
  const source: Point[] = [
    [978.7127251485637, 1339.149050189603],
    [1023.0824813075121, 1336.0692473360036],
    [1037.224175453186, 1356.919231414795],
    [997.1248989105225, 1366.0964374542236],
    [999.1218433380127, 1370.8178033828735],
    [1048.2661209106445, 1359.4047565460205],
    [1032.4182026959515, 1335.4212342779754],
    [1115.1840292112634, 1329.6762753070668],
    [1139.5202300385881, 1367.623682803864],
    [1003.0489259758884, 1377.0964576864008],
  ];
  const boundary: Point[] = [
    [979, 1339],
    [1023, 1336],
    [1037, 1357],
    [997, 1366],
    [999, 1370],
  ];
  const restored = restoreReceivingBoundary(boundary, [[source]]);
  assert.ok(restored);
  assert.ok(restored.some((p) => p[0] === source[0]![0] && p[1] === source[0]![1]));
  assert.deepEqual(clipping.difference([restored], [source]), []);
  assert.deepEqual(
    clipping.xor([restored.map(([x, y]): Point => [Math.round(x), Math.round(y)])], [boundary]),
    [],
  );
  // Neither an unrelated contour nor two competing owners can restore a region.
  const distant: MultiPolygon = [[source.map(([x, y]) => [x + 10, y])]];
  assert.equal(restoreReceivingBoundary(boundary, distant), undefined);
  assert.equal(restoreReceivingBoundary(boundary, [[source], [source]]), undefined);
});
