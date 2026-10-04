"""Audit a saved private scene with physical-alpha-aware source-camera first hits."""
import argparse
import json
import sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from refinement_review import _tree
from tree_geometry import SIN,RAY


def full_mask(row,inventory_path):
    image=Image.open(inventory_path.parent/row['png']).convert('L');x,y=row['box_top_left'];w,h=image.size
    result=np.zeros((1152,1792),bool)
    result[max(y,0):min(y+h,1152),max(x,0):min(x+w,1792)]=np.asarray(image)[max(-y,0):min(h,1152-y),max(-x,0):min(w,1792-x)]>0
    return result


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('stage',type=Path);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    stage=args.stage.resolve();output=stage/'first-hit'
    if output.exists():raise FileExistsError(output)
    output.mkdir();report=json.loads((stage/'assembly.json').read_text())
    if sha(stage/'scene.blend')!=report['model_sha256']:raise ValueError('Scene changed')
    bpy.ops.wm.open_mainfile(filepath=str(stage/'scene.blend'));scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.window.scene=scene;bpy.context.view_layer.update()
    objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and not o.hide_render]
    names=sorted({o.get('asset_group') or o.get('source_node') or o.name for o in objects});indices={name:i for i,name in enumerate(names)}
    tree,owners,_=_tree(objects);labels=np.full((1152,1792),-1,np.int16)
    triangle_labels=np.array([indices[o.get('asset_group') or o.get('source_node') or o.name] for o in owners],np.int16)
    for y in range(1152):
        for x in range(1792):
            origin=Vector((x+.5,-(y+.5)/SIN,0))+RAY*5000
            hit,normal,index,distance=tree.ray_cast(origin,-RAY)
            if hit is not None:labels[y,x]=triangle_labels[index]
        if y%128==0:print('FIRST-HIT ROW',y,flush=True)
    np.savez_compressed(output/'source-first-hit.npz',labels=labels,names=np.array(names))
    counts={name:int((labels==i).sum()) for i,name in enumerate(names)}
    ground='croisement02-ground-receiver';known=np.asarray(Image.open(OUT/'ground-receiver-review-v5/reference/ground-observed-domain.png'))>0
    ground_covered=known&(labels!=indices[ground])
    native_path=OUT/'review-mask-inventory.json';native={r['index']:r for r in json.loads(native_path.read_text())['masks']}
    shared=full_mask(native[119],native_path)&full_mask(native[23],native_path)
    shared_owners={names[int(i)] if i>=0 else 'unassigned':int((shared&(labels==i)).sum()) for i in np.unique(labels[shared])}
    palette=np.array([[(i*83+59)%206+35,(i*149+17)%206+35,(i*47+101)%206+35] for i in range(len(names))],np.uint8)
    color=np.zeros((1152,1792,3),np.uint8);good=labels>=0;color[good]=palette[labels[good]]
    Image.fromarray(color).save(output/'first-hit-owners.png')
    source=np.asarray(Image.open(OUT/'ground-receiver-review-v5/reference/source.png').convert('RGB'));difference=source.copy();difference[ground_covered]=[255,0,0]
    Image.fromarray(difference).save(output/'observed-ground-covered.png')
    fern=source.copy();fern[shared]=color[shared];Image.fromarray(fern).crop((560,0,680,110)).resize((960,880)).save(output/'fern119-tree23-owners.png')
    legend=Image.new('RGB',(1100,max(100,len(names)*22)), '#242424');draw=ImageDraw.Draw(legend)
    for i,name in enumerate(names):draw.rectangle((0,i*22,20,i*22+20),fill=tuple(map(int,palette[i])));draw.text((28,i*22+4),f'{name}: {counts[name]} pixels',fill='white')
    legend.save(output/'owner-legend.png')
    write_json(output/'audit.json',dict(status='measured; visual interpretation pending',model_sha256=report['model_sha256'],method='Exact1792x1152 source pixel-center rays against saved mesh BVH with shared physical-alpha cutout sampling and one-sided foliage culling. No metadata-only occluder skipping.',visible_objects=len(objects),triangles=len(owners),first_hit_pixels=counts,unassigned_pixels=int((labels<0).sum()),observed_ground_pixels=int(known.sum()),observed_ground_covered_by_scene=int(ground_covered.sum()),fern119_tree23=dict(shared_native_pixels=int(shared.sum()),actual_first_hit_owners=shared_owners,claim='Physical scene visibility only. Does not remove stale fern paint from approved tree23 cached material or grant a new texture approval.'),images={p.name:sha(p) for p in output.glob('*.png')},limitations=['Source ownership masks overlap in places; physical first-hit is reported separately from ownership permission.','Covered observed-ground pixels can reflect inferred scenery protrusion and need regional review; they are not automatically discarded.','Four state-only metadata groups and their pending actual state assets remain absent in base-scene renders.']))


if __name__=='__main__':main()
