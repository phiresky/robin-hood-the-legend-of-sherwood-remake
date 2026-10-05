"""Identify the saved faces underneath independently measured unfilled review pixels."""
import argparse
import json
from pathlib import Path
import sys
ROOT=Path(__file__).resolve().parents[3]
BASE=ROOT/'level-editor/work/york-refinement/restart2/pair-textures-v1'
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--grazing',action='store_true')
args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
import numpy as np
from mathutils import Matrix,Vector
from mathutils.bvhtree import BVHTree
from PIL import Image
for asset,experiment,bake in [('york-market-southeast-tall-narrow-house','experiment-v2-shadow','bake-v2-shadow'),('york-southwest-square-west-house','experiment','bake-v1')]:
    if args.grazing:
        experiment,bake=('experiment-v3-grazing','bake-v3-grazing') if 'tall-narrow' in asset else ('experiment-v2-grazing','bake-v2-grazing')
    work=BASE/asset;manifest=json.loads((work/experiment/'views.json').read_text())
    bpy.ops.wm.open_mainfile(filepath=str(work/bake/'model.blend'))
    scene=bpy.data.scenes['york Refinement'];bpy.context.window.scene=scene;bpy.context.view_layer.update()
    objects=[o for o in scene.objects if o.type=='MESH' and not o.hide_render and o.get('asset_group')==asset]
    vertices=[];triangles=[];owners=[];normals={}
    for obj in objects:
        offset=len(vertices);vertices.extend(obj.matrix_world@v.co for v in obj.data.vertices);obj.data.calc_loop_triangles()
        for t in obj.data.loop_triangles:
            triangles.append(tuple(offset+i for i in t.vertices));owners.append((obj.name,t.polygon_index))
        for face in obj.data.polygons:normals[(obj.name,face.index)]=(obj.matrix_world.to_3x3()@face.normal).normalized()
    tree=BVHTree.FromPolygons(vertices,triangles,all_triangles=True);rows=[]
    for view in manifest['views']:
        im=np.asarray(Image.open(work/bake/'coverage-v1'/f"view-{view['index']}-textured.png").convert('RGBA'))
        rgb=im[:,:,:3].astype(float);red=(im[:,:,3]>127)&(rgb[:,:,0]>80)&(rgb[:,:,0]>2*rgb[:,:,1])&(rgb[:,:,0]>2*rgb[:,:,2])
        if not red.any():continue
        height,width=red.shape;matrix=Matrix(view['camera_matrix_world']);direction=matrix.to_3x3()@Vector((0,0,1));scale=view['ortho_scale']
        # Orthographic scale describes the longer image edge.
        horizontal=scale if width>=height else scale*width/height
        vertical=scale if height>=width else scale*height/width
        hits={};precision={};plane=max(v.dot(direction) for v in vertices)+1
        for y,x in np.argwhere(red):
            origin=matrix@Vector((((x+.5)/width-.5)*horizontal,(.5-(y+.5)/height)*vertical,0))
            hit,normal,index,distance=tree.ray_cast(origin,-direction)
            if index is not None:
                owner=owners[index];hits[owner]=hits.get(owner,0)+1
                row=precision.setdefault(owner,{'long_ray_rejected':0,'bounded_ray_rejected':0,'long_ray_max_error':0.,'bounded_ray_max_error':0.})
                for label,origin in [('long',hit+direction*100000),('bounded',hit+direction*(plane-hit.dot(direction)))]:
                    other,_,other_index,_=tree.ray_cast(origin,-direction)
                    error=(other-hit).length if other is not None else 1e9
                    row[label+'_ray_max_error']=max(row[label+'_ray_max_error'],error)
                    if other_index is None or owners[other_index]!=owner or error>.02:row[label+'_ray_rejected']+=1
        for owner,count in hits.items():
            rows.append({'view':view['index'],'object':owner[0],'face':owner[1],'red_pixels':count,'normal':list(normals[owner]),
                'camera_cosines':[normals[owner].dot(Matrix(v['camera_matrix_world']).to_3x3()@Vector((0,0,1))) for v in manifest['views']],
                'precision':precision[owner]})
    target=work/bake/('gap-faces-precision.json' if args.grazing else 'gap-faces.json')
    if target.exists():raise FileExistsError(target)
    target.write_text(json.dumps({'asset':asset,'faces':rows,'method':'First target geometry hit at explicitly red coverage pixels; diagnostic only, not source ownership authority.'},indent=2)+'\n');print(target)
