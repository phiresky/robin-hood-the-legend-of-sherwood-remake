"""Measure native branch cross sections without treating foliage color as bark."""
import hashlib
import json
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[3]
R = ROOT/'level-editor/work/croisement01-refinement/restart2'
OUT = R/'tree14-upper-branch-profiles-v1'
OUT.mkdir(exist_ok=False)
prep = json.loads((R/'tree14-source-preparation-v1/preparation.json').read_text())
native_path = R.parent/'baseline/masks/000014.png'
mask = np.asarray(Image.open(native_path).convert('L')) > 0
profiles = {}
for name, trace in prep['trace_screen_coordinates'].items():
    if name.startswith('root_') or name == 'main_stem':
        continue
    xy = np.array([r[:2] for r in trace], dtype=float)
    samples = []
    cumulative = 0.
    for a, b in zip(xy[:-1], xy[1:]):
        delta = b-a
        length = np.linalg.norm(delta)
        tangent = delta/length
        normal = np.array([-tangent[1], tangent[0]])
        for t in np.linspace(0, 1, max(2, int(length/2)), endpoint=False):
            center = a+delta*t
            offsets = np.arange(-35, 35.001, .25)
            positions = center+offsets[:,None]*normal
            local = np.floor(positions-[689,122]).astype(int)
            inside = (local[:,0]>=0)&(local[:,0]<227)&(local[:,1]>=0)&(local[:,1]<317)
            occupied = np.zeros(len(offsets),bool)
            occupied[inside] = mask[local[inside,1],local[inside,0]]
            hit = np.flatnonzero(occupied)
            if not len(hit):
                continue
            runs = np.split(hit,np.flatnonzero(np.diff(hit)>1)+1)
            chosen = min(runs,key=lambda v: min(abs(offsets[v])))
            if min(abs(offsets[chosen])) > 6:
                continue
            lo,hi = offsets[chosen[0]]-.125, offsets[chosen[-1]]+.125
            shifted = center+normal*(lo+hi)/2
            samples.append(dict(x=float(shifted[0]),y=float(shifted[1]),radius=float((hi-lo)/2),
                                along=cumulative+t*length,normal=normal.tolist(),bounds=[lo,hi]))
        cumulative += length
    assert len(samples)>5, name
    # Smooth pixel-grid chatter, without changing which native run was measured.
    radii = np.array([r['radius'] for r in samples])
    smooth = np.convolve(np.pad(radii,(2,2),mode='edge'),[.1,.2,.4,.2,.1],mode='valid')
    for row,radius in zip(samples,smooth):
        row['smoothed_radius'] = float(radius)
    profiles[name] = dict(length=cumulative,samples=samples,
                          tip=[*trace[-1][:2], min(.4,trace[-1][2])],
                          limitations='Nearest contiguous native silhouette run across traced tangent. Junctions require 3D continuity review; no pixel color ownership inferred.')
source = Image.open(R.parent/'baseline/covered.png').convert('RGB').crop((680,110,925,315))
source = source.resize((1225,1025),Image.Resampling.NEAREST)
draw = ImageDraw.Draw(source)
for name,profile in profiles.items():
    points = [((row['x']-680)*5,(row['y']-110)*5) for row in profile['samples']]
    draw.line(points,fill='cyan',width=1)
    for row in profile['samples'][::4]:
        c=np.array([row['x'],row['y']]);n=np.array(row['normal']);rad=row['smoothed_radius']
        endpoints=[tuple((p-np.array([680,110]))*5) for p in (c-n*rad,c+n*rad)]
        draw.line(endpoints,fill='magenta',width=1)
source.save(OUT/'measured-cross-sections.png')
report=dict(status='SOURCE_MEASUREMENTS_NOT_GEOMETRY_APPROVAL',native_sha256=hashlib.sha256(native_path.read_bytes()).hexdigest(),
            profiles=profiles,source_ownership='Use separately reviewed uncontested bark domain; silhouette measurements do not authorize overlapping foliage colors.')
(OUT/'profiles.json').write_text(json.dumps(report,indent=2)+'\n')
print({k:len(v['samples']) for k,v in profiles.items()})
