"""Own-source body-axis hypotheses before butterfly wing-motion fitting."""
import copy,json,math
import numpy as np
from PIL import Image,ImageDraw
from scipy.ndimage import distance_transform_edt
from scipy.optimize import differential_evolution
from scipy.spatial import ConvexHull
from scipy.spatial.transform import Rotation
from matplotlib.path import Path as Polygon
from restart14_butterfly07_pose_fit import geometry
import restart14_butterfly07_fit_alternatives as audit
B=audit.B;OUT=B/'butterfly07-body-axis-fit-v1'
# Own-source warm center / paired-lobe gap hypotheses, not identified head or tail.
LANDMARKS={18:([2.7,4.9],[-.65,.76]),19:([3.4,5.1],[.30,.95]),20:([4.5,2.8],[0.,1.]),21:([4.5,5.5],[.15,.99]),22:([2.8,5.8],[.30,.95]),91:([5.7,5.3],[.45,.89]),92:([2.8,6.1],[.30,.95]),93:([4.0,6.9],[.73,.68])}

def main():
    assert not OUT.exists();OUT.mkdir()
    source=B/'butterfly07-continuous-selected-v1/fit.json';fit=json.loads(source.read_text());rows={r['phase']:r for r in fit['rows']};banks={};contexts={}
    for phase,row in rows.items():
        a=np.asarray(Image.open(row['source']['source']).convert('RGBA'));mask=a[:,:,3]>0;h,w=mask.shape;yy,xx=np.nonzero(mask);center=np.array([xx.mean()+.5,yy.mean()+.5]);gy,gx=np.mgrid[-3:h+3,-3:w+3];q=np.c_[gx.ravel()+.5,gy.ravel()+.5];target=np.zeros(gx.shape,bool);target[3:h+3,3:w+3]=mask;dist=distance_transform_edt(~target).ravel();weights=np.ones(gx.shape);weights[3:h+3,3:w+3]+=2*(a[:,:,:3].max(2)>=100);target=target.ravel();weights=weights.ravel();contexts[phase]=(a,center)
        landmark,axis=map(np.array,LANDMARKS[phase]);axis=axis/np.linalg.norm(axis);zref=math.degrees(math.atan2(-axis[0],axis[1]))
        def evaluate(p):
            body,wings=geometry(p);shift=center+p[5:7];b=body[:,:2]+shift;pred=Polygon(b[ConvexHull(b).vertices]).contains_points(q)
            for wing in wings:pred|=Polygon(wing[:,:2]+shift).contains_points(q)
            bodyaxis=Rotation.from_euler('xyz',p[:3],degrees=True).apply([0.,1.,0.])[:2];bodyaxis/=max(np.linalg.norm(bodyaxis),1e-8)
            anchor=float(np.sum((shift-landmark)**2)+4*np.sum((bodyaxis-axis)**2));loss=(weights[target&~pred].sum()+(1+.2*dist[pred&~target]).sum()+1.5*anchor)/target.sum()
            return {'parameters':list(map(float,p)),'covered':int((target&pred).sum()),'missing':int((target&~pred).sum()),'extra':int((pred&~target).sum()),'source_pixels':int(target.sum()),'bright_missing':int(((weights>1)&target&~pred).sum()),'loss':float(loss),'body_axis_screen':bodyaxis.tolist(),'body_center_local':shift.tolist(),'quaternion':Rotation.from_euler('xyz',p[:3],degrees=True).as_quat().tolist()}
        candidates=[]
        for seed in [8701,9701]:
            result=differential_evolution(lambda p:evaluate(p)['loss'],[(-55,55),(-55,55),(zref-25,zref+25),(-88,88),(-88,88),(-2,2),(-2,2)],seed=seed+phase,popsize=7,maxiter=110,polish=False,tol=.003)
            for mirrored in [False,True]:
                p=result.x.copy()
                if mirrored:p[[0,1,3,4]]*=-1
                candidate=evaluate(p);candidate.update(depth_mirrored=mirrored,seed=seed);candidates.append(candidate)
        banks[phase]=candidates;print(phase,[(c['covered'],c['extra'],c['bright_missing'])for c in candidates],flush=True)
    def transition(a,b):
        angle=(Rotation.from_quat(a['quaternion']).inv()*Rotation.from_quat(b['quaternion'])).magnitude();hinges=np.deg2rad(np.array(a['parameters'][3:5])-b['parameters'][3:5]);return .3*(angle**2+.1*float(hinges@hinges))
    selected={}
    for phases in ([18,19,20,21,22],[91,92,93]):
        cost=np.array([r['loss']for r in banks[phases[0]]]);back=[]
        for prev,current in zip(phases,phases[1:]):
            total=cost[:,None]+np.array([[transition(a,b)for b in banks[current]]for a in banks[prev]]);parent=total.argmin(0);cost=total[parent,np.arange(len(parent))]+[c['loss']for c in banks[current]];back.append(parent)
        ids=[int(cost.argmin())]
        for parent in reversed(back):ids.append(int(parent[ids[-1]]))
        selected.update({phase:banks[phase][i]for phase,i in zip(phases,reversed(ids))})
    result=[];sheet=Image.new('RGB',(1200,560),'#252525');draw=ImageDraw.Draw(sheet)
    for i,(phase,old)in enumerate(rows.items()):
        c=copy.deepcopy(selected[phase]);c.update(phase=phase,source=old['source'],fixed_path_anchor_zup=old['fixed_path_anchor_zup']);result.append(c);a,center=contexts[phase];ox=i%4*300;oy=i//4*280;im=Image.fromarray(a);im=im.resize((im.width*12,im.height*12),Image.Resampling.NEAREST);sheet.paste(im,(ox+25,oy+40),im);draw.text((ox+4,oy+5),f"Phase{phase}: {c['covered']}/{c['source_pixels']} extra{c['extra']} brightmiss{c['bright_missing']}",fill='white');p=np.array(c['parameters']);body,wings=geometry(p)
        for v,col in [(body,'orange'),(wings[0],'#ff66cc'),(wings[1],'#22ddff')]:
            poly=v[:,:2]+center+p[5:7]
            if len(poly)>7:poly=poly[ConvexHull(poly).vertices]
            draw.line([(ox+25+x*12,oy+40+y*12)for x,y in np.vstack([poly,poly[0]])],fill=col,width=2)
        centerline=np.array(LANDMARKS[phase][0]);axis=np.array(LANDMARKS[phase][1]);draw.line([(ox+25+x*12,oy+40+y*12)for x,y in [centerline-2*axis,centerline+2*axis]],fill='white',width=1);draw.text((ox+4,oy+245),'White: uncertain source body-axis hypothesis',fill='white')
    sheet.save(OUT/'axis-and-wing-fit.png');proposal=copy.deepcopy(fit);proposal.update(status='BODY_AXIS_HYPOTHESES_NOT_IDENTIFIED_ANATOMY',rows=result,candidate_banks=banks,parent_fit_sha256=audit.reader.sha(source),body_landmarks=LANDMARKS,body_landmark_uncertainty_pixels=1.5,body_axis_uncertainty_degrees=25,observed_vs_inferred='Native RGBA and phase positions observed. Body center/axis from own warm pixels and wing gap is a manual uncertain hypothesis; head/tail identity and all depth remain inferred. Long dark pixels are not automatically antennae or anatomy. Neighbor source16–24 and89–95 inspected.');(OUT/'fit.json').write_text(json.dumps(proposal,indent=2)+'\n');audit.main(OUT/'fit.json',B/'butterfly07-body-axis-contacts-v1')

if __name__=='__main__':main()
