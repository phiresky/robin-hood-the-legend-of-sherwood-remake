"""Match compact source workers to the frozen whole-scene receiver authority."""
import sys,json
from pathlib import Path
from collections import Counter
import bpy
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
from restart6_source_gap_audit import ROOT,OUT
from approved_texture_stage import geometry,appearance
from evidence_io import sha,write_json,digest
from render_slots import acquire,release
acquire()
try:
 selection=json.load(open(OUT/'restart2-textures/batch-v3-coherent-selection-v1/selection.json'))['records'];pins=json.load(open(OUT/'restart2-textures/batch10-linked-static-v1/source-pins.json'))['receivers'];proof=json.load(open(OUT/'restart2-textures/batch-v3-coherent-scene-v1/reopened-preservation.json'));records=[]
 for tree in [24,38]:
  asset=f'croisement02-tree-{tree}';row=next(r for r in selection if r['asset_id']==asset);source=Path(row['model']);assert sha(source)==row['model_sha256'];expected={r['object_name']:proof['receivers'][r['object_name']]for r in pins.values()if r['asset_group']==asset};bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.view_layer.update();actual={o.name:dict(geometry=geometry(o),appearance=appearance(o))for o in bpy.context.scene.objects if o.type=='MESH'and o.get('asset_group')==asset};equal=Counter(digest(v)for v in actual.values())==Counter(digest(v)for v in expected.values());records.append(dict(asset_id=asset,source=str(source),source_sha256=sha(source),scene_sha256=proof['model_sha256'],exact_geometry_appearance_world=equal,actual=actual,expected=expected));print(asset,equal,flush=True)
 write_json(ROOT/'small-tree-source-equivalence-v1.json',dict(records=records,scope='Read-only exact receiver fingerprints, ignoring import-generated datablock names only. No model or catalog edits.'))
finally:release()
