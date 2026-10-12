"""Own-source observations with separate ellipsoid body parts and ray-depth ownership."""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
import argparse,json,importlib.util,hashlib
from pathlib import Path
import numpy as np
from PIL import Image
from matplotlib.path import Path as Polygon
from scipy.spatial.transform import Rotation
import restart64_butterfly07_source_ownership as ownership
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()

def front_depth(xy,rotation,part):
    radii=np.array(part['radii']);origin=np.c_[xy,np.zeros(len(xy))]@rotation-np.array(part['center']);direction=rotation[2,:]
    aa=np.sum((direction/radii)**2);bb=2*np.sum(origin*direction/radii**2,axis=1);cc=np.sum((origin/radii)**2,axis=1)-1;disc=bb*bb-4*aa*cc
    return np.where(disc>=0,(-bb+np.sqrt(np.maximum(disc,0)))/(2*aa),-np.inf)

def main():
    p=argparse.ArgumentParser();p.add_argument('--fit',type=Path,required=True);p.add_argument('--geometry',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();a.output.mkdir(exist_ok=False);fit=json.loads(a.fit.read_text());spec=importlib.util.spec_from_file_location('segmented',a.geometry);geo=importlib.util.module_from_spec(spec);spec.loader.exec_module(geo)
    assert fit['fixed_geometry']==geo.SPEC;ownership.geometry=geo.geometry;ownership.HINGES=geo.HINGES;rows=[ownership.prepare(r) for r in fit['rows']];outline=geo.WING;parts=geo.SPEC['body_parts'];scale=fit['fixed_uniform_scale'];surfaces=[]
    def save(name,colors,phases,coordinates,shape,domain):
        known=phases>=0
        for phase in np.unique(phases[known]):
            ids=np.flatnonzero(phases==phase);xy=coordinates[ids];image=rows[int(phase)]['rgba'];assert np.array_equal(colors[ids,:3],image[xy[:,1],xy[:,0],:3]);assert np.all(image[xy[:,1],xy[:,0],3]>0)
        target=a.output/f'{name}.png';Image.fromarray(colors.reshape(*shape,4)).save(target);Image.fromarray((known.reshape(shape)*255).astype('uint8')).save(a.output/f'{name}-known.png');np.savez_compressed(a.output/f'{name}-provenance.npz',phase=phases.reshape(shape),source_xy=coordinates.reshape(*shape,2));surfaces.append(dict(name=name,known_texels=int((known&domain).sum()),unknown_surface_texels=int((~known&domain).sum()),image_sha256=sha(target),exact_rgb_readback=True))
    w,h=64,96;yy,xx=np.mgrid[:h,:w];uv=np.c_[(xx.ravel()+.5)/w*8*scale,((yy.ravel()+.5)/h*13-7)*scale];domain=Polygon(outline[:,:2]).contains_points(uv);best=np.zeros((4,h*w));colors=np.full((4,h*w,4),255,np.uint8);colors[:,:,:3]=112;phases=np.full((4,h*w),-1,int);coordinates=np.full((4,h*w,2),-1,int)
    for c in rows:
        image=c['rgba'];rot=Rotation.from_euler('xyz',c['row']['parameters'][:3],degrees=True).as_matrix()
        for m in c['maps']:
            xy=uv@m['matrix'][:2,:2].T+m['offset'][:2]+c['shift'];ix,iy=np.floor(xy).astype(int).T;z=uv@m['matrix'][2,:2]+m['offset'][2];valid=domain&(ix>=0)&(ix<image.shape[1])&(iy>=0)&(iy<image.shape[0])
            for part in parts:valid&=front_depth(xy-c['shift'],rot,part)<z-1e-7
            other=c['maps'][1-m['side']];otheruv=(xy-c['shift']-other['offset'][:2])@np.linalg.inv(other['matrix'][:2,:2]).T;otherz=otheruv@other['matrix'][2,:2]+other['offset'][2];valid&=~(Polygon(outline[:,:2]).contains_points(otheruv)&(otherz>z))
            ids=np.flatnonzero(valid);ids=ids[image[iy[ids],ix[ids],3]>0];weight=abs(np.linalg.det(m['matrix'][:2,:2]))**2;index=m['side']*2+m['face'];ids=ids[weight>best[index,ids]];best[index,ids]=weight;colors[index,ids,:3]=image[iy[ids],ix[ids],:3];phases[index,ids]=c['row']['phase'];coordinates[index,ids]=np.c_[ix[ids],iy[ids]]
    for index in range(4):save(f'wing-{index}',colors[index],phases[index],coordinates[index],(h,w),domain)
    w,h=64,32;yy,xx=np.mgrid[:h,:w];polar=(yy.ravel()+.5)/h*np.pi;az=(xx.ravel()+.5)/w*2*np.pi;unit=np.c_[np.sin(polar)*np.cos(az),np.cos(polar),np.sin(polar)*np.sin(az)]
    for own,part in enumerate(parts):
        radii=np.array(part['radii']);points=unit*radii+np.array(part['center']);normals=unit/radii;normals/=np.linalg.norm(normals,axis=1)[:,None];best=np.zeros(h*w);colors=np.full((h*w,4),255,np.uint8);colors[:,:3]=112;phases=np.full(h*w,-1,int);coordinates=np.full((h*w,2),-1,int)
        for c in rows:
            rot=Rotation.from_euler('xyz',c['row']['parameters'][:3],degrees=True).as_matrix();vertices=points@rot.T;n=normals@rot.T;xy=vertices[:,:2]+c['shift'];ix,iy=np.floor(xy).astype(int).T;image=c['rgba'];valid=(n[:,2]>0)&(ix>=0)&(ix<image.shape[1])&(iy>=0)&(iy<image.shape[0])
            for other,piece in enumerate(parts):
                if own!=other:valid&=front_depth(vertices[:,:2],rot,piece)<vertices[:,2]-1e-7
            for m in c['maps']:
                winguv=(xy-c['shift']-m['offset'][:2])@np.linalg.inv(m['matrix'][:2,:2]).T;z=winguv@m['matrix'][2,:2]+m['offset'][2];valid&=~(Polygon(outline[:,:2]).contains_points(winguv)&(z>=vertices[:,2]))
            ids=np.flatnonzero(valid);ids=ids[image[iy[ids],ix[ids],3]>0];ids=ids[n[ids,2]>best[ids]];best[ids]=n[ids,2];colors[ids,:3]=image[iy[ids],ix[ids],:3];phases[ids]=c['row']['phase'];coordinates[ids]=np.c_[ix[ids],iy[ids]]
        save(part['name'],colors,phases,coordinates,(h,w),np.ones(h*w,bool))
    report=dict(status='PRIVATE_SEGMENTED_OWN_SOURCE_OBSERVATIONS_NOT_APPROVED',animation_index=fit['animation_index'],fit_sha256=sha(a.fit),geometry_sha256=sha(a.geometry),recipe_sha256=sha(Path(__file__)),source_frames=[r['source'] for r in fit['rows']],surfaces=surfaces,uv_domain=[0,8*scale,-7*scale,6*scale],body_parts=parts,limitations=['Exact own-frame RGB readback; unknown surfaces remain gray, no synthesis.','Separate head/neck/thorax/abdomen and wing-depth ownership uses analytic ellipsoid ray tests.','Surface identities remain inferred; mixed source pixels and unresolved head direction require visual review.']);(a.output/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(surfaces))
if __name__=='__main__':main()
