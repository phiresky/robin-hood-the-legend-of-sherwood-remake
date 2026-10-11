"""Measure historical source-edge samples against current installed static GLBs."""
import json
import argparse
import hashlib
from pathlib import Path
from collections import Counter
import restart14_butterfly_canopy22_audit as core

BASE = core.W
OUT = BASE / 'static-terrain-closure-oct11-v1/current-first-hit-v1'
LEDGER = BASE / 'restart6-source-coverage/residual-role-ledger-v9.json'

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--subpixel", action="store_true")
    parser.add_argument("--context", action="store_true")
    args = parser.parse_args()
    output = OUT.with_name("current-context-subpixels-v1" if args.context else "current-aa-subpixels-v1") if args.subpixel else OUT
    data = json.loads(LEDGER.read_text())
    points = {}
    for group, row in data['groups'].items():
        for xy in row['coordinates']:
            points.setdefault(tuple(xy), []).append(group)
    assert len(points) == 487
    mem = dict(line.split(':', 1) for line in Path('/proc/meminfo').read_text().splitlines())
    assert int(mem['MemAvailable'].split()[0]) * 1024 >= 6 * 1024**3
    rays = [dict(pixel=list(xy), screen=[xy[0]+.5, xy[1]+.5], historical_groups=groups, hits=[]) for xy, groups in sorted(points.items())]
    if args.subpixel:
        groups = ['other_context_and_outside_inventory'] if args.context else ['bank420_subpixel_contact_no_new_geometry', 'sampled_root_boundary_AA_no_new_volume', 'single_dark_native_edge_accepted_limitation']
        selected = [r for r in rays if any(g in r['historical_groups'] for g in groups)]
        rays = [dict(pixel=r['pixel'], screen=[r['pixel'][0]+(x+.5)/8, r['pixel'][1]+(y+.5)/8], historical_groups=r['historical_groups'], hits=[]) for r in selected for y in range(8) for x in range(8)]
    map_path = core.LIB / 'scenes/croisement02.rhlos-map.json'
    map_sha = core.sha(map_path)
    def finish(rays, records, mp):
        summary = {}
        for group in data['groups']:
            selected = [r for r in rays if group in r['historical_groups']]
            summary[group] = dict(samples=len(selected), first_hits=dict(Counter(r['first_hit']['asset'] if r['first_hit'] else '<none>' for r in selected)))
        result = dict(status='MEASURED_CURRENT_STATIC_FIRST_HITS', map_sha256=map_sha, ledger_sha256=core.sha(LEDGER), samples=len(rays), summary=summary, records=records, rays=rays, limits=['Exact source pixel centers and bilinear level0 alpha; subpixel raster antialias/mips not modeled.', 'Static placements and initial visibility only; native FX and patch controllers excluded.', 'First-hit ownership is measurement, not authorization to change source roles or geometry.'])
        output.mkdir(parents=True, exist_ok=False)
        (output/'report.json').write_text(json.dumps(result, indent=2)+'\n')
        print(json.dumps(summary, indent=2))
    core.main(ray_records=rays, postprocess=finish, output=output, minimum_free_bytes=10*1024**3, output_limit_bytes=100*1024**2, expected_map_sha256=map_sha)

if __name__ == '__main__':
    main()
