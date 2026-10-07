"""Source-space diagnostic plots and checks; does not render or change a model."""
import json,hashlib,sys
from pathlib import Path
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
import numpy as np
from PIL import Image
from scipy.spatial import ConvexHull
from restart21_butterfly07_geometry_v2 import geometry
ROOT=Path(__file__).resolve().parents[3]
B=ROOT/'level-editor/work/croisement02-refinement/restart14-butterflies'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
    fitpath=B/'butterfly07-global-registration-v2/fit.json';fit=json.loads(fitpath.read_text())
    out=B/'butterfly07-global-registration-v2/source-review';out.mkdir(exist_ok=False)
    old=json.loads((B/'butterfly07-full-local-registration-v1/fit.json').read_text());assert len(fit['rows'])==99
    for phase,row in enumerate(fit['rows']):
        assert row['phase']==phase and row['source']==old['rows'][phase]['source']
        assert sha(Path(row['source']['source']))==row['source']['sha256']
    assert fit['fixed_material_proposal']==old['fixed_material_proposal']
    for first in range(0,99,25):
        fig,axes=plt.subplots(5,5,figsize=(12,12),facecolor='#202020')
        for offset,ax in enumerate(axes.flat):
            phase=first+offset;ax.set_facecolor('#202020');ax.set_xticks([]);ax.set_yticks([])
            if phase>=99:ax.axis('off');continue
            row=fit['rows'][phase];a=np.asarray(Image.open(row['source']['source']).convert('RGBA'))
            yy,xx=np.nonzero(a[:,:,3]>0);center=np.array([xx.mean()+.5,yy.mean()+.5]);h,w=a.shape[:2]
            ax.imshow(a,extent=[0,w,h,0],interpolation='nearest');body,wings=geometry(row['parameters']);shift=center+row['parameters'][5:7]
            for shape,color in [(body,'orange'),(wings[0],'#ff66cc'),(wings[1],'#22ddff')]:
                xy=shape[:,:2]+shift
                if len(shape)>7:xy=xy[ConvexHull(xy).vertices]
                xy=np.vstack([xy,xy[0]]);ax.plot(xy[:,0],xy[:,1],color=color,linewidth=.65)
            ax.set_xlim(-3,w+3);ax.set_ylim(h+3,-3)
            ax.set_title(f"{phase}: {row['covered']}/{row['source_pixels']} +{row['extra']} brightmiss{row['bright_missing']}",fontsize=8,color='white')
        fig.tight_layout();fig.savefig(out/f'phases-{first:02}-{min(first+24,98):02}.png',dpi=100);plt.close(fig)
    edges=fit['edges'];report=dict(status='HOLD_SOURCE_AND_CONTINUOUS_CONTACT_REVIEW_REQUIRED',fit_sha256=sha(fitpath),
        source_hashes_checked=99,source_records_unchanged=True,palette_unchanged=True,geometry_same_across_all_phases=True,
        covered=fit['source_pixels']-fit['missing_pixels'],source_pixels=fit['source_pixels'],missing=fit['missing_pixels'],extra=fit['extra_pixels'],bright_missing=fit['bright_missing'],
        max_body_step_degrees=max(e['body_rotation_degrees'] for e in edges),body_steps_over60=[e for e in edges if e['body_rotation_degrees']>60],
        max_absolute_hinge_step=max(abs(x) for e in edges for x in e['hinge_delta_degrees']),
        worst_source_rows=sorted([dict(phase=r['phase'],covered=r['covered'],source_pixels=r['source_pixels'],extra=r['extra'],bright_missing=r['bright_missing']) for r in fit['rows']],key=lambda r:r['bright_missing'],reverse=True)[:15],
        scope='Native2Dregistration only; fixed shape/palette; no depth edited. Figure lines are projected restgeometry, not saved-model rendered evidence.',
        outputs=[dict(path=str(p.relative_to(ROOT)),sha256=sha(p)) for p in sorted(out.glob('*.png'))])
    (out/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:v for k,v in report.items() if k not in ['outputs','body_steps_over60','worst_source_rows']}))
if __name__=='__main__':main()
