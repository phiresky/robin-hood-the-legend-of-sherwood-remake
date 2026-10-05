"""Remove numerically collapsed faces from the already closed hay contour result."""
import argparse,json,sys
from pathlib import Path
import bpy,bmesh
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release

def main():
 parser=argparse.ArgumentParser();parser.add_argument('--raw',type=Path,required=True);parser.add_argument('--destination',type=Path,required=True);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:]);old=args.raw;dest=args.destination;dest.mkdir(exist_ok=False);digest=sha(old/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(old/'model.blend'));bpy.context.view_layer.update();result={}
 for obj in bpy.data.collections['Croisement02 Working'].all_objects:
  if obj.type!='MESH' or obj.get('asset_group')!=old.name:continue
  bm=bmesh.new();bm.from_mesh(obj.data);before=dict(vertices=len(bm.verts),faces=len(bm.faces),degenerate_faces=sum(f.calc_area()<1e-8 for f in bm.faces));bmesh.ops.remove_doubles(bm,verts=list(bm.verts),dist=.0001);bmesh.ops.dissolve_degenerate(bm,dist=.0001,edges=list(bm.edges));
  for iteration in range(3):
   tiny=[f for f in bm.faces if f.calc_area()<1e-8]
   if tiny:bmesh.ops.dissolve_faces(bm,faces=tiny,use_verts=True)
   wire=[e for e in bm.edges if not e.link_faces]
   if wire:bmesh.ops.delete(bm,geom=wire,context='EDGES')
   loose=[v for v in bm.verts if not v.link_edges]
   if loose:bmesh.ops.delete(bm,geom=loose,context='VERTS')
   bmesh.ops.dissolve_degenerate(bm,dist=.0001,edges=list(bm.edges))
  bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));after=dict(vertices=len(bm.verts),faces=len(bm.faces),nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),degenerate_faces=sum(f.calc_area()<1e-8 for f in bm.faces));result[obj['source_node']]=dict(before=before,after=after);bm.to_mesh(obj.data);bm.free()
 write_json(dest/'topology.json',dict(parent_model_sha256=digest,result=result,merge_distance=.0001));bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(dest/'model.blend'));print(result)
 if any(v['after']['nonmanifold_edges'] or v['after']['degenerate_faces'] for v in result.values()):raise ValueError('Cleaned hay still invalid; diagnostic only')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
