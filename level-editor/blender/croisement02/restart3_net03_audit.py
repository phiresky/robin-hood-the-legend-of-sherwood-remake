"""Independently reopen net03 source receivers and closed-body contacts."""
import json,sys
from pathlib import Path
import bpy,bmesh,numpy as np
from PIL import Image
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from log_trap_state_candidate import point
from tree_geometry import RAY
from render_slots import acquire,release


def main():
 base=OUT/'restart3-net03/endpoints-v4';rows=[r for r in json.loads((OUT/'net-endpoint-volume-fit-v2/manifest.json').read_text())['records']if r['family']=='piege03']
 acquire()
 try:
  for row in rows:
   suffix=row['variant'];folder=base/suffix;report=json.loads((folder/'manifest.json').read_text());assert sha(folder/'model.blend')==report['model_sha256'];source=np.array(Image.open(row['source']).convert('RGBA'));x,y,w,h=row['bbox'];bpy.ops.wm.open_mainfile(filepath=str(folder/'model.blend'));bpy.context.view_layer.update();objects=[];closed=[]
   for obj in bpy.context.scene.objects:
    if obj.type!='MESH':continue
    bm=bmesh.new();bm.from_mesh(obj.data);assert all(e.is_manifold for e in bm.edges);v=bm.calc_volume(signed=True);bm.free();assert v>0;closed.append(dict(object=obj.name,closed_volume=v));tree=BVHTree.FromPolygons([obj.matrix_world@v.co for v in obj.data.vertices],[list(f.vertices)for f in obj.data.polygons]);component='bag' if obj.name.endswith(' bag') else 'wood' if obj.name=='Net03 counterweight' else obj.name.removeprefix('Net03 ');path=folder/(component+'-observed.png');pixels=np.array(Image.open(path).convert('RGBA'))if path.exists()else None
    if pixels is not None:
     known=pixels[:,:,3]>0
     if not np.array_equal(pixels[known],source[known]):raise ValueError('Known RGBA altered')
    objects.append((obj,tree,pixels))
   counts=dict(opaque_source=int((source[:,:,3]>0).sum()),accepted_source=0,unknown_geometry=0,no_geometry=0);issues=[];diagnostic=source.copy()
   for py,px in np.argwhere(source[:,:,3]>0):
    origin=point(x+float(px)+.5,y+float(py)+.5,0)+RAY*5000;hits=[]
    for index,(obj,tree,pixels)in enumerate(objects):
     hit=tree.ray_cast(origin,-RAY)
     if hit[0]is not None:hits.append((hit[3],index,hit[2]))
    if not hits:
     counts['no_geometry']+=1;issues.append(dict(pixel=[int(px),int(py)],reason='no geometry'));diagnostic[py,px]=[255,0,0,255];continue
    _,index,face=min(hits);obj,tree,pixels=objects[index];valid=pixels is not None and pixels[py,px,3]>0 and obj.data.polygons[face].material_index==0
    counts['accepted_source'if valid else 'unknown_geometry']+=1
    if not valid:issues.append(dict(pixel=[int(px),int(py)],reason='unknown first-hit',object=obj.name));diagnostic[py,px]=[255,0,255,255]
   air_checks=[]
   for opening in report.get('source_air_openings',[]):
    for py,px in opening['coordinates']:
     origin=point(x+float(px)+.5,y+float(py)+.5,0)+RAY*5000
     occluders=[obj.name for obj,tree,pixels in objects if tree.ray_cast(origin,-RAY)[0] is not None]
     air_checks.append(dict(pixel=[px,py],occluders=occluders))
   Image.fromarray(diagnostic).resize((w*5,h*5),Image.Resampling.NEAREST).save(folder/'source-first-hit.png')
   bag=bpy.data.objects['Net03 '+suffix+' bag'];wood=bpy.data.objects['Net03 counterweight'];copy=bag.copy();copy.data=bag.data.copy();bpy.context.scene.collection.objects.link(copy);bpy.context.view_layer.objects.active=copy;mod=copy.modifiers.new('Independent body intersection','BOOLEAN');mod.operation='INTERSECT';mod.solver='EXACT';mod.object=wood;bpy.ops.object.modifier_apply(modifier=mod.name);bm=bmesh.new();bm.from_mesh(copy.data);overlap=abs(bm.calc_volume(signed=True));bm.free()
   write_json(folder/'reopened-audit.json',dict(status='numeric audit only; classify native misses and inspect support',model_sha256=report['model_sha256'],counts=counts,issues=issues,closed_objects=closed,native_rgba_unchanged=True,source_air_pixels=len(air_checks),source_air_blocked=sum(bool(r['occluders'])for r in air_checks),source_air_checks=air_checks,bag_wood_overlap_volume=overlap,overlap_tolerance=.001,source_camera_first=True))
 finally:release()
if __name__=='__main__':main()
