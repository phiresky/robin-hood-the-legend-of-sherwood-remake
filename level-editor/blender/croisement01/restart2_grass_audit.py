"""Audit native physical alpha and protected image bytes of saved grass workers."""
import argparse,json,math,sys
from pathlib import Path
import bpy
import numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from render_slots import acquire
from physical_opacity import OpacityRegistry
from evidence_io import sha


def main():
    parser=argparse.ArgumentParser();parser.add_argument('workspace',type=Path);parser.add_argument('--mask',type=int,required=True);parser.add_argument('--include-branch',action='store_true');args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    worker=args.workspace.resolve();dest=worker/('inspection/branch-foreground-audit' if args.include_branch else 'inspection/native-physical-audit');dest.mkdir(exist_ok=False)
    acquire();bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'));config=json.loads((worker/'workspace.json').read_text())
    obj=next(o for o in bpy.data.collections[config['collection_name']].all_objects if o.type=='MESH' and o.get('asset_group')==config['asset_id'])
    mesh=obj.data;mesh.calc_loop_triangles();points=[];faces=[];registry=OpacityRegistry();known=[]
    for triangle in mesh.loop_triangles:
        start=len(points);points.extend(obj.matrix_world@mesh.vertices[i].co for i in triangle.vertices);faces.append((start,start+1,start+2));registry.add(obj,mesh,triangle);known.append(bool(mesh.materials[triangle.material_index].get('foliage_observed')))
    target_triangle_count=len(faces)
    if args.include_branch:
        branch=next(o for o in bpy.data.collections[config['collection_name']].all_objects if o.type=='MESH' and o.get('asset_group')=='croisement01-east-fallen-branch')
        bm=branch.data;bm.calc_loop_triangles()
        for triangle in bm.loop_triangles:
            start=len(points);points.extend(branch.matrix_world@bm.vertices[i].co for i in triangle.vertices);faces.append((start,start+1,start+2));registry.add(branch,bm,triangle);known.append(False)
    tree=registry.wrap(BVHTree.FromPolygons(points,faces,all_triangles=True))
    base=ROOT/'level-editor/work/croisement01-refinement/baseline';native=json.loads((base/'masks/manifest.json').read_text());row=next(r for r in native['masks'] if r['index']==args.mask)
    expected=np.array(Image.open(base/'masks'/row['png']).convert('L'))>127;x,y=row['box_top_left'];h,w=expected.shape
    sine,cosine=math.sin(math.radians(35)),math.cos(math.radians(35));ray=Vector((0,-cosine,sine));actual=np.zeros_like(expected);observed=np.zeros_like(expected);foreground=np.zeros_like(expected)
    for py in range(h):
        for px in range(w):
            target=Vector((x+px+.5,-(y+py+.5)/sine,0));hit,normal,index,distance=tree.ray_cast(target+ray*5000,-ray,10000)
            if hit is not None:actual[py,px]=True;observed[py,px]=known[index];foreground[py,px]=index<target_triangle_count
    pixels=np.array(Image.open(worker.parent.parent/'source/native.png').convert('RGBA'));source=np.array(Image.open(base/'covered.png').convert('RGBA').crop((x,y,x+w,y+h)));source[:,:,3]=np.array(Image.open(base/'masks'/row['png']).convert('L'))
    image_exact=bool(np.array_equal(pixels,source))
    diagnostic=np.zeros((h,w,3),np.uint8);diagnostic[actual&expected]=(180,180,180);diagnostic[expected&~actual]=(255,40,40);diagnostic[actual&~expected]=(0,200,255);Image.fromarray(diagnostic).resize((w*6,h*6),Image.Resampling.NEAREST).save(dest/'coverage.png')
    report=dict(status='measurement only; independent visual review still required',model_sha256=sha(worker/'model.blend'),native_mask=args.mask,expected_pixels=int(expected.sum()),missing_pixels=int((expected&~actual).sum()),extra_pixels_within_bbox=int((actual&~expected).sum()),observed_material_first_hits=int((observed&expected).sum()),native_source_rgba_exact=image_exact,target_foreground_pixels=int((foreground&expected).sum()),target_hidden_by_branch_pixels=int((expected&actual&~foreground).sum()),include_branch=args.include_branch,limitations=['Native occupancy is the declared candidate domain; this audit does not classify its pixels semantically.','Does not establish out-of-domain oblique appearance or neighboring-asset visibility.'])
    (dest/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))
    if not image_exact:raise ValueError('Native source RGBA changed')


if __name__=='__main__':main()
