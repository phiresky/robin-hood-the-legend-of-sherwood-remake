"""Catalog-v2 grouping proposals for the south gate/wall lane (offline).

Writes <round-2 workspace>/grouping-proposal.json for every lane asset and a
colour-coded evidence sheet (inspection/modular-split.png) that overlays the
projected outlines of each proposed modular asset on covered.png. Pieces
named here are the projection_component meshes written by refine_south_gate.py.
"""
import hashlib
import json
import shutil
from pathlib import Path

from PIL import Image, ImageDraw

HERE = Path(__file__).resolve().parent
R = HERE.parents[2] / 'level-editor/work/needs-review/lincoln-refinement'
WS = R / 'round-2/assets'

# proposed asset -> (name, [(source_node, component or None)])
PROPOSED = {
    'lincoln-south-gatehouse': ('Southern gatehouse: gate block, east round tower and cone stair turret', [
        (n, None) for n in ('building-088', 'building-089', 'building-090', 'building-092', 'building-112',
                            'building-113', 'building-114', 'building-115', 'building-116', 'building-125',
                            'building-087', 'building-111', 'building-120', 'building-126',
                            'building-117', 'building-118', 'building-119', 'building-124', 'building-128',
                            'building-129')]),
    'lincoln-south-gatehouse-west-tower': ('Southern gatehouse western round tower',
                                           [('building-091', None), ('building-121', None)]),
    'lincoln-south-gate-drawbridge': ('Southern gate drawbridge and footbridge',
                                      [('building-058', 'footbridge-deck'), ('building-058', 'footbridge-rails'),
                                       ('building-059', None), ('building-457', None)]),
    'lincoln-south-curtain-wall-west': ('Southern curtain wall west run (with projecting middle bay)',
                                        [('building-375', None), ('building-376', None)]),
    'lincoln-south-curtain-wall-central': ('Southern curtain wall straight run', [
        ('building-086', None), ('building-110', 'south-central-parapet-straight')]),
    'lincoln-south-angle-bastion': ('Half-round angle bastion (corner piece)', [
        ('building-110', 'south-angle-bastion-parapet'), ('building-085', 'south-angle-bastion-body')]),
    'lincoln-south-wall-stair': ('Wall stair (steps and landing parapets)', [
        ('building-106', None), ('building-107', None), ('building-108', None)]),
    'lincoln-south-curtain-wall-angle': ('Southern curtain angled run to the door turret', [
        ('building-109', None), ('building-085', 'south-angle-curtain-walk'),
        ('building-104', 'wall-fill-below-door-turret'), ('building-127', 'wall-fill-below-door-turret')]),
    'lincoln-south-wall-cone-turret': ('Wall-top door turret (pyramid roof, placeable on any wall walk)', [
        ('building-101', None), ('building-102', None), ('building-103', None), ('building-105', None),
        ('building-104', 'door-turret-wall'), ('building-127', 'door-turret-wall')]),
    'lincoln-southeast-curtain-wall': ('Southeastern curtain wall straight run', [
        ('building-084', None), ('building-099', 'southeast-parapet-straight'),
        ('building-122', None), ('building-100', None)]),
    'lincoln-southeast-corner-turret': ('Southeastern round corner turret (corner piece)', [
        ('building-083', 'southeast-corner-turret-body'), ('building-099', 'southeast-corner-turret-parapet')]),
    'lincoln-east-curtain-wall-lower': ('(east lane) eastern curtain wall lower run: gains its walkway body', [
        ('building-083', 'east-curtain-walk')]),
}

