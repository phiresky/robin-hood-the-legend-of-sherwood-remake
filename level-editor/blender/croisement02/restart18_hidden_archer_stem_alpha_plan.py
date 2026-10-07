"""CPU audit of native-alpha holes on frozen climbing support tube sections."""
import json,math,hashlib
from pathlib import Path
import numpy as np
from scipy.spatial import cKDTree
from scipy.ndimage import gaussian_filter1d
from PIL import Image,ImageDraw
BASE=Path(__file__).resolve().parents[2]/'work/croisement02-refinement/restart14-hidden-archer'
DEST=BASE/'climbing-v17/inmap-stem-opacity-cpu-v2'
SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35));RAY=np.array([0,-COS,SIN])
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
    assert not DEST.exists();DEST.mkdir();inputs={}
    def read(p):inputs[str(p)]=sha(p);return json.loads(p.read_text())
    surface=BASE/'surface-v8/surfaces.npz';inputs[str(surface)]=sha(surface);data=np.load(surface);rv=data['vertices0'];rt=data['triangles0'];q=rv[rt];normals=np.zeros_like(rv);fn=np.cross(q[:,1]-q[:,0],q[:,2]-q[:,0])
    for k in range(3):np.add.at(normals,rt[:,k],fn)
    normals/=np.maximum(np.linalg.norm(normals,axis=1)[:,None],1e-12);tree=cKDTree(rv);attachment=read(BASE/'skeleton-v9-cpu/root-attachment-final-centers.json');paths=read(BASE/'geodesic-v8-cpu/report.json')['paths'];guides=[]
    for k,path in enumerate(paths):
        p=np.array(path['points']);n=gaussian_filter1d(normals[tree.query(p)[1]],1.5,axis=0);n/=np.maximum(np.linalg.norm(n,axis=1)[:,None],1e-12);guides.append(np.vstack([attachment['states'][0]['rock_path_roots'][k],p+n*4]))
    states=[];board=Image.new('RGB',(1000,500),'#333333');draw=ImageDraw.Draw(board)
    for row,state in enumerate(['initial','applied']):
        plan=read(BASE/f'skeleton-v9-cpu/{state}-plan.json');arr=read(BASE/f'lobes-v16-cpu/{state}-arrangement.json');construction=read(BASE/f'climbing-v17/profile-05-{state}/construction.json');model=BASE/f'climbing-v17/profile-05-{state}/model.blend';inputs[str(model)]=sha(model);assert inputs[str(model)]==construction['model_sha256'];source=Path(plan['source']);inputs[str(source)]=sha(source);assert inputs[str(source)]==plan['source_sha256'];rgba=np.array(Image.open(source).convert('RGBA'));h,w=rgba.shape[:2];ox,oy=plan['source_top_left'];a=next(r for r in attachment['states'] if r['state']==state);front=np.array(plan['front'])-RAY*.6;chains=guides+[front[c] for c in plan['segments']]+[np.array(a['climber_join']),np.array(a['right_branch'])];core=len(chains);chains += [np.array([l['twig_base'],l['twig_tip']]) for l in arr['lobes']+arr['offmap_lobes']];primary={0,1,2,core-1};records=[];all_bad=[]
        for ci,chain in enumerate(chains):
            base_radius=.65 if ci in primary else .18 if ci<core else .12;rows=[]
            for si,(p,q) in enumerate(zip(chain[:-1],chain[1:])):
                length=np.linalg.norm(q-p)
                if length<1e-7:continue
                axis=(q-p)/length;u=np.cross(axis,[0,0,1])
                if np.linalg.norm(u)<.01:u=np.cross(axis,[1,0,0])
                u/=np.linalg.norm(u);v=np.cross(axis,u);steps=max(1,math.ceil(length/.25));ds=length/steps
                for t in (np.arange(steps)+.5)/steps:
                    center=p+(q-p)*t;radius=base_radius*(1-.35*(si+t)/max(1,len(chain)-1));ring=center+radius*np.array([u*math.cos(j*math.tau/8)+v*math.sin(j*math.tau/8) for j in range(8)]);screen=np.column_stack([ring[:,0],-ring[:,1]*SIN-ring[:,2]*COS]);lo=np.floor(screen.min(0)-[ox,oy]).astype(int);hi=np.floor(screen.max(0)-[ox,oy]).astype(int);crop=np.zeros((hi[1]-lo[1]+1,hi[0]-lo[0]+1),dtype=np.uint8)
                    for y in range(lo[1],hi[1]+1):
                        for x in range(lo[0],hi[0]+1):
                            if 0<=x<w and 0<=y<h:crop[y-lo[1],x-lo[0]]=rgba[y,x,3]
                    inmap=bool(screen[:,1].min()>=0);opaque=bool((crop>=128).any());source_center=[float(center[0]),float(-center[1]*SIN-center[2]*COS)];rows.append(dict(world=center.tolist(),source=source_center,segment=si,inmap=inmap,fully_alpha_empty=not opaque,step_world=ds))
            bad=[r for r in rows if r['inmap'] and r['fully_alpha_empty']];all_bad+=bad
            longest=run=0.
            for r in rows:
                run=run+r['step_world'] if r['inmap'] and r['fully_alpha_empty'] else 0.;longest=max(longest,run)
            records.append(dict(chain=ci,role='rock guide' if ci<len(guides) else 'independent right bank stem' if ci==core-1 else 'core support' if ci<core else 'lobe twig',samples=len(rows),inmap_alpha_empty_sections=len(bad),sampled_empty_length=sum(r['step_world'] for r in bad),longest_sampled_empty_run=longest,examples=bad[:3]))
        # Native source overlay is diagnostic only: red marks certified empty
        # projected tube-section bounds, not changed source or a render.
        canvas=Image.new('RGBA',(250,210),(45,45,45,255));canvas.alpha_composite(Image.fromarray(rgba),(ox-80,oy+18));d=ImageDraw.Draw(canvas)
        for r in all_bad:
            x,y=r['source'];d.point((int(x-80),int(y+18)),fill=(255,50,180,255))
        board.paste(canvas.resize((500,420),Image.Resampling.NEAREST),(row*500,40));draw.text((row*500+8,8),state+': magenta = alpha-empty in-map tube sections',fill='white')
        states.append(dict(state=state,model_sha256=construction['model_sha256'],chains=records,inmap_alpha_empty_sections=len(all_bad),method='Exact projected bounding rectangle of each octagonal tube section sampled at <=0.25world spacing. A rectangle with no native opaque pixels certifies this section invisible under saved nearest/CLIP alpha; intervals between sections still require exact mesh audit.'))
    board.save(DEST/'native-alpha-holes.png');result=dict(status='CPU SUPPORT OPACITY HOLD; no geometry mutation',inputs=inputs,states=states,plot_sha256=sha(DEST/'native-alpha-holes.png'),next_plan=['Do not blanket-opacify in-map supports; empty native regions would acquire invented visible branches.','Preserve all7073known native pixels. Route physically continuous inferred support around receiver surfaces into rock-occluded space, with verified exterior clearance and source first-hit occlusion.','Keep visible connecting portions inside native opaque silhouette; connect across crest/edge without penetrating rock.','Independent right bank stem needs separate morphology decision: endpoint is bank-rooted but long bare geometry has no observed-source authority.','Off-map-only opacity correction remains a separate safe scoped change; it does not resolve these in-map gaps.'])
    encoded=json.dumps(result,indent=2)+'\n';assert len(encoded.encode())<2*1024**2;(DEST/'report.json').write_text(encoded);print(json.dumps({s['state']:s['inmap_alpha_empty_sections'] for s in states}))
if __name__=='__main__':main()
