"""CPU-only own-source articulated butterfly07 hypothesis; no scene mutation."""
import hashlib,json,math
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
from scipy.ndimage import distance_transform_edt
from scipy.optimize import differential_evolution
from scipy.spatial import ConvexHull
from scipy.spatial.transform import Rotation
from matplotlib.path import Path as Polygon
ROOT=Path(__file__).resolve().parents[3]
B=ROOT/'level-editor/work/croisement02-refinement/restart14-butterflies'
OUT=B/'butterfly07-pose-fit-v1'
PHASES=[18,19,20,21,22,91,92,93]
SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35))
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
# Inferred conserved rest anatomy, bounded by this sequence's own open-wing frames.
WING=np.array([[0.,-1.5,0.],[1.8,-3.3,0.],[4.5,-3.1,0.],[4.8,-1.2,0.],[3.7,1.5,0.],[1.6,2.5,0.],[0.,1.1,0.]])
BODY=np.array([[.40*math.sin(a)*math.cos(t),2.7*math.cos(a),.45*math.sin(a)*math.sin(t)] for a in np.linspace(0,math.pi,9) for t in np.linspace(0,math.tau,17)[:-1]])

def geometry(p):
    rot=Rotation.from_euler('xyz',p[:3],degrees=True)
    wings=[]
    for sign,angle in [(-1,p[3]),(1,p[4])]:
        v=WING.copy();v[:,0]*=sign
        v=Rotation.from_euler('y',-sign*angle,degrees=True).apply(v)
        v[:,0]+=sign*.22
        wings.append(rot.apply(v))
    return rot.apply(BODY),wings

