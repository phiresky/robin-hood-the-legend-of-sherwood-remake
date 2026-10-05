"""Record source axial silhouette and radiance evidence without assigning cargo count."""
import hashlib
import json
from pathlib import Path
import numpy as np
from PIL import Image
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
from restart3_fit_cart_barrel import OUT, SIN, silhouette


def main():
    root=OUT/'restart3-south-cart'
    dest=root/'barrel-axial-profile-v1';dest.mkdir(exist_ok=False)
    fit_path=root/'barrel-fit-v1/fit.json';fit=json.loads(fit_path.read_text())
    source_path=Path(fit['source_frame']['image'])
    rgba=np.asarray(Image.open(source_path).convert('RGBA'))
    mask=np.asarray(Image.open(root/'barrel-fit-v1/body-domain.png'))>0
    model=silhouette(fit['parameters'])
    angle,_,_,cx,cy=fit['parameters']
    axis=np.array([np.cos(angle),-np.sin(angle)*SIN]);axis/=np.linalg.norm(axis)
    across=np.array([-axis[1],axis[0]])
    yy,xx=np.indices(mask.shape);q=np.stack([xx-cx,yy-cy],axis=-1)
    axial=q@axis;radial=q@across
    luma=rgba[:,:,:3]@np.array([.2126,.7152,.0722])
    rows=[]
    for center in np.arange(-37.5,38,2.5):
        band=abs(axial-center)<1.25
        entry={'axial_center':float(center)}
        for name,domain in [('native',mask),('single_cask',model)]:
            values=radial[band&domain]
            entry[name]=None if not len(values) else [float(values.min()),float(values.max())]
        brightness=luma[band&mask]
        entry['radiance_median']=float(np.median(brightness)) if len(brightness) else None
        entry['radiance_p90']=float(np.quantile(brightness,.9)) if len(brightness) else None
        rows.append(entry)
    fig,axs=plt.subplots(2,2,figsize=(12,9),layout='constrained')
    axs[0,0].imshow(rgba,interpolation='nearest');axs[0,0].set_ylim(125,60)
    axs[0,0].set_title('Terminal native appearance (no relabeling)')
    for t in [-15,0,15]:
        pts=np.array([cx,cy])+axis*t+np.array([-22,22])[:,None]*across
        axs[0,0].plot(pts[:,0],pts[:,1],label=f'axis {t}')
    axs[0,0].legend()
    axs[0,1].scatter(axial[mask],radial[mask],c=rgba[mask,:3]/255,s=18,marker='s')
    axs[0,1].invert_yaxis();axs[0,1].set_aspect('equal');axs[0,1].set_title('Native pixels in fitted axial coordinates')
    for name,style in [('native','-'),('single_cask','--')]:
        valid=[r for r in rows if r[name] is not None]
        for index,label in [(0,'edge A'),(1,'edge B')]:
            axs[1,0].plot([r['axial_center'] for r in valid],[r[name][index] for r in valid],style,label=f'{name} {label}')
    axs[1,0].set_title('Opaque contour: two separate radial edges');axs[1,0].legend();axs[1,0].set_ylabel('Radial pixels')
    for key in ['radiance_median','radiance_p90']:
        axs[1,1].plot([r['axial_center'] for r in rows],[r[key] for r in rows],label=key)
    axs[1,1].set_title('Radiance variation is not geometry proof');axs[1,1].legend()
    for ax in axs[1]:ax.set_xlabel('Axial pixels');ax.grid(alpha=.2)
    fig.savefig(dest/'profile.png',dpi=150);plt.close(fig)
    sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
    report=dict(source_path=str(source_path),source_sha256=sha(source_path),fit_sha256=sha(fit_path),
        model_sha256=sha(root/'barrel-v1/worker.blend'),axial_unit=axis.tolist(),radial_unit=across.tolist(),rows=rows,
        findings=['The full opaque contour alone is nearly constant width and includes a dark lower-left region of unresolved material/role.',
         'Two bright lobes and the intervening dark join require a separate geometric hypothesis; silhouette IoU alone cannot select a single cylinder.',
         'Dark regions are not automatically shadow; bright regions are not automatically separate objects.',
         'Thin vertical strip remains source-reserved; no physical ownership inferred.'],status='Evidence only; original single-cask candidate remains HOLD')
    (dest/'report.json').write_text(json.dumps(report,indent=2)+'\n')

if __name__=='__main__':main()
