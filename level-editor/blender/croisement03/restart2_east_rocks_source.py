"""Record tentative visible stone domains before building the eastern outcrop."""
import json
import sys
from pathlib import Path
from PIL import Image, ImageDraw, ImageChops
from catalog import OUT
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'refinement/blender'))
from evidence_io import sha, write_json

# The source shows an upright stratified face, a low front block, and a left
# ledge. Their connections behind vegetation remain an inference.
TRACES = {
    'upright-face': [(1121,397),(1134,399),(1137,410),(1144,420),
                     (1154,427),(1156,437),(1152,449),(1144,452),
                     (1131,451),(1125,449),(1123,438),(1120,432),
                     (1121,421),(1117,416),(1118,405)],
    'front-block': [(1123,451),(1135,449),(1143,454),(1146,467),
                    (1135,471),(1121,468),(1118,459)],
    'left-ledge': [(1107,447),(1120,442),(1124,445),(1125,450),
                   (1117,456),(1110,458),(1107,468),(1099,470),
                   (1093,463),(1094,456),(1101,450)],
}

def main():
    out = OUT / 'restart2/east-rocks98-preflight'
    out.mkdir(exist_ok=False)
    level = json.loads((OUT / 'baseline/Croisement03.rhp.json').read_text())
    source = Image.open(OUT / 'baseline/covered.png').convert('RGB')
    native = Image.new('L', source.size)
    native.paste(Image.open(OUT / 'baseline/masks/000098.png'),
                 tuple(level['masks'][98]['box_top_left']))
    union = Image.new('L', source.size)
    annotated = source.copy()
    draw = ImageDraw.Draw(annotated)
    for index, (name, points) in enumerate(TRACES.items()):
        domain = Image.new('L', source.size)
        ImageDraw.Draw(domain).polygon(points, fill=255)
        domain = ImageChops.darker(domain, native)
        domain.save(out / f'{name}.png')
        union = ImageChops.lighter(union, domain)
        draw.line(points + points[:1], fill=['red','cyan','yellow'][index], width=1)
    union.save(out / 'tentative-stone-domain.png')
    ImageChops.subtract(native, union).save(out / 'unresolved-native-domain.png')
    box = (1048,374,1185,490)
    crops = [source.crop(box), annotated.crop(box)]
    sheet = Image.new('RGB', (137*10,116*5))
    for i, crop in enumerate(crops):
        sheet.paste(crop.resize((137*5,116*5),Image.Resampling.NEAREST),(i*137*5,0))
    sheet.save(out / 'source-traces.png')
    write_json(out / 'source-traces.json', dict(
        status='PRIVATE PRECONSTRUCTION HYPOTHESIS; ownership not approved',
        source_sha256=sha(OUT / 'baseline/covered.png'),
        native_mask=98, source_nodes=['building-074','building-075'],
        traces=TRACES,
        native_obstacles={str(i):level['sight_obstacles'][i] for i in (74,75)},
        source_crop=list(box), limitations=[
            'Mask98 includes substantial foreground shrub and is not a stone material domain.',
            'The tall pale face has stone-like horizontal strata; the nearby tree is separate.',
            'Three visible faces do not establish three independent physical rocks.',
            'Trace boundaries are tentative pixel observations, not independently verified ownership.',
            'Hidden joins, backs, seating and source-ray height remain to be constructed and reviewed.',
        ]))
    print(out)

if __name__ == '__main__':
    main()
