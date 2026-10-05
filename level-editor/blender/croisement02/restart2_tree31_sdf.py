"""Continuous lower wood volume from native centreline radii, without tube caps."""
import json
from pathlib import Path
import numpy as np
from scipy.ndimage import gaussian_filter
from skimage.measure import marching_cubes
ROOT=Path(__file__).resolve().parents[3];OUT=ROOT/'level-editor/work/croisement02-refinement'

def main():
    out=OUT/'restart2-wood/tree31-sdf-v3';out.mkdir(exist_ok=False)
    record=next(r for r in json.loads((OUT/'forest-v4-sources/manifest.json').read_text()) if r['mask']==31);ground=record['ground_y'];sin=np.sin(np.deg2rad(35));cos=np.cos(np.deg2rad(35))
    paths=next(r for r in json.loads((OUT/'wood-traces.json').read_text()) if r['mask']==31)['paths'];nodes=[]
    for path in paths:
        for a,b in zip(path,path[1:]):
            a=np.array([a[0],-ground/sin,(ground-a[1])/cos,a[2]]);b=np.array([b[0],-ground/sin,(ground-b[1])/cos,b[2]])
            for t in np.linspace(0,1,max(2,int(np.linalg.norm(b[:3]-a[:3])/1.5)+1)):
                p=a*(1-t)+b*t
                if p[2]<=112:nodes.append(p)
    nodes=np.array(nodes);radii=nodes[:,3]*1.01+.25;minimum=np.floor(np.min(nodes[:,:3]-radii[:,None],axis=0)-5);maximum=np.ceil(np.max(nodes[:,:3]+radii[:,None],axis=0)+5);spacing=.65;shape=np.ceil((maximum-minimum)/spacing).astype(int)+1
    field=np.full(tuple(shape),30.,np.float32)
    for p,radius in zip(nodes,radii):
        lo=np.maximum(0,np.floor((p[:3]-radius-4-minimum)/spacing).astype(int));hi=np.minimum(shape,np.ceil((p[:3]+radius+4-minimum)/spacing).astype(int)+1)
        grid=np.ogrid[lo[0]:hi[0],lo[1]:hi[1],lo[2]:hi[2]];distance=np.sqrt(sum((grid[d]*spacing+minimum[d]-p[d])**2 for d in range(3)))-radius;sl=tuple(slice(lo[d],hi[d]) for d in range(3));field[sl]=np.minimum(field[sl],distance)
    field=gaussian_filter(field,sigma=.8/spacing);vertices,faces,_,_=marching_cubes(field,0,spacing=(spacing,)*3,gradient_direction='ascent');vertices+=minimum
    np.savez_compressed(out/'lower-volume.npz',vertices=vertices,faces=faces)
    (out/'sdf-evidence.json').write_text(json.dumps(dict(native_mask=31,source='Existing native31 centreline/radius trace; hidden transverse depth inferred',nodes=len(nodes),grid_shape=shape.tolist(),spacing=spacing,smoothing_world_sigma=.8,vertices=len(vertices),faces=len(faces),native_ground_y=ground,limitations=['No generated textures or canonical edits.','Low roots retain existing source-ray depth pending ground/source review.']),indent=2)+'\n')
    print(out)
if __name__=='__main__':main()
