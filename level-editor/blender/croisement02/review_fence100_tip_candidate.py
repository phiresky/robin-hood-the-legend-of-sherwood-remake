"""Inspect the private rail tip at native scale and eight close viewpoints."""
import sys,json,hashlib,math
from pathlib import Path
import bpy
from mathutils import Vector,Matrix
R=Path.cwd();sys.path[:0]=[str(R/'level-editor/refinement'),str(R/'level-editor/refinement/blender')]
from render_slots import acquire,release
from refinement_review import render_review,_tree
B=R/'level-editor/work/active/croisement02';D=B/'static-terrain-closure-oct11-v1/fence100-tip-candidate-v2';A='croisement02-southwest-field-wattle-fence'
def main():
 acquire(slots=2)
 try:
  bpy.ops.wm.open_mainfile(filepath=str(D/'model.blend'));scene=bpy.context.scene;col=bpy.data.collections['Croisement02 Working'];objects=[o for o in col.all_objects if o.type=='MESH'and o.get('asset_group')==A];tip=next(o for o in objects if o.get('projection_component')=='source100-tip-candidate');base=[o for o in objects if o!=tip]
  tree,owners,_=_tree(objects);s=math.sin(math.radians(35));c=math.cos(math.radians(35));ray=Vector((0,-c,s));rows=[]
  for x,y in json.loads((D/'candidate.json').read_text())['native_centers']:
   hit,n,i,d=tree.ray_cast(Vector((x+.5,-(y+.5)/s,0))+ray*6000,-ray);rows.append(dict(pixel=[x,y],object=owners[i].name if hit is not None else None,hit=list(hit)if hit is not None else None))
  tb,_,_=_tree([tip]);bb,_,_=_tree(base);overlap=tb.overlap(bb)if hasattr(tb,'overlap')else None
  frames=json.loads((D/'review/views.json').read_text());center=Vector((571,-(1034.5+28*c)/s,28));frames['tile_size']=[384,384]
  for view in frames['views']:
   matrix=Matrix(view['camera_matrix_world']);matrix.translation=center+matrix.col[2].to_3d()*10000;view['camera_matrix_world']=[list(row)for row in matrix];view['camera_location']=list(matrix.translation);view['ortho_scale']=24
  render_review(D/'close-review',scene_name=scene.name,collection_name=col.name,asset_id=A,source_path=B/'source-states/covered.png',frame_manifest=frames,render_object_names=[o.name for o in objects],allow_projection_revision=True,allow_mask_revision=True)
  (D/'source-fit.json').write_text(json.dumps(dict(rows=rows,tip_triangle_overlap_pairs=len(overlap)if overlap is not None else None,scope='Ray fit and overlap, not manifold connectedness or approval'),indent=2)+'\n')
 finally:release()
if __name__=='__main__':main()
