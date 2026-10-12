"""Reproject generated bark onto only the faces with audited visible gaps."""
import json,shutil,sys
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from review_evidence import sha
from bake_reviewed_asset import stage
R=ROOT/'level-editor/work/active/croisement01/restart2';W=R/'approved-tree08-bark-fill-v3/croisement01-tree-08';E=W/'experiment';src=W/'baked-v3-native-exact';out=W/'baked-v5-bounded-gaps'
acquire(slots=2)
try:
 assert shutil.disk_usage(R).free>=10*1024**3
 assert next(int(x.split()[1]) for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:'))>=6*1024**2
 audit=json.loads((src/'visible-fill-audit/report.json').read_text());assert audit['model_sha256']==sha(src/'worker.blend')
 faces=sorted({r[2] for v in audit['views'] for r in v['unfilled']});assert 0<len(faces)<1000
 bpy.ops.wm.open_mainfile(filepath=str(src/'worker.blend'));obj=next(o for o in bpy.context.scene.objects if o.type=='MESH')
 manifest=json.loads((E/'baked-v1-sampling-views.json').read_text());manifest['texture_receiver_face_indices']={obj.name:faces};manifest['texture_generated_bounded_visibility']={obj.name:faces};manifest['texture_material_suffix']='visible-gap-v1'
 path=E/'baked-v5-bounded-sampling-views.json';assert not path.exists();path.write_text(json.dumps(manifest,indent=2)+'\n')
 G=E/'generation-short-no-mask-with-lighting-openrouter-with-auxiliary';bpy.context.scene.cycles.samples=8
 report=stage(path,G/'generated-preserved.png',out,texels_per_unit=4,reconciliation_reference=G/'generated-raw.png')
 (out/'scope.json').write_text(json.dumps({'status':'SCOPED_BAKE_AWAITING_REVIEW','parent_model_sha256':sha(src/'worker.blend'),'model_sha256':sha(out/'worker.blend'),'audited_faces':faces,'audit_sha256':sha(src/'visible-fill-audit/report.json'),'policy':'Finite visibility rays and increased sampling only on exact faces with visible class-zero pixels. Same approved generated sheet. Unselected atlas/materials retained.'},indent=2)+'\n');print('BOUNDED_BAKE',len(faces),report['counts'],flush=True)
finally:release()
