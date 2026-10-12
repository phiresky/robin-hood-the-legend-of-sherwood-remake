"""Orthographic source front for inspecting painted repeat features in scene units.

This samples the original materials without editing them. X runs along the
recipe's wall direction, Z up; the nearest negative-Y surface wins.
"""
import io
import json
import sys

import numpy as np
from PIL import Image, ImageDraw
from build_segments import LIB, ROOT, source_geometry


def profile(recipe, entries):
    _,path,doc,buffers,parts,_=source_geometry(recipe,entries)
    points=np.concatenate([a['POSITION'] for a,_,_,_ in parts])
    lo,hi=points.min(0),points.max(0)
    scale=4
    width,height=int((hi[0]-lo[0])*scale)+1,int((hi[2]-lo[2])*scale)+1
    pixels=np.full((height,width,3),40,dtype=np.uint8)
    depth=np.full((height,width),np.inf)
    images=[]
    for item in doc.get('images',[]):
        if 'uri' in item:
            image=Image.open(path.parent/item['uri'])
        else:
            view=doc['bufferViews'][item['bufferView']];offset=view.get('byteOffset',0)
            image=Image.open(io.BytesIO(buffers[view.get('buffer',0)][offset:offset+view['byteLength']]))
        images.append(np.asarray(image.convert('RGB')))
    for attrs,triangles,material,_ in parts:
        texture=doc['materials'][material].get('pbrMetallicRoughness',{}).get('baseColorTexture')
        if texture is None:continue
        image=images[doc['textures'][texture['index']]['source']]
        for tri in triangles:
            p=attrs['POSITION'][tri];uv=attrs['TEXCOORD_0'][tri]
            q=np.stack([(p[:,0]-lo[0])*scale,(hi[2]-p[:,2])*scale],axis=1)
            matrix=np.c_[q[1]-q[0],q[2]-q[0]]
            if abs(np.linalg.det(matrix))<1e-7:continue
            x0,y0=np.maximum(np.floor(q.min(0)).astype(int),0)
            x1,y1=np.minimum(np.ceil(q.max(0)).astype(int),[width-1,height-1])
            ys,xs=np.mgrid[y0:y1+1,x0:x1+1]
            bary=(np.stack([xs,ys],axis=-1)-q[0])@np.linalg.inv(matrix).T
            u,v=bary[:,:,0],bary[:,:,1]
            d=p[0,1]+u*(p[1,1]-p[0,1])+v*(p[2,1]-p[0,1])
            mask=(u>=0)&(v>=0)&(u+v<=1)&(d<depth[y0:y1+1,x0:x1+1])
            coords=uv[0]+u[:,:,None]*(uv[1]-uv[0])+v[:,:,None]*(uv[2]-uv[0])
            tx=np.clip((coords[:,:,0]*image.shape[1]).astype(int),0,image.shape[1]-1)
            ty=np.clip((coords[:,:,1]*image.shape[0]).astype(int),0,image.shape[0]-1)
            pixels[y0:y1+1,x0:x1+1][mask]=image[ty,tx][mask]
            depth[y0:y1+1,x0:x1+1][mask]=d[mask]
    result=Image.fromarray(pixels)
    draw=ImageDraw.Draw(result)
    for x in range(int(lo[0]//20)*20,int(hi[0])+1,20):
        px=int((x-lo[0])*scale)
        draw.line((px,0,px,25),fill='red')
        draw.text((px+2,2),str(x),fill='red')
    folder=ROOT/'work/needs-review/wall-presets/profiles';folder.mkdir(exist_ok=True)
    result.save(folder/(recipe['id']+'.png'))


if __name__=='__main__':
    entries={e['id']:e for e in json.loads((LIB/'index.json').read_text())['assets']}
    for recipe in json.loads((ROOT/'refinement/walls/recipes.json').read_text()):
        if recipe['id'] in sys.argv[1:]:profile(recipe,entries)
