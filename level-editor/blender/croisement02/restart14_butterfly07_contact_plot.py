"""Small CPU source-space review of exact conservative butterfly contacts."""
import hashlib
import json
from pathlib import Path
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
from PIL import Image

ROOT = Path(__file__).resolve().parents[3]
B = ROOT / 'level-editor/work/croisement02-refinement/restart14-butterflies'
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()


def main():
    out = B / 'butterfly07-contact-review-v1'
    assert not out.exists(), 'Preserve previous evidence'
    report_path = B / 'butterfly07-prism-contacts-v2/report.json'
    plan_path = B / 'all7-context-plan-v1/plan.json'
    report = json.loads(report_path.read_text())
    plan = json.loads(plan_path.read_text())
    source = B.parent / 'source-states/covered.png'
    assert sha(source) == plan['base_art_sha256']
    sequence = next(s for s in plan['sequences'] if s['index'] == 14)
    art = Image.open(source).convert('RGB')
    colors = {'croisement02-tree-01': '#ff389e', 'croisement02-tree-02': '#00e5ff'}
    fig, axes = plt.subplots(2, 3, figsize=(14, 9))
    fig.suptitle('Butterfly07: exact canopy contacts inside conservative source-pixel prisms', fontsize=14)
    ax = axes[0, 0]
    ax.imshow(art)
    path = [f['alpha_centroid_display'] for f in sequence['path']]
    ax.plot(*zip(*path), color='white', linewidth=.8)
    for phase in (19, 20, 21, 92):
        x, y = path[phase]
        ax.scatter(x, y, s=30, color='#ffda38')
        ax.annotate(str(phase), (x, y), xytext=(8, 5 if phase%2 else -12), textcoords='offset points', color='white')
    ax.set(xlim=(100, 280), ylim=(380, 250), title='All99 source anchors; flagged phases numbered')
    for ax, phase in zip([axes[0,1], axes[0,2], axes[1,0], axes[1,1]], (19,20,21,92)):
        frame = sequence['path'][phase]
        image = Path(frame['source'])
        assert sha(image) == frame['sha256']
        x, y, w, h = frame['bbox']
        ax.imshow(art)
        ax.imshow(Image.open(image), extent=(x,x+w,y+h,y), interpolation='nearest')
        for row in report['rows']:
            if row['phase'] != phase:
                continue
            points = [p['screen'] for p in row['witnesses']]
            ax.scatter(*zip(*points), marker='x', s=25, color=colors[row['receiver']], label=row['receiver'].replace('croisement02-', ''))
        ax.set(xlim=(x-5,x+w+5), ylim=(y+h+5,y-5), title=f'Phase{phase}: source sprite + contact witnesses')
        ax.legend(fontsize=7, loc='upper right')
    ax = axes[1,2]
    ax.axis('off')
    lines = ['Exact alpha-tested triangle/pixel contact counts', 'Phase / receiver     ±0Z   ±2Z   ±4Z   ±8Z']
    for r in report['rows']:
        c=r['contacts_by_uniform_half_depth']
        lines.append(f"{r['phase']:>3} / {r['receiver'][-7:]}     {c['0.0']:>3}   {c['2.0']:>3}   {c['4.0']:>3}   {c['8.0']:>3}")
    lines += ['', 'Pink: Tree01; cyan: Tree02.', 'Witnesses are a bounded subset, not all contacts.', '±0Z is a horizontal footprint plane, not anatomy.', '', 'HOLD: butterfly07 actual body/wing geometry unbuilt.', 'No path lift or canopy change proposed.', 'Native FX display order is a separate contract.', 'Level0 bilinear alpha; no mip/swept proof.']
    ax.text(0,1,'\n'.join(lines),va='top',fontsize=9,family='monospace')
    fig.tight_layout(rect=(0,0,1,.95))
    out.mkdir()
    fig.savefig(out/'source-contact-plot.png', dpi=120)
    plt.close(fig)
    receipt = {'status':'ROOT_REVIEW_CPU_ONLY', 'report_sha256':sha(report_path), 'plan_sha256':sha(plan_path), 'source_sha256':sha(source), 'recipe_sha256':sha(Path(__file__)), 'plot_sha256':sha(out/'source-contact-plot.png'), 'next_prototype':{'scope':'Butterfly07 only; own source anatomy, fixed materials and conserved body/wing dimensions.', 'phases':[18,19,20,21,22,91,92,93], 'path':'Freeze proposal-v2 anchors and heights initially. Never elevate to reproduce unmasked FX composition.', 'context':'Stream only exact Tree01/02 GLBs named in report; no complete scene copy.', 'cpu_first':'Fit own source body registration and articulated wings, report ambiguous depths, then actual posed/swept triangle contacts. Conservative pixel prisms are not anatomy.', 'render_after_lane_assignment':'One bounded native/reverse pair at worst physical-contact phase, saved-model reload and actual/solid material review; no Blender before lane assignment.', 'integration':'Private prototype only. Shared native composition adapter belongs to runtime owner.'}}
    (out/'review.json').write_text(json.dumps(receipt,indent=2)+'\n')
    assert sum(p.stat().st_size for p in out.iterdir()) < 2*1024**2
    print(out)


if __name__ == '__main__':
    main()
