"""Round-3 grouping proposals (for catalog v4) for the south_gate_walls lane.

- Stair top landing: new component 'south-wall-stair-top-landing' of building-085,
  cut out of the angled run's walk; moves to lincoln-south-wall-stair. The recipe
  builds it when SOUTH_GATE_LANDING_SPLIT=1 (the frozen tooling rejects components
  that the catalog does not yet list, so the split is not saved in the v3 workspace).
- SE curtain: the straight run (084 + 099 southeast-parapet-straight) and the
  short angled junction block that steps the wall walk from z 320 (door turret)
  to z 350 (122 + 100) become two assets.
Writes grouping-proposal.json in the round-3 workspaces and an evidence sheet.
"""
import hashlib
import json
import shutil
from pathlib import Path

from PIL import Image, ImageDraw

R = Path(__file__).resolve().parents[3] / 'level-editor/work/needs-review/lincoln-refinement'
WS = R / 'round-3/assets'
CUT = (1884.0, 1885 + (1884 - 1852) * 18 / 44)
LANDING = [(1863, 1915), (1831, 1901), (1852, 1885), CUT]
WALK_REST = [(1793, 1975), (1863, 1915), CUT, (1896, 1903), (1898, 1898), (1888, 1893), (1943, 1852),
             (1955, 1882), (1916, 1912), (1922, 1914), (1846, 1992)]
STAIR = [(1761, 1962), (1793, 1974), (1863, 1915), (1831, 1902)]
J122 = [(1944, 1853), (1969, 1849.5), (1980, 1854), (1991, 1881), (1956, 1882)]
J100 = [(1959, 1882), (1992, 1881), (1993, 1884), (1959, 1884)]


def sha(p):
    return hashlib.sha256(Path(p).read_bytes()).hexdigest()


def draw(out, polys, box, scale=4):
    img = Image.open(R / 'source-states/covered.png').convert('RGB')
    x0, y0, x1, y1 = box
    crop = img.crop(box).resize(((x1 - x0) * scale, (y1 - y0) * scale), Image.NEAREST)
    marked = crop.copy()
    d = ImageDraw.Draw(marked)
    for poly, z, col, label in polys:
        pts = [((x - x0 + .5) * scale, (y - z - y0 + .5) * scale) for x, y in poly]
        d.line(pts + pts[:1], fill=col, width=2)
        d.text(pts[0], label, fill=col)
    sheet = Image.new('RGB', (crop.width * 2 + 8, crop.height), (20, 20, 20))
    sheet.paste(crop, (0, 0))
    sheet.paste(marked, (crop.width + 8, 0))
    sheet.save(out)


