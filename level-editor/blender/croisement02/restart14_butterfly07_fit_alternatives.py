"""Source fidelity and exact contacts of unchanged-anatomy pose alternatives."""
import collections,json
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
from scipy.spatial import ConvexHull
from matplotlib.path import Path as Polygon
import restart14_butterfly_canopy22_audit as reader
from restart14_butterfly07_pose_fit import geometry
from restart14_butterfly07_anatomy_contacts import triangle_planes,clip_planes
from restart14_butterfly07_prism_contacts import alpha_maximum
B=reader.B;OUT=B/'butterfly07-fit-alternatives-v1'

def main():
    assert not OUT.exists()
    fitp=B/'butterfly07-pose-fit-v1/fit.json';fit=json.loads(fitp.read_text());base={r['phase']:r for r in fit['rows']}
    variants=[];missing=[]
    for phase,row in base.items():
        source=np.asarray(Image.open(row['source']['source']).convert('RGBA'));mask=source[:,:,3]>0;h,w=mask.shape
        yy,xx=np.nonzero(mask);center=np.array([xx.mean()+.5,yy.mean()+.5]);gy,gx=np.mgrid[:h,:w];q=np.c_[gx.ravel()+.5,gy.ravel()+.5]
        for index,candidate in enumerate(fit['candidate_banks'][str(phase)]):
            p=np.array(candidate['parameters']);body,wings=geometry(p);anchor=np.array(row['fixed_path_anchor_zup'])
            def world(v):
                x=v[:,0]+p[5];y=v[:,1]+p[6];d=v[:,2]
                return anchor+np.c_[x,-reader.SIN*y-reader.COS*d,-reader.COS*y+reader.SIN*d]
            parts=[];v=world(body);parts.append(('body',v,ConvexHull(v).equations))
            for wi,wing in enumerate(wings):
                v=world(wing)
                for k in range(1,len(v)-1):parts.append((f'wing{wi}',v[[0,k,k+1]],triangle_planes(v[[0,k,k+1]])))
            variants.append(dict(phase=phase,index=index,candidate=candidate,parts=parts,contacts=collections.Counter(),witnesses=[]))
        p=np.array(row['parameters']);body,wings=geometry(p);shift=center+p[5:7];b=body[:,:2]+shift
        pred=Polygon(b[ConvexHull(b).vertices]).contains_points(q)
        for wing in wings:pred|=Polygon(wing[:,:2]+shift).contains_points(q)
        absent=mask&~pred.reshape(h,w);pixels=[]
        for y,x in zip(*np.nonzero(absent)):
            rgb=source[y,x,:3].tolist()
            # Brightness is evidence, not an anatomical classification.
            pixels.append({'local_xy':[int(x),int(y)],'screen_xy':[int(x+row['source']['bbox'][0]),int(y+row['source']['bbox'][1])],'rgba':source[y,x].tolist(),'role':'bright_unresolved_body_or_wing' if max(rgb)>=100 else 'dark_unresolved_filament_or_edge'})
        assert len(pixels)==row['missing']
        missing.append(dict(phase=phase,pixels=pixels,mask=absent,source=source))
    def inspect(placed,node,ni,pi,tri,uvs,mat,alpha,texture_record):
        world=np.stack([tri[:,:,0],-tri[:,:,2],tri[:,:,1]],axis=2);uv=np.zeros((*tri.shape[:2],2))if uvs is None else uvs
        attrs=np.concatenate([world,uv],axis=2);low=world.min(1);high=world.max(1)
        opaque=mat.get('alphaMode','OPAQUE')=='OPAQUE';image,sampler,factor=(None,{},1.)if opaque else texture_record(mat)
        cutoff=mat.get('alphaCutoff',.5)if mat.get('alphaMode')=='MASK'else .01
        for variant in variants:
            for name,v,planes in variant['parts']:
                ids=np.flatnonzero(np.all(high>=v.min(0)-1e-8,axis=1)&np.all(low<=v.max(0)+1e-8,axis=1))
                for idx in ids:
                    poly=clip_planes(attrs[idx],planes)
                    if not len(poly):continue
                    value,witness,_=alpha_maximum(poly,image,sampler,factor)
                    if value<cutoff:continue
                    assert opaque or abs(value-alpha(mat,witness[3:5],True))<1e-6
                    variant['contacts'][placed['id']]+=1
                    if len(variant['witnesses'])<3:variant['witnesses'].append({'part':name,'receiver':placed['id'],'material':mat.get('name'),'triangle':int(idx),'world_zup':witness[:3].tolist(),'alpha':value})
    def finish(rays,assets,map_path):
        OUT.mkdir();sheet=Image.new('RGB',(1200,640),'#252525');draw=ImageDraw.Draw(sheet)
        rows=[]
        for i,entry in enumerate(missing):
            phase=entry['phase'];ox=i%4*300;oy=i//4*320;source=Image.fromarray(entry['source']);im=source.resize((source.width*10,source.height*10),Image.Resampling.NEAREST);sheet.paste(im,(ox+20,oy+40),im)
            for pix in entry['pixels']:
                x,y=pix['local_xy'];col='#ff5577'if pix['role'].startswith('bright')else'#22ddff';draw.rectangle([ox+20+x*10,oy+40+y*10,ox+29+x*10,oy+49+y*10],outline=col,width=1)
            draw.text((ox+5,oy+5),f"Phase{phase}: missing{len(entry['pixels'])}",fill='white')
            for j,v in enumerate(t for t in variants if t['phase']==phase):
                c=v['candidate'];contacts=dict(v['contacts']);draw.text((ox+5,oy+220+j*17),f"alt{v['index']}: {c['covered']}/{c['source_pixels']} extra{c['extra']} contacts{sum(contacts.values())}",fill='#ff9999'if contacts else'#99ffbb')
                rows.append({'phase':phase,'alternative':v['index'],'covered':c['covered'],'missing':c['missing'],'extra':c['extra'],'parameters':c['parameters'],'depth_mirrored':c['depth_mirrored'],'contacts':contacts,'witnesses':v['witnesses']})
        draw.text((5,620),'Missing pixels: pink = bright unresolved body/wing; cyan = dark unresolved filament/edge. Color alone does not identify anatomy.',fill='white')
        sheet.save(OUT/'comparison.png')
        report={'status':'SOURCE_AND_CONTACT_ALTERNATIVES_NOT_PATH_CLEARANCE','fit_sha256':reader.sha(fitp),'recipe_sha256':reader.sha(Path(__file__)),'map_sha256':reader.sha(map_path),'assets':assets,'model_complexity':'All32 candidates retain exactly the same body ellipsoid, two7vertex wing outlines and10wing triangles. Only body rotation, two hinges and bounded source body registration differ. No extra sections, new dimensions, path changes or material changes.','rows':rows,'missing49_provenance':[{k:v for k,v in e.items()if k not in ('source','mask')}for e in missing],'limits':['Exact discrete posed contacts only; a contact-free pose does not clear temporal sweeps.','Missing49 source pixels have per-pixel provenance and unresolved anatomical roles; none silently discarded.','No image-derived evidence distinguishes depth mirror branches.','No Blender, render or library writes.']}
        (OUT/'report.json').write_text(json.dumps(report,indent=2)+'\n');print([(r['phase'],r['alternative'],r['covered'],r['contacts'])for r in rows],flush=True)
        return report
    reader.main(ray_records=[{'screen':r['source']['alpha_centroid_display'],'hits':[]}for r in base.values()],postprocess=finish,output=OUT,asset_ids={'croisement02-tree-01','croisement02-tree-02'},triangle_callback=inspect,query_margin=15.,output_limit_bytes=2*1024**2)

if __name__=='__main__':main()
