"""Export approved wood and separate private Arbre06-provenance crown roles."""
import hashlib,json,shutil,struct,sys
from pathlib import Path
import bpy,numpy as np
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).resolve().parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from export_editor import export_editor
from evidence_io import sha,write_json
from restart2_adjacent_export_composite_v2 import flatten_normal_gate,finalize_document
from restart2_export_approved_static_pair import crown
B=ROOT/'level-editor/work/croisement03-refinement/restart2'
def main(tree):
 assert tree in (10,11);planpath=B/'approved-tree10-11-integration-v1/plan.json';plan=json.loads(planpath.read_text());entry=next(r for r in plan['rows'] if r['asset_id']==f'croisement03-tree-{tree}');out=planpath.parent/f'full-export-v1/tree{tree}';assert not out.exists()
 for p,h in plan['pins'].items():assert sha(p)==h,p
 assert shutil.disk_usage(ROOT).free>=10*1024**3;available=int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024;assert available>=6*1024**3
 acquire()
 try:
  source=Path(entry['source_model']);bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.data.scenes[entry['source_scene']];bpy.context.window.scene=scene;scene.render.threads_mode='FIXED';scene.render.threads=2;collection=bpy.data.collections[entry['working_collection']];asset=entry['asset_id'];wood=[o for o in scene.objects if o.type=='MESH' and o.get('asset_group')==asset];leaves=[o for o in scene.objects if o.type=='MESH' and o.get('asset_group')==entry['foliage_group']];assert len(wood)==(1 if tree==10 else 3) and len(leaves)==1
  assert sorted(o.get('source_node') for o in wood)==entry['wood_source_nodes']
  images={im.name:hashlib.sha256(np.asarray(im.pixels[:],np.float32).tobytes()).hexdigest() for im in bpy.data.images if im.has_data};records=[flatten_normal_gate(o) for o in wood];crowns=[crown(leaves[0],'dynamic-frame0-provenance',entry['native_crown_faces'],asset,collection)]
  for o in wood+leaves:o['asset_name']=f'North tree{tree}';o['part_name']=o.get('part_name',o.name);o.hide_render=False
  assert images=={name:hashlib.sha256(np.asarray(bpy.data.images[name].pixels[:],np.float32).tobytes()).hexdigest() for name in images};out.mkdir(parents=True)
  write_json(out/'views.json',[dict(camera_matrix=[list(r) for r in bpy.data.objects[f'Tree13 view{i}'].matrix_world],ortho_scale=bpy.data.objects[f'Tree13 view{i}'].data.ortho_scale) for i in range(8)]);export=export_editor('Croisement03',out/'model.glb',asset_id=asset)
  raw=(out/'model.glb').read_bytes();length,kind=struct.unpack_from('<II',raw,12);doc=json.loads(raw[20:20+length]);finalize_document(doc)
  for mat in doc['materials']:
   if mat.get('extras',{}).get('private_foliage_alpha'):mat.update(alphaMode='MASK',alphaCutoff=.5,doubleSided=True)
  assert len([m for m in doc['materials'] if m.get('extras',{}).get('crown_source_role')=='dynamic-frame0-provenance'])==1
  assert not any(m.get('extras',{}).get('crown_source_role')=='static-native-samples' for m in doc['materials'])
  encoded=json.dumps(doc,separators=(',',':')).encode();encoded+=b' '*(-len(encoded)%4);tail=raw[20+length:];(out/'model.glb').write_bytes(struct.pack('<III',0x46546c67,2,20+len(encoded)+len(tail))+struct.pack('<II',len(encoded),kind)+encoded+tail)
  assert sha(source)==entry['source_model_sha256'];assert (out/'model.glb').stat().st_size<=8*1024**2;assert sum(p.stat().st_size for p in out.rglob('*') if p.is_file())<=32*1024**2
  write_json(out/'report.json',dict(status='PRIVATE conversion complete; independent packed-material, native, surface and visual proofs pending',source_sha256=sha(source),model_sha256=sha(out/'model.glb'),plan_sha256=sha(planpath),crowns=crowns,records=records,export=export,limits=['Full frame0 crown is a private provenance role, not a static animation replacement.','No canonical changes; current gameplay metadata must be preserved separately.','No source fidelity claim before independent exported content and first-hit proof.']))
 finally:release()
if __name__=='__main__':main(int(sys.argv[sys.argv.index('--')+1]))
