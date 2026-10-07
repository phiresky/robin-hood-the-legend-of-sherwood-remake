"""CPU surface-plane and adjacency scope before a bounded bank prototype."""
import hashlib
import json
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw

R = Path(__file__).resolve().parents[3]
B = R/'level-editor/work/croisement03-refinement'
O = B/'restart2/bank-west-transition-v1'


def inside(point, poly):
    x, y = point
    hit = False
    for a, b in zip(poly, poly[1:]+poly[:1]):
        if (a[1] > y) != (b[1] > y):
            if x < (b[0]-a[0])*(y-a[1])/(b[1]-a[1])+a[0]:
                hit = not hit
    return hit


def main():
    O.mkdir(exist_ok=True)
    assert not (O/'surface-scope.json').exists()
    lp = B/'baseline/Croisement03.rhp.json'
    level = json.loads(lp.read_text())
    source = B/'baseline/covered.png'
    surfaces = {}
    for i in (52, 53, 54, 94, 95, 96, 97):
        points = level['sight_obstacles'][i]['points']
        poly = [[p['x'], p['y']-p['z_top']] for p in points]
        a = np.array([[*p, 1] for p in poly])
        heights = np.array([p['z_top'] for p in points])
        coeff = np.linalg.lstsq(a, heights, rcond=None)[0]
        error = float(np.max(np.abs(a@coeff-heights)))
        assert error < .001
        surfaces[i] = dict(projected_polygon=poly, height_from_native_xy=coeff.tolist(), max_plane_residual=error)
    samples = []
    for y in range(330, 370):
        left = [i for i in (94, 95, 97) if inside((121.5, y+.5), surfaces[i]['projected_polygon'])]
        right = [i for i in (53,) if inside((123.5, y+.5), surfaces[i]['projected_polygon'])]
        heights = {str(i):float(np.dot(surfaces[i]['height_from_native_xy'], [122, y+.5, 1])) for i in left+right}
        samples.append(dict(y=y+.5,left_surfaces=left,right_surfaces=right,edge_heights=heights))
    src = Image.open(source).convert('RGB')
    marked = src.copy()
    draw = ImageDraw.Draw(marked)
    for i, color in [(53,'#ff66cc'),(94,'#55ffff'),(95,'#ffff55'),(97,'#66ff66')]:
        p = surfaces[i]['projected_polygon']
        draw.line([tuple(q) for q in p+[p[0]]], fill=color, width=1)
        draw.text(tuple(p[0]),str(i),fill=color)
    draw.line([(122,332),(122,367)],fill='white',width=1)
    crop=(60,285,225,430)
    sheet=Image.new('RGB',(1320,610),'#222222');d=ImageDraw.Draw(sheet)
    for i,(label,im) in enumerate([('Untouched native surface',src),('53 pink / 94 cyan / 95 gold / 97 green; white authored join',marked)]):
        sheet.paste(im.crop(crop).resize((660,580),Image.Resampling.NEAREST),(i*660,30));d.text((i*660+4,8),label,fill='white')
    sheet.save(O/'surface-adjacency.png')
    record=dict(status='CPU scope ready for coordinator; geometry construction not started',
        source_hashes={str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in (lp,source)},
        surfaces=surfaces, west_join_samples=samples,
        findings=['The previous 7.075px mismatch compared the join endpoint only with obstacle94. Adjacent plateau95 covers the upper west side, so that number alone does not establish a visible void.',
            'All seven top profiles are planar to less than0.001 authored height units; use the variable ramp planes instead of flattening them.',
            'Native elevation lines retain their explicit obstacle references and coordinates. The visual surface neighborhood also includes95/97; it must not rewrite the gameplay line from94 to95.',
            'The source crop shows layered rock shelves and grass. Height discontinuity should initially retain the source-supported step, not be smoothed into an invented continuous ramp.'],
        proposed_candidate=dict(id='croisement03-northwest-bank-full-v1',
            geometry_nodes=[52,53,54], unchanged_context_nodes=[94,95,96,97],
            extent='Complete authored bank and both ramp footprints, with shared x175–340 reviewed crest inserted unchanged.',
            mesh='Separate closed bank/ramp solids with planar source-supported top profiles, visible shelf breaks and inferred closed rear/bottom surfaces. No new collision surfaces.',
            appearance='Own native source only on explicitly traced exposed rock patches. Mixed leaf/ivy domains remain unknown and reserved for separate vegetation ownership; hidden surfaces neutral.',
            firsthit_guards=['Tree02 v7', 'Tree03 exact leaf2787 derivative', 'Tree04–07 accepted source domains', 'Tree01 existing coarse hits'],
            resource_limits=dict(max_model_bytes=8*1024**2,max_round_bytes=32*1024**2,threads=2,min_free_bytes=10*1024**3),
            lane='Coordinator request before any Blender; CPU packet does not reserve lane',
            review='Native camera first, saved actual/solid eight views, top/ramp/neighbor joints, explicit source/unknown ownership.'),
        remaining_before_source_appearance=['Trace visible rock patches against ivy and gold crown overlap; no exclusive mask ownership accepted.', 'Tree01 bark/crown partition remains separate and may not borrow bank RGB.'],
        limits=['This packet does not approve morphology, source ownership, gameplay changes, texture synthesis or publication.'])
    (O/'surface-scope.json').write_text(json.dumps(record,indent=2)+'\n')
    assert sum(p.stat().st_size for p in O.iterdir()) < 2*1024**2
    print(O/'surface-scope.json')


if __name__=='__main__':
    main()
