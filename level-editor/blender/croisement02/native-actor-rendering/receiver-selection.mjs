/** Convert explicit evaluated game-world terrain triangles to camera-ray support. */
export function projectedTerrainReceivers(evaluated, footprint) {
  if (!Array.isArray(evaluated) || !Array.isArray(footprint) || footprint.length !== 4 ||
      !footprint.every(Number.isFinite) || footprint[0] >= footprint[2] || footprint[1] >= footprint[3])
    throw Error('Invalid evaluated terrain or shadow footprint');
  const ids = new Set(), selected = [];
  for (const row of evaluated) {
    if (typeof row.id !== 'string' || ids.has(row.id) || !Array.isArray(row.points) ||
        row.points.length !== 3 || row.points.some(p => !Array.isArray(p) || p.length !== 3 || !p.every(Number.isFinite)))
      throw Error('Invalid or duplicate evaluated receiver');
    ids.add(row.id);
    const points = row.points.map(([x,y,z]) => [x,y-z,z]);
    const xs = points.map(p=>p[0]), ys = points.map(p=>p[1]);
    if (Math.max(...xs) <= footprint[0] || Math.min(...xs) >= footprint[2] ||
        Math.max(...ys) <= footprint[1] || Math.min(...ys) >= footprint[3]) continue;
    selected.push({id:row.id,points});
  }
  // Selection preserves all intersecting evaluated surfaces. The projector must
  // reject overlapping receivers rather than choosing an invented top surface.
  return selected;
}
