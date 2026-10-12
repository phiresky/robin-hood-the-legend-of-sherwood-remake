"""Private whole-mass source proposals for bank53; no geometry or ownership mutation."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[3]
BASE = ROOT / 'level-editor/work/croisement03-refinement'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    source_path = BASE / 'baseline/covered.png'
    render_path = BASE / 'restart2/bank-main-masses-source-v1/native-foreground-context-v1/native-phase00.png'
    marks_path = BASE / 'restart2/bank-morphology-trace-v1/recipe.json'
    old_geometry_path = BASE / 'restart2/bank-continuous-strata-plan-v1/geometry.json'
    source = Image.open(source_path).convert('RGB')
    old = json.loads(marks_path.read_text())
    masses = [
        {'id': 'A-upper-rounded-cap', 'color': '#00ffff',
         'outline': [[151, 267], [158, 260], [169, 257], [181, 261], [193, 268], [202, 276], [207, 282], [202, 288], [188, 292], [173, 291], [159, 287], [151, 278]],
         'visible_shoulders': ['Rounded upper shoulder and right cheek; broad face extending above the old narrow band.'],
         'uncertain_edges': ['Left edge intersects moss/dark vegetation; bottom joins the broad mass below around y290..298.'],
         'edge_uncertainty_px': 3, 'status': 'Whole visible face proposal; bank52/53 top attachment needs receiver check'},
        {'id': 'B-broad-upper-ledged-mass', 'color': '#ff9944',
         'outline': [[125, 299], [132, 292], [144, 288], [158, 290], [176, 292], [194, 297], [202, 304], [207, 314], [205, 323], [197, 329], [181, 331], [163, 329], [148, 325], [137, 323], [128, 315]],
         'visible_shoulders': ['Wide moss-covered top, rounded left corner, bright layered front and right shoulder turn toward dark cleft.'],
         'uncertain_edges': ['Upper left includes vegetation/moss; lower ledge may share a fracture with the transition below.'],
         'edge_uncertainty_px': 3, 'status': 'Primary complete exposed boulder proposal'},
        {'id': 'C-middle-recessed-transition', 'color': '#ff55ff',
         'outline': [[153, 333], [164, 330], [178, 332], [186, 337], [186, 346], [178, 351], [166, 349], [155, 344]],
         'visible_shoulders': ['Small exposed central rock face beneath upper overhang, with right light edge and darker left.'],
         'uncertain_edges': ['Both side continuations vanish into dark/green material; this may be a recessed part of B or D rather than an independent boulder.'],
         'edge_uncertainty_px': 4, 'status': 'Partial face; do not invent an independent full-width shelf'},
        {'id': 'D-broad-lower-ledge', 'color': '#55aaff',
         'outline': [[128, 354], [138, 348], [154, 347], [172, 350], [185, 351], [196, 357], [201, 366], [196, 374], [180, 378], [165, 381], [149, 382], [135, 377], [127, 367]],
         'visible_shoulders': ['Broad top slope, round left nose, striped but coherent front face and right shoulder.'],
         'uncertain_edges': ['Left/top surface has heavy green coverage; pale bands alone do not fix rock thickness.'],
         'edge_uncertainty_px': 3, 'status': 'Primary complete exposed boulder proposal'},
        {'id': 'E-rounded-basal-shoulder', 'color': '#ffffff',
         'outline': [[148, 385], [157, 379], [171, 377], [186, 380], [198, 382], [207, 389], [207, 397], [199, 405], [185, 404], [176, 401], [163, 405], [149, 405], [143, 397]],
         'visible_shoulders': ['Visible rounded lower gray/brown mass and left descending shoulder beneath the lower ledge.'],
         'uncertain_edges': ['Lower/right boundary is covered by ivy; polygon closure through green material is inferred for reconstruction scope only.'],
         'edge_uncertainty_px': 4, 'status': 'Whole exposed mass proposal with obscured basal closure'},
    ]
    gaps = [
        {'id': 'right-cleft', 'points': [[210, 290], [211, 311], [216, 327], [208, 344], [205, 357], [206, 374]],
         'role': 'Dark recess with visible shoulder turns; not a through-hole or new black material assignment.'},
        {'id': 'under-upper-overhang', 'points': [[146, 330], [156, 332], [169, 332], [185, 332], [198, 329]],
         'role': 'Local dark underside/recess. Depth and exact separation remain inferred.'},
        {'id': 'lower-joint', 'points': [[139, 382], [151, 385], [164, 382], [179, 379], [194, 377]],
         'role': 'Irregular transition between lower ledge and basal shoulder; partial vegetation coverage.'},
    ]
    exclusions = [
        {'box': [102, 260, 148, 287], 'reason': 'Adjacent left cap is context; do not silently absorb into bank53.'},
        {'box': [211, 270, 244, 316], 'reason': 'Right rock context belongs to a separate receiver investigation.'},
        {'box': [121, 330, 150, 350], 'reason': 'Hanging ivy/green coverage, not a broad exposed stone riser.'},
        {'box': [173, 402, 226, 433], 'reason': 'Basal ivy obscures exact closure and ground contact.'},
    ]
    box = (105, 245, 245, 440)
    source.crop(box).resize((700, 975), Image.Resampling.NEAREST).save(args.output / 'original-5x.png')
    marked = source.copy()
    draw = ImageDraw.Draw(marked)
    for mass in masses:
        points = mass['outline']
        area = abs(sum(a[0] * b[1] - b[0] * a[1] for a, b in zip(points, points[1:] + points[:1]))) / 2
        assert area > 50
        mass['proposal_area_pixels2'] = area
        mass['source_ownership_assigned'] = False
        mass['depth_measured'] = False
        draw.line(points + points[:1], fill=mass['color'], width=1)
        draw.text((points[0][0], points[0][1] - 10), mass['id'][0], fill=mass['color'])
    for gap in gaps:
        draw.line(gap['points'], fill='#ff3333', width=1)
    marked.crop(box).resize((700, 975), Image.Resampling.NEAREST).save(args.output / 'whole-mass-proposals-5x.png')
    band_image = source.copy()
    draw = ImageDraw.Draw(band_image)
    band_marks = []
    for row in old['shelves']:
        band_marks.append({'id': row['id'], 'rim': row['rim'], 'lower': row['lower'],
                           'status': 'Retained original2D artwork marks only; old inferred3D depths are not locked.'})
        draw.line(row['rim'], fill='#00ffff', width=1)
        draw.line(row['lower'], fill='#ff55ff', width=1)
    band_image.crop(box).resize((700, 975), Image.Resampling.NEAREST).save(args.output / 'retained-band-marks-5x.png')
    shutil.copyfile(render_path, args.output / 'current-native-phase00.png')
    report = {
        'status': 'HOLD_PRIVATE_BANK53_SOURCE_MASS_PROPOSALS',
        'pins': {str(p): sha(p) for p in [source_path, render_path, marks_path, old_geometry_path]},
        'source_crop_xyxy': box, 'masses': masses, 'recess_proposals': gaps,
        'context_and_ivy_exclusions': exclusions, 'retained_2d_band_marks': band_marks,
        'comparison': [
            'Current native render retains a tall broad gray body punctuated by small accepted texture areas. It does not yet represent the complete exposed faces and shoulder changes seen in source.',
            'Source has a broad upper mass and broad lower ledge separated by an irregular partly obscured transition; no evidence supports three uniformly thin independent shelves.',
            'Several bright stripes lie inside a continuous exposed stone face. Treat them as source marks or possible surface striation until a silhouette turn, shadowed joint or independently visible return supports geometric separation.',
            'Green coverage can be moss on stone or foreground ivy. Neither green nor gray color alone assigns material ownership.'],
        'next_closed_front_scope': [
            'Reconstruct the full outlined face areas with broad irregular convex shoulders, not small caps on a planar front backing.',
            'Use actual outlined source edges as approximate2D constraints with stated uncertainty; keep whole-face interior depth a smooth inferred hypothesis.',
            'Fit B and D as the two broad primary masses, A as the upper shoulder, and E as the basal mass. Keep C subordinate/recessed unless clearer context establishes an independent rock.',
            'Connect neighboring masses through narrow recessed joints with coherent side/back surfaces. Do not extrude every pale stripe into a deep groove or disconnected ledge.',
            'Retain the original2D band marks in the source comparison, but allow old sampled3D band depths/heights to change.',
            'Preserve fixed shared crest and neighbors, exact accepted source RGBA and gameplay metadata; ray-audit any retained assignments against the rebuilt complete receiver.',
            'Keep ivy and neighboring cap regions unassigned until state/context evidence establishes owners; none of these proposal polygons is an accepted texture domain.',
            'Review full saved native, eight oblique/solid views and whole bank contacts before replacing53 baseline.'],
        'limits': ['Source-only manual proposals with2..4px boundary uncertainty, not approved topology or material ownership.',
                   'Some lower/side closures are obscured by ivy; complete physical depth and hidden boundaries remain inferred.',
                   'No geometry mutations, source RGB edits, bank assignments, heavy jobs or canonical changes.']}
    (args.output / 'receipt.json').write_text(json.dumps(report, indent=2) + '\n')
    print({'mass_proposals': len(masses), 'recess_proposals': len(gaps), 'status': report['status']})


if __name__ == '__main__':
    main()