CURRENT = {
    'lincoln-south-gate-drawbridge': ['building-058', 'building-059', 'building-457'],
    'lincoln-south-gatehouse-west-tower': ['building-091', 'building-121'],
    'lincoln-south-gatehouse-arch': ['building-088', 'building-089', 'building-090', 'building-092', 'building-112',
                                     'building-113', 'building-114', 'building-115', 'building-116', 'building-125'],
    'lincoln-south-gatehouse-east-tower': ['building-087', 'building-111', 'building-120', 'building-126'],
    'lincoln-south-gatehouse-cone-turret': ['building-117', 'building-118', 'building-119', 'building-124',
                                            'building-128', 'building-129'],
    'lincoln-south-curtain-wall-west': ['building-375', 'building-376'],
    'lincoln-south-curtain-wall-central': ['building-086', 'building-110'],
    'lincoln-south-wall-stair': ['building-085', 'building-106', 'building-107', 'building-108', 'building-109'],
    'lincoln-south-wall-cone-turret': ['building-100', 'building-101', 'building-102', 'building-103', 'building-104',
                                       'building-105', 'building-122', 'building-127'],
    'lincoln-southeast-curtain-wall': ['building-084', 'building-099'],
    'lincoln-southeast-corner-turret': ['building-083'],
}

RATIONALE = {
    'lincoln-south-gate-drawbridge': 'Unchanged grouping; the drawbridge leaf, landing and footbridge form one stateful gate prop.',
    'lincoln-south-gatehouse-west-tower': 'Unchanged grouping (approved without comment).',
    'lincoln-south-gatehouse-arch': 'User on the approved east tower: "should either be merged with above or have some geometry of above"; on the approved cone turret: "should be merged with above". Gate block, east tower and cone turret become one gatehouse asset; this also removes the need to split walkway slab 112, which spans the gate block and the east tower interior.',
    'lincoln-south-gatehouse-east-tower': 'User: "should either be merged with above or have some geometry of above" -> merged into lincoln-south-gatehouse (approved model unchanged).',
    'lincoln-south-gatehouse-cone-turret': 'User: "should be merged with above" -> merged into lincoln-south-gatehouse (approved model unchanged).',
    'lincoln-south-curtain-wall-west': 'Unchanged grouping; the projecting middle bay stays part of the run (user asked for it to project and carry battlements).',
    'lincoln-south-curtain-wall-central': 'User: "should be only straight part with corner thing separate so we can dynamically create walls". The straight run keeps 086 and the straight parapet component of 110; the half-round angle bastion parapet goes to the new corner piece lincoln-south-angle-bastion.',
    'lincoln-south-wall-stair': 'User: "make stairs separate and part of this should be part of corner circle as mentioned above". The steps (106) and their landing parapets (107/108) become the stair asset; the half-round bastion body component of 085 joins lincoln-south-angle-bastion; the angled wall skin 109 and the angled walk component of 085 form the new straight run lincoln-south-curtain-wall-angle.',
    'lincoln-south-wall-cone-turret': 'User: "tower door thingy should be separate so we can prop it onto any wall". The roofed door turret is cut at the wall-walk level (z 320): its walls above the walk (door-turret-wall components of 104/127), floor 105, block 101 and roof 102/103 form a placeable prop; the wall fill below the turret joins the angled run, and the junction ramp/parapet 122/100 join the southeast straight run.',
    'lincoln-southeast-curtain-wall': 'User: "again - circle piece should be separate". The straight parapet component of 099 stays with 084 (plus junction pieces 122/100 from the old turret asset); the corner-turret parapet ring component goes to lincoln-southeast-corner-turret.',
    'lincoln-southeast-corner-turret': 'User: "again about the corner/circle". The corner piece is the round turret body component of 083 plus the ring parapet component of 099. The east-curtain walk component of 083 is the walkway the east lane was asked to include ("walkway should be part of curtain wall" on lincoln-east-curtain-wall-lower): proposed to move to that asset.',
}

COLORS = {k: c for k, c in zip(PROPOSED, [(255, 80, 80), (255, 170, 0), (255, 255, 0), (120, 255, 0), (0, 255, 160),
                                           (0, 220, 255), (80, 120, 255), (190, 90, 255), (255, 80, 220),
                                           (255, 255, 255), (255, 140, 140), (140, 255, 255)])}


def sha(p):
    return hashlib.sha256(Path(p).read_bytes()).hexdigest()