def main():
    ev1 = R / 'scratch/south_gate_walls-r3/stair-top-landing.png'
    draw(ev1, [(LANDING, 320, (255, 255, 0), 'top landing (z 320)'), (WALK_REST, 320, (0, 255, 255), 'angle walk'),
               (STAIR, 320, (255, 0, 255), 'stair top edge')], (1740, 1500, 1970, 1700))
    ev2 = R / 'scratch/south_gate_walls-r3/southeast-two-pieces.png'
    draw(ev2, [(J122, 350, (255, 255, 0), '122 junction'), (J100, 360, (255, 128, 0), '100'),
               ([(1994, 1883), (2329, 1843), (2320, 1813), (1981, 1854)], 380, (0, 255, 255), 'straight run')],
         (1930, 1440, 2350, 1640), scale=3)
    props = {
        'lincoln-south-curtain-wall-angle': {
            'components': [
                {'source_node': 'building-085', 'projection_component': 'south-angle-curtain-walk',
                 'proposed_asset_id': 'lincoln-south-curtain-wall-angle', 'proposed_name': 'Southern curtain angled run'},
                {'source_node': 'building-085', 'projection_component': 'south-wall-stair-top-landing',
                 'proposed_asset_id': 'lincoln-south-wall-stair', 'proposed_name': 'Wall stair with top landing',
                 'new_component': True,
                 'polygon_native_xy': [list(p) for p in LANDING], 'z': [218.0, 320.0],
                 'build': 'refine_south_gate.py with SOUTH_GATE_LANDING_SPLIT=1 once the catalog lists the component'},
                {'source_node': 'building-104', 'projection_component': 'wall-fill-below-door-turret',
                 'proposed_asset_id': 'lincoln-south-curtain-wall-angle', 'proposed_name': 'Southern curtain angled run'},
                {'source_node': 'building-127', 'projection_component': 'wall-fill-below-door-turret',
                 'proposed_asset_id': 'lincoln-south-curtain-wall-angle', 'proposed_name': 'Southern curtain angled run'}],
            'whole_nodes': [{'source_node': 'building-109', 'proposed_asset_id': 'lincoln-south-curtain-wall-angle',
                             'proposed_name': 'Southern curtain angled run'}],
            'rationale': 'User on the stair: "missing the upper ending of the stair where you can stand on after going up"; '
                         'on the angle run: "this is where that upper thing on the stairs is instead of being part of the stairs". '
                         'The top landing (between the stair head and landing parapets 107/108, walk height z 320) is cut out of '
                         '085 south-angle-curtain-walk along the native walk edge and moves to lincoln-south-wall-stair. '
                         'Union of the two pieces equals the current walk component; both are closed prisms.',
            'evidence': [ev1]},
        'lincoln-south-wall-stair': {
            'components': [{'source_node': 'building-085', 'projection_component': 'south-wall-stair-top-landing',
                            'proposed_asset_id': 'lincoln-south-wall-stair', 'proposed_name': 'Wall stair with top landing',
                            'new_component': True}],
            'whole_nodes': [{'source_node': n, 'proposed_asset_id': 'lincoln-south-wall-stair',
                             'proposed_name': 'Wall stair with top landing'} for n in ('building-106', 'building-107', 'building-108')],
            'rationale': 'Receives the top landing from lincoln-south-curtain-wall-angle (user: "missing the upper ending of the stair"); '
                         'the landing parapets 107/108 then stand on the stair asset\'s own landing.',
            'evidence': [ev1]},
        'lincoln-southeast-curtain-wall': {
            'components': [{'source_node': 'building-099', 'projection_component': 'southeast-parapet-straight',
                            'proposed_asset_id': 'lincoln-southeast-curtain-wall', 'proposed_name': 'Southeastern curtain wall straight run'}],
            'whole_nodes': [
                {'source_node': 'building-084', 'proposed_asset_id': 'lincoln-southeast-curtain-wall',
                 'proposed_name': 'Southeastern curtain wall straight run'},
                {'source_node': 'building-122', 'proposed_asset_id': 'lincoln-south-wall-walk-step',
                 'proposed_name': 'Curtain wall walk step / angled junction (z 320 to 350)'},
                {'source_node': 'building-100', 'proposed_asset_id': 'lincoln-south-wall-walk-step',
                 'proposed_name': 'Curtain wall walk step / angled junction (z 320 to 350)'}],
            'rationale': 'User: "but should be two separate pieces?". The asset visibly consists of the long straight crenellated '
                         'run and a short angled junction block at its west end (122 with sloped top 320->350 and parapet 100) '
                         'that steps the wall walk up from the door turret level to the SE curtain level. The junction becomes '
                         'its own modular piece so straight runs can be placed without it. No mesh split is needed: the two '
                         'pieces are already separate source nodes.',
            'evidence': [ev2]},
    }
    for asset, p in props.items():
        ws = WS / asset
        (ws / 'inspection').mkdir(exist_ok=True)
        ev = []
        for e in p['evidence']:
            dst = ws / 'inspection' / Path(e).name
            shutil.copyfile(e, dst)
            ev.append('inspection/' + Path(e).name)
        doc = {'version': 1, 'asset_id': asset, 'components': p['components'], 'whole_nodes': p['whole_nodes'],
               'rationale': p['rationale'], 'evidence': ev,
               'evidence_sha256': {e: sha(ws / e) for e in ev}, 'catalog_target': 'v4'}
        (ws / 'grouping-proposal.json').write_text(json.dumps(doc, indent=1) + '\n')
        print(asset, 'proposal written')


if __name__ == '__main__':
    main()
