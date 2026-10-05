"""Reopen the attached net and independently check source receivers and solid contacts."""
import json,sys
from pathlib import Path
import bpy,bmesh,numpy as np
from PIL import Image
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent)]
from catalog import OUT
from log_trap_state_candidate import sha,point
from tree_geometry import RAY


def main():
    base=OUT/('restart2-state/'+(sys.argv[sys.argv.index('--candidate')+1] if '--candidate' in sys.argv else 'net-empty01-v5'));original=base
    report=json.loads((base/'report.json').read_text());assert sha(base/'model.blend')==report['model_sha256']
    source_row=next(r for r in json.loads((OUT/'net-endpoint-volume-fit-v2/manifest.json').read_text())['records']if r['family']=='piege01'and r['variant']=='e')
    source=np.asarray(Image.open(source_row['source']).convert('RGBA'));x,y,w,h=source_row['bbox']
    bpy.ops.wm.open_mainfile(filepath=str(base/'model.blend'));scene=bpy.context.scene;objects=[];closed=[]
    for obj in scene.objects:
        if obj.type!='MESH':continue
        bm=bmesh.new();bm.from_mesh(obj.data);assert all(e.is_manifold for e in bm.edges);volume=bm.calc_volume(signed=True);bm.free();assert volume>0
        tree=BVHTree.FromPolygons([obj.matrix_world@v.co for v in obj.data.vertices],[list(p.vertices)for p in obj.data.polygons]);path=original/('bag-observed.png' if obj.name=='Empty bag' else obj.name.replace(' ','-')+'-observed.png')
        pixels=np.asarray(Image.open(path).convert('RGBA'))if path.exists()else None
        if pixels is not None:
            known=pixels[:,:,3]>0;assert np.array_equal(pixels[:,:,:3][known],source[:,:,:3][known])
        objects.append((obj,tree,pixels));closed.append(dict(object=obj.name,volume=volume))
    counts=dict(opaque_source=int((source[:,:,3]>0).sum()),accepted_source=0,unknown_gray_geometry=0,no_geometry=0)
    missing=[]
    for py,px in np.argwhere(source[:,:,3]>0):
        origin=point(x+float(px)+.5,y+float(py)+.5,0)+RAY*5000;hits=[]
        for index,(obj,tree,pixels)in enumerate(objects):
            hit=tree.ray_cast(origin,-RAY)
            if hit[0]is not None:hits.append((hit[3],index,hit[2]))
        if not hits:counts['no_geometry']+=1;missing.append([int(px),int(py)]);continue
        _,index,face=min(hits);obj,tree,pixels=objects[index]
        key='accepted_source'if pixels is not None and pixels[py,px,3]>0 and obj.data.polygons[face].material_index==0 else 'unknown_gray_geometry';counts[key]+=1
    bag=bpy.data.objects['Empty bag'];wood=bpy.data.objects['Wooden piece'];copy=bag.copy();copy.data=bag.data.copy();scene.collection.objects.link(copy);bpy.context.view_layer.objects.active=copy
    mod=copy.modifiers.new('Independent intersection probe','BOOLEAN');mod.operation='INTERSECT';mod.solver='EXACT';mod.object=wood;bpy.ops.object.modifier_apply(modifier=mod.name)
    bm=bmesh.new();bm.from_mesh(copy.data);overlap=abs(bm.calc_volume(signed=True));bm.free()
    result=dict(status=('PASS scoped source union and closed-body checks; appearance/support review remains separate' if overlap<1e-3 else 'HOLD Boolean intersection exceeds fixed numerical tolerance'),model_sha256=report['model_sha256'],counts=counts,objects=closed,bag_wood_overlap_volume=overlap,overlap_tolerance=1e-3,missing_source_crop_coordinates=missing,source_rgb_unchanged=True,limitations=['Cord/wood contact is inferred and requires the joint scene review.','Local cloth indentation is a new geometry hypothesis.','This is one empty endpoint, not a complete net state implementation.'])
    (base/'reopened-audit.json').write_text(json.dumps(result,indent=2)+'\n');print(result)


if __name__=='__main__':main()