def main():
    assert not (OUT/'fit.json').exists(),'Preserve prior fit'
    OUT.mkdir(exist_ok=True)
    planp=B/'all7-context-plan-v1/plan.json';proposalp=B/'footprint-path-proposal-v2/proposal.json'
    plan=json.loads(planp.read_text());seq=next(s for s in plan['sequences'] if s['index']==14)
    route=next(s for s in json.loads(proposalp.read_text())['rows'] if s['sequence']==14)
    contexts={};bright=[];warm=[]
    for phase in PHASES:
        f=seq['path'][phase];assert sha(Path(f['source']))==f['sha256']
        a=np.asarray(Image.open(f['source']).convert('RGBA'));mask=a[:,:,3]>0;h,w=mask.shape
        yy,xx=np.nonzero(mask);center=np.array([xx.mean()+.5,yy.mean()+.5])
        gy,gx=np.mgrid[-3:h+3,-3:w+3];q=np.c_[gx.ravel()+.5,gy.ravel()+.5]
        target=np.zeros(gx.shape,bool);target[3:h+3,3:w+3]=mask
        dist=distance_transform_edt(~target).ravel()
        rgb=a[:,:,:3];br=mask&(rgb.max(2)>100);wa=mask&(rgb[:,:,0]>rgb[:,:,2]*1.3)&(rgb.max(2)<110)
        bright.extend(rgb[br].tolist());warm.extend(rgb[wa].tolist())
        contexts[phase]=(a,center,q,target.ravel(),dist)
    def evaluate(phase,p):
        a,center,q,target,dist=contexts[phase];body,wings=geometry(p);shift=center+p[5:7]
        body2=body[:,:2]+shift
        pred=Polygon(body2[ConvexHull(body2).vertices]).contains_points(q)
        for wing in wings:pred|=Polygon(wing[:,:2]+shift).contains_points(q)
        missed=int((target&~pred).sum());extra=int((pred&~target).sum())
        loss=(missed+(1+.2*dist[pred&~target]).sum())/target.sum()+.012*float(np.sum(p[5:7]**2))
        return {'parameters':list(map(float,p)),'missing':missed,'extra':extra,'covered':int((pred&target).sum()),'source_pixels':int(target.sum()),'loss':float(loss),'quaternion':Rotation.from_euler('xyz',p[:3],degrees=True).as_quat().tolist()}
    banks={}
    for phase in PHASES:
        candidates=[]
        for seed in (417,911):
            fit=differential_evolution(lambda p:evaluate(phase,p)['loss'],[(-85,85),(-85,85),(-120,120),(-88,88),(-88,88),(-2,2),(-2,2)],popsize=7,maxiter=100,polish=False,seed=seed+phase,tol=.005)
            for mirrored in (False,True):
                p=fit.x.copy()
                if mirrored:p[[0,1,3,4]]*=-1
                c=evaluate(phase,p);c.update(seed=seed,depth_mirrored=mirrored);candidates.append(c)
        banks[phase]=candidates
        print('fit',phase,'best',min(c['loss'] for c in candidates),flush=True)
    def transition(a,b):
        angle=2*math.acos(np.clip(abs(np.dot(a['quaternion'],b['quaternion'])),0,1))
        hinge=np.deg2rad(np.array(a['parameters'][3:5])-b['parameters'][3:5])
        return .12*(angle**2+.1*float(np.sum(hinge**2)))
    selected={}
    for phases in ([18,19,20,21,22],[91,92,93]):
        cost=np.array([c['loss'] for c in banks[phases[0]]]);back=[]
        for prev,current in zip(phases,phases[1:]):
            costs=cost[:,None]+np.array([[transition(a,b) for b in banks[current]]for a in banks[prev]])
            parent=costs.argmin(0);cost=costs[parent,np.arange(len(parent))]+[c['loss'] for c in banks[current]];back.append(parent)
        ids=[int(cost.argmin())]
        for p in reversed(back):ids.append(int(p[ids[-1]]))
        ids.reverse()
        selected.update({phase:banks[phase][idx] for phase,idx in zip(phases,ids)})
    rows=[];sheet=Image.new('RGB',(1200,520),'#252525');draw=ImageDraw.Draw(sheet)
    for i,phase in enumerate(PHASES):
        row=selected[phase];p=np.array(row['parameters']);a,center,*_=contexts[phase];body,wings=geometry(p)
        anchor=np.array(route['world_zup_knots'][phase]);shift=p[5:7]
        def world(v):
            # Orthonormal camera basis: source X, source down, source ray.
            x=v[:,0]+shift[0];y=v[:,1]+shift[1];d=v[:,2]
            return anchor+np.c_[x,-SIN*y-COS*d,-COS*y+SIN*d]
        vertices=np.vstack([world(body),*[world(w) for w in wings]])
        row.update(phase=phase,source=seq['path'][phase],fixed_path_anchor_zup=anchor.tolist(),registration_pixels=shift.tolist(),body_vertices_zup=world(body).tolist(),wing_vertices_zup=[world(w).tolist() for w in wings],height_range=[float(vertices[:,2].min()),float(vertices[:,2].max())],depth_range=[float(np.vstack([body,*wings])[:,2].min()),float(np.vstack([body,*wings])[:,2].max())])
        # Verify exact projection of the unchanged anchor and local registration.
        projected=np.c_[vertices[:,0],-SIN*vertices[:,1]-COS*vertices[:,2]]
        expected=np.vstack([body,*wings])[:,:2]+shift+np.array(seq['path'][phase]['alpha_centroid_display'])
        assert np.max(abs(projected-expected))<1e-10
        rows.append(row)
        ox=i%4*300;oy=i//4*260;scale=12
        draw.text((ox+5,oy+5),f"Phase{phase} covered{row['covered']}/{row['source_pixels']} extra{row['extra']}",fill='white')
        im=Image.fromarray(a);im=im.resize((im.width*scale,im.height*scale),Image.Resampling.NEAREST);sheet.paste(im,(ox+40,oy+40),im)
        points=lambda v:[(ox+40+float(x)*scale,oy+40+float(y)*scale)for x,y in v]
        for w,col in zip(wings,['#ff66cc','#22ddff']):
            poly=w[:,:2]+center+shift;draw.line(points(np.vstack([poly,poly[0]])),fill=col,width=2)
        b=body[:,:2]+center+shift;hull=b[ConvexHull(b).vertices];draw.line(points(np.vstack([hull,hull[0]])),fill='#ffaa33',width=2)
        draw.text((ox+5,oy+235),'Pink/cyan wings; orange body. Inferred depth.',fill='white')
    sheet.save(OUT/'eight-phase-fit.png')
    report={'status':'PRIVATE_CPU_ANATOMY_HYPOTHESIS_NOT_CONTACT_CLEARED','recipe_sha256':sha(Path(__file__)),'plan_sha256':sha(planp),'path_sha256':sha(proposalp),'own_source_only':True,'fixed_geometry':{'wing_outline':WING.tolist(),'body_radii':[.4,2.7,.45],'wing_hinge_offsets':[-.22,.22],'inferred':'One shared rest shape estimated from own open-wing frames; not identified physical dimensions.'},'fixed_material_proposal':{'wing_rgb_median':np.median(bright,axis=0).tolist(),'body_rgb_median':np.median(warm,axis=0).tolist(),'authority':'Own eight native frames only; one fixed palette across phases, no texture switching. Detailed UV pattern remains unbuilt.'},'rows':rows,'candidate_banks':banks,'limits':['CPU silhouette fit is not physical or rendered acceptance.','Depth mirror produces indistinguishable projected shapes; neither alternative is proven by source alone.','Warm body/antenna source pixels have ambiguous correspondence; no invented exact landmarks.','Only two short phase windows; full99 cycle and actual/swept receiver contacts remain unverified.','No canopy edits, path anchor changes, runtime changes or Blender output.']}
    (OUT/'fit.json').write_text(json.dumps(report,indent=2)+'\n')
    assert sum(p.stat().st_size for p in OUT.iterdir())<2*1024**2

if __name__=='__main__':main()
