"""Review inferred depth diagnostics without promoting an incomplete restshape."""
import json
import numpy as np
from scipy.interpolate import CubicSpline
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
import restart14_butterfly_canopy22_audit as reader
B=reader.B;OUT=B/'butterfly07-depth-review-v1'

def main():
    assert not OUT.exists();curvep=B/'butterfly07-depth-curve-v1/curve.json';candidate=json.loads(curvep.read_text());curve=CubicSpline(candidate['curve']['times'],candidate['curve']['heights'],bc_type='periodic');oldp=B/'footprint-path-proposal-v2/proposal.json';old=next(r for r in json.loads(oldp.read_text())['rows']if r['sequence']==14);height=np.array(old['world_zup_knots'])[:,2];densep=B/'butterfly07-depth-curve-dense32-v1/report.json';dense=json.loads(densep.read_text());sourcep=B/'butterfly07-full99-source-depth-v1/report.json';source=json.loads(sourcep.read_text());envelopep=B/'butterfly07-full99-envelope-v2/report.json';envelope=json.loads(envelopep.read_text());t=np.linspace(0,99,3961);hits=[]
    for key,value in dense['results'].items():
        if key.startswith('selected:dense-pose:')and value['contact_counts']:
            _,_,phase,fraction=key.split(':');hits.append(int(phase)+float(fraction))
    fig,axes=plt.subplots(3,1,figsize=(12,9));axes[0].plot(np.arange(100),np.r_[height,height[0]],label='Previous inferred height',color='#777777');axes[0].plot(t,curve(t),label='Private C2 ray-depth candidate',color='#148e96');axes[0].set(xlim=(0,99),ylabel='World Z',title='HOLD: current restshape is too small for wider own-source frames');axes[0].legend()
    for ax,(left,right)in zip(axes[1:],[(18,22),(91,93)]):
        q=np.linspace(left,right,1000);ax.plot(q,curve(q),color='#148e96');local=[v for v in hits if left<=v<=right];ax.scatter(local,curve(local),color='#d3294d',label='Confirmed dense anatomical contact');ax.set(xlim=(left,right),ylabel='World Z',xlabel='Phase (integer phases are native observations)');ax.legend()
    fig.suptitle('Butterfly07 inferred depth: native screen anchors, timing and elevation/order metadata unchanged');fig.tight_layout();OUT.mkdir();fig.savefig(OUT/'depth-diagnostics.png',dpi=120);plt.close(fig)
    report={'status':'HOLD_REVISED_GLOBAL_RESTSHAPE_AND_ACTUAL_FULL99_MOTION','bindings':{p.name if p==curvep else str(p.relative_to(B)):reader.sha(p)for p in [curvep,oldp,densep,sourcep,envelopep]},'native_metadata':'Elevation100 remains real native sprite-position/order metadata; never substituted by reconstructed body heights.','candidate':'PeriodicC2 depth candidate; native2D screen anchor exact by ray construction. Max deviation4atquarterknots, approximately4.080betweenknots. Speed3.045Z/phase,acceleration15.248Z/phase², both inferred diagnostic bounds.','exact_old_rig':'All8endpoints and6midpoints clear, but32step pose audit confirms20intermediate contacts across19–20,91–92,92–93. Three ofsix bounded32step sweeps clear.','full99':'All5015source-pixel samples across99frames tested against static map. ±8Z source footprints flag phases19/20/21/92. Conservative9radius fixed-rig continuous envelope clears50of99intervals; remaining possible contacts areTree01,Tree02 andshrub60. These envelope contacts are not actual anatomy collisions.','restshape_hold':'Independent full99 own-source audit proves old fixed rig cannot cover wide frames: conservative occupied-center bound86.47 versus124native pixels. No full99 physical completion can be claimed. A single globally revised restshape is in progress; all geometry-sensitive proofs must be repeated.','next_ready_interface':'Exact topology/dimensions guard rejects a changed fit with the old geometry helper. Contact interval enumeration now supports all99 including98→0. Native source/timing/path untouched.','limits':['Complete99actual anatomical proof awaits revised restshape plus99own registrations.','Static geometry only; no dynamic state/actor proof.','No Blender, API or library publication.']};(OUT/'review.json').write_text(json.dumps(report,indent=2)+'\n')

if __name__=='__main__':main()