def owner_of(node, comp):
    hits = [a for a, (_, parts) in PROPOSED.items() if (node, comp) in parts]
    if len(hits) != 1:
        raise ValueError(f'{node}/{comp} proposed {len(hits)} times')
    return hits[0]


def evidence():
    """Overlay the projected first-hit silhouettes of every proposed asset."""
    import numpy as np
    src = Image.open(R / 'source-states/covered.png').convert('RGB')
    box = (740, 1290, 2500, 2010)
    base = np.array(src.crop(box)).astype(float) * 0.45
    for asset, comps in _component_hits().items():
        for (node, comp), mask in comps.items():
            col = COLORS[owner_of(node, comp)]
            base[mask] = base[mask] * 0.35 + np.array(col) * 0.65
    img = Image.fromarray(base.astype('uint8'))
    d = ImageDraw.Draw(img)
    for k, (a, (name, _)) in enumerate(PROPOSED.items()):
        d.rectangle([6, 6 + k * 14, 16, 16 + k * 14], fill=COLORS[a])
        d.text((20, 5 + k * 14), a, fill=COLORS[a])
    out = R / 'scratch/south_gate_walls-r2/modular-split.png'
    img.save(out)
    return out


def _component_hits():
    """Per-pixel owner from each workspace's source-camera hit map (node level),
    refined to components by the per-component hit maps written by the ray tool."""
    import numpy as np
    result = {}
    for asset in CURRENT:
        ws = WS / asset
        path = ws / 'inspection/coverage-hits.npz'
        meta_path = ws / 'inspection/coverage-hits.json'
        if not path.exists():
            continue
        h = np.load(path)
        meta = json.loads(meta_path.read_text())
        x0, y0, x1, y1 = meta['box']
        comps = {}
        cmap = h['comp'] if 'comp' in h.files else None
        for k, node in enumerate(meta['nodes']):
            own = (h['cls'] == 1) & (h['node'] == k)
            names = meta.get('components', {}).get(node) or [None]
            for ci, comp in enumerate(names):
                m = own & (cmap == ci) if (cmap is not None and comp is not None) else own
                full = np.zeros((2010 - 1290, 2500 - 740), bool)
                ys, xs = np.where(m)
                ys, xs = ys + y0 - 1290, xs + x0 - 740
                ok = (ys >= 0) & (ys < full.shape[0]) & (xs >= 0) & (xs < full.shape[1])
                full[ys[ok], xs[ok]] = True
                comps[(node, comp)] = full
        result[asset] = comps
    return result


def main():
    ev = evidence()
    for asset, nodes in CURRENT.items():
        ws = WS / asset
        (ws / 'inspection').mkdir(exist_ok=True)
        dst = ws / 'inspection/modular-split.png'
        shutil.copyfile(ev, dst)
        comps, whole = [], []
        for node in nodes:
            parts = [(a, c) for a, (_, ps) in PROPOSED.items() for n, c in ps if n == node]
            if any(c is not None for _, c in parts):
                for a, c in parts:
                    comps.append({'source_node': node, 'projection_component': c,
                                  'proposed_asset_id': a, 'proposed_name': PROPOSED[a][0]})
            else:
                (a, _), = parts
                whole.append({'source_node': node, 'proposed_asset_id': a, 'proposed_name': PROPOSED[a][0]})
        doc = {'version': 1, 'asset_id': asset, 'components': comps, 'whole_nodes': whole,
               'rationale': RATIONALE[asset],
               'evidence': ['inspection/modular-split.png'] + (['inspection/coverage-audit.png']
                                                               if (ws / 'inspection/coverage-audit.png').exists() else []),
               'evidence_sha256': {'inspection/modular-split.png': sha(dst)}}
        (ws / 'grouping-proposal.json').write_text(json.dumps(doc, indent=1) + '\n')
        print(asset, len(comps), 'components', len(whole), 'whole nodes')


if __name__ == '__main__':
    main()
