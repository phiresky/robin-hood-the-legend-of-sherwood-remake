"""Retain inspectable native bridge observations before constructing its depth."""
import hashlib
import json
from pathlib import Path
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'level-editor/work/croisement03-refinement'
# Native full-image coordinates; endpoints hidden by foliage are estimates.
RUNS = {
    'deck_perimeter': [(840, 663), (914, 638), (1058, 732), (985, 757)],
    'near_handrail': [(837, 643), (984, 745)],
    'far_handrail': [(942, 632), (1060, 711)],
    'near_post_0': [(835, 630), (837, 665)],
    'near_post_1': [(887, 680), (889, 699)],
    'near_post_2': [(922, 705), (923, 724)],
    'near_post_3': [(950, 724), (951, 743)],
    'near_post_4': [(984, 740), (985, 762)],
    'far_post_0': [(950, 638), (950, 650)],
    'far_post_1': [(976, 656), (976, 668)],
    'far_post_2': [(1000, 674), (1001, 687)],
    'far_post_3': [(1041, 698), (1042, 716)],
    'far_post_4': [(1061, 698), (1061, 735)],
    'pier': [(908, 732), (910, 761)],
    'pier_brace_left': [(895, 709), (908, 734)],
    'pier_brace_right': [(908, 734), (924, 724)],
}


def main():
    source = OUT / 'baseline/covered.png'
    destination = OUT / 'bridge-research/native-trace-v1'
    destination.mkdir(parents=True, exist_ok=False)
    crop = (820, 620, 1080, 785)
    raw = Image.open(source).convert('RGB').crop(crop)
    raw.save(destination / 'source.png')
    marked = raw.resize((1040, 660), Image.Resampling.NEAREST)
    draw = ImageDraw.Draw(marked)
    observations = []
    for number, (name, points) in enumerate(RUNS.items()):
        xy = [((x-crop[0])*4, (y-crop[1])*4) for x,y in points]
        draw.line(xy + (xy[:1] if name == 'deck_perimeter' else []), fill='#ff55ff', width=2)
        for x,y in xy:
            draw.ellipse((x-3,y-3,x+3,y+3), fill='#ffff00')
        draw.text((xy[0][0]+4,xy[0][1]), str(number), fill='white', stroke_width=1, stroke_fill='black')
        observations.append(dict(index=number, name=name, source_pixels=points,
                                 uncertainty_pixels=3, status='manual trace awaiting mesh/source comparison'))
    marked.save(destination / 'numbered-trace.png')
    report = dict(source_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),
                  crop=crop, observations=observations,
                  limitations=[
                      'Deck endpoints and hidden pier feet are inferred where foliage or bank obscures them.',
                      'The short far railing does not establish a railing across the northwest landing.',
                      'Cross-plank count and widths require a separate source trace before final geometry.',
                      'Depth and water level are not determined by these projected coordinates.',
                      'Native masks110/111/112 cover rails and pier, not the complete deck domain.'])
    (destination / 'observations.json').write_text(json.dumps(report, indent=2)+'\n')
    print(destination)


if __name__ == '__main__':
    main()
