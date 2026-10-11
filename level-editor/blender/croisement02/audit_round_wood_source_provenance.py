"""Reproduce approved source atlases exactly to recover categorical ownership.

Neutral projection atlases have opaque alpha; their material's legacy alpha
label cannot establish source provenance. Reproduce the pinned projection and
require exact pixels and UVs before using its explicit ownership buffers.
"""
from pathlib import Path
import sys,json,hashlib,shutil
import bpy,numpy as np
R=Path.cwd();sys.path[:0]=[str(R/'level-editor/refinement'),str(R/'level-editor/refinement/blender')]
from render_slots import acquire,release
from source_projection_bake import bake
from refinement_workspace import _geometry
n=int(sys.argv[sys.argv.index('--')+1]);assert n in (32,38);B=R/'level-editor/work/croisement02-refinement';E=B/f'approved-round-wood-textures-oct11-v1/tree{n}/experiment';D=E/'source-authority-v2';sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();read=lambda p:json.loads(p.read_text())
def pixels(im):
 p=np.empty(len(im.pixels),np.float32);im.pixels.foreach_get(p);return p
acquire(slots=2)
try:
 if int(next(l.split()[1]for l in Path('/proc/meminfo').read_text().splitlines()if l.startswith('MemAvailable:')))*1024<6*1024**3 or shutil.disk_usage(R).free<10*1024**3:raise RuntimeError('Resource guard')
 assert not D.exists();D.mkdir();meta=read(E/'views.json');model=E/'approved-model.blend';digest=sha(model);bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.data.scenes[meta['scene_name']];bpy.context.window.scene=scene;names=meta['object_names'];before={o.name:_geometry(o,protect_appearance=True)for o in scene.objects if o.type=='MESH'and o.name not in names};geometry={name:_geometry(scene.objects[name])for name in names};original={};objects=[scene.objects[name]for name in names]
 for ob in objects:
  mats={p.material_index for p in ob.data.polygons};assert len(mats)==1;mat=ob.data.materials[next(iter(mats))];node=next(x for x in mat.node_tree.nodes if x.type=='TEX_IMAGE');uvname=node.inputs['Vector'].links[0].from_node.uv_map;original[ob.name]=dict(pixels=pixels(node.image),uv=np.array([v.uv[:]for v in ob.data.uv_layers[uvname].data]),uvname=uvname,image_sha256=hashlib.sha256(bytes(node.image.packed_file.data)).hexdigest(),image_name=node.image.name)
 report=bake('Croisement02',meta['source_image'],D/'projection.json',receiver_object_names=names,receiver_face_indices={ob.name:set(range(len(ob.data.polygons)))for ob in objects},material_suffix='local-field-only',projection_label='exterior',elevation_deg=35.,preserve_authored=False,source_mask_manifest=meta['source_mask_manifest'],provenance_directory=D/'provenance')
 records={}
 for ob in objects:
  data=original[ob.name];mat=ob.data.materials[ob.data.polygons[0].material_index];node=next(x for x in mat.node_tree.nodes if x.type=='TEX_IMAGE');uvname=node.inputs['Vector'].links[0].from_node.uv_map
  assert np.array_equal(pixels(node.image),data['pixels']),('Source reproduction differs',ob.name)
  assert uvname==data['uvname'] and np.array_equal(np.array([v.uv[:]for v in ob.data.uv_layers[uvname].data]),data['uv'])
  assert _geometry(ob)==geometry[ob.name]
  row=next(r for r in report['objects']if r['object']==ob.name);proof=row['texel_provenance'];assert sha(Path(proof['path']))==proof['sha256'];flags=np.load(proof['path'])['ownership'];assert (flags==1).any()and(flags==0).any();records[ob.name]=dict(**proof,original_packed_image_sha256=data['image_sha256'],original_image_name=data['image_name'],exact_reproduced_rgba=True,exact_uv=True,known_texels=int((flags==1).sum()))
 assert before=={o.name:_geometry(o,protect_appearance=True)for o in scene.objects if o.name in before};assert sha(model)==digest
 (D/'authority.json').write_text(json.dumps(dict(status='PASS_EXACT_SOURCE_REPRODUCTION',model_sha256=digest,records=records,limitation='Original alpha is physical opacity, not provenance; explicit reproduced flags establish known source. No model saved.'),indent=2)+'\n');print('PASS',list(records))
finally:release()
