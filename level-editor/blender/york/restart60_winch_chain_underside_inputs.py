"""Supplement approved chain with underneath views, retaining source camera at index zero."""
import hashlib,json,shutil,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/york-refinement/restart2';I=W/'restart60-winch-chain-texture-inputs-v1';O=W/'restart60-winch-chain-underside-inputs-v1';assert not O.exists()
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
for row in v['views'][1:]:
 old=Matrix(row['camera_matrix_world']);back=old.to_3x3().col[2];dist=(old.translation-center).dot(back);target=old.translation-back*dist;location=old.translation.copy();location.z=2*target.z-location.z;rotation=(target-location).to_track_quat('-Z','Y');matrix=rotation.to_matrix().to_4x4();matrix.translation=location;row['camera_matrix_world']=[list(r) for r in matrix];row['camera_location']=list(location);row['camera_rotation_euler']=list(rotation.to_euler());row['supplementary_elevation_degrees']=-35
O.mkdir();(O/'model.blend').symlink_to(I/'model.blend');(O/'supplementary-cameras.json').write_text(json.dumps(v,indent=2)+'\n')
render_review(O/'modified',scene_name=scene.name,collection_name=v['collection_name'],asset_id=report['render_asset'],source_path=I/'source-frame44.png',frame_manifest=v,source_mask_manifest=I/'source-masks.json',projection_layers=v['projection_layers'],render_object_names=names)
report['supplementary_policy']='Native camera0 unchanged; views1..7 mirrored below target to supply only previously unseen underside textures. No geometry or source domain changed.';report['source_counts']=[r['counts']['source'] for r in json.loads((O/'modified/views.json').read_text())['views']];(O/'input-review.json').write_text(json.dumps(report,indent=2)+'\n')
print(O)
