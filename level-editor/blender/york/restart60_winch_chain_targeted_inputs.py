"""Supplement approved chain with targeted residual views, retaining source camera at index zero."""
import hashlib,json,shutil,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/active/york/restart2';I=W/'restart60-winch-chain-texture-inputs-v1';O=W/'restart60-winch-chain-targeted2-inputs-v1';assert not O.exists()
assert shutil.disk_usage(ROOT).free>10*1024**3;assert int(next(l.split()[1] for l in Path('/proc/meminfo').read_text().splitlines() if l.startswith('MemAvailable:')))*1024>6*1024**3
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire
acquire(slots=2)
import bpy
from mathutils import Vector,Matrix
from refinement_review import render_review
bpy.ops.wm.open_mainfile(filepath=str(I/'model.blend'));scene=bpy.context.scene;scene.render.threads_mode='FIXED';scene.render.threads=2
report=json.loads((I/'input-review.json').read_text());names=report['object_names'];points=[o.matrix_world@v.co for o in scene.objects if o.name in names for v in o.data.vertices];center=Vector([(min(p[i] for p in points)+max(p[i] for p in points))/2 for i in range(3)])
v=json.loads((I/'modified/views.json').read_text())
audit=json.loads((W/'restart60-winch-chain-combined-v1/remaining-visibility.json').read_text());selection=json.loads((W/'restart60-winch-chain-combined-v1/targeted-camera-selection.json').read_text());v['tile_size']=[640,768]
for row,i in zip(v['views'][1:],selection['direction_indices'][1:]):
 old=Matrix(row['camera_matrix_world']);back=old.to_3x3().col[2];dist=(old.translation-center).dot(back);target=old.translation-back*dist;direction=Vector(audit['directions'][i]);location=target+direction*dist;rotation=(-direction).to_track_quat('-Z','Y');matrix=rotation.to_matrix().to_4x4();matrix.translation=location;row['camera_matrix_world']=[list(r) for r in matrix];row['camera_location']=list(location);row['camera_rotation_euler']=list(rotation.to_euler());row['residual_direction_index']=i
 inv=matrix.inverted();projected=[inv@p for p in points];xmin,xmax=min(p.x for p in projected),max(p.x for p in projected);ymin,ymax=min(p.y for p in projected),max(p.y for p in projected);matrix.translation+=matrix.to_3x3()@Vector(((xmin+xmax)/2,(ymin+ymax)/2,0));row['camera_matrix_world']=[list(r) for r in matrix];row['camera_location']=list(matrix.translation);row['ortho_scale']=max((xmax-xmin)*768/640,ymax-ymin)*1.06
O.mkdir();(O/'model.blend').symlink_to(I/'model.blend');(O/'supplementary-cameras.json').write_text(json.dumps(v,indent=2)+'\n')
render_review(O/'modified',scene_name=scene.name,collection_name=v['collection_name'],asset_id=report['render_asset'],source_path=I/'source-frame44.png',frame_manifest=v,source_mask_manifest=I/'source-masks.json',projection_layers=v['projection_layers'],render_object_names=names)
report['supplementary_policy']='Native camera0 unchanged; views1..7 selected to cover remaining visible points at twice input resolution. No geometry or source domain changed.';report['source_counts']=[r['counts']['source'] for r in json.loads((O/'modified/views.json').read_text())['views']];(O/'input-review.json').write_text(json.dumps(report,indent=2)+'\n')
print(O)
