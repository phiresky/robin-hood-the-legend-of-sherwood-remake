"""Private exact approved wall101 export, preserving frozen part IDs and scene pivot."""
import sys,json,struct,hashlib
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3];R=ROOT/'level-editor/work/croisement02-refinement/restart2-textures';sys.path[:0]=[str(Path(__file__).parent),str(R),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from render_slots import acquire,release
from evidence_io import sha,write_json
from exact_composite_export_v2 import convert,finalize_document
import export_editor
D=OUT/'restart7-wall101-approved-export-v1';MODEL=OUT/'restart7-fence-residual/wall101-appearance-reuse-v1/model.blend';ASSET='croisement02-east-stone-wall-and-gate'
def main():
 D.mkdir(exist_ok=False);assert sha(MODEL)=='3204d38f8b94a3ef78662b4633b7219b131ff8d61450d0fa8b8c562be64b0475';approval=OUT/'restart3-review-batches/batch-v15/user-approval.json';assert sha(approval)=='811890ebd230fc4085dd57d2a2b37600daf23f8841093764a5051e561361ad9d'
 placement=R/'batch10-private-level3d-exact-v2/placement-evidence';doc=json.loads((placement/'expanded-document.json').read_text());mat=json.loads((placement/'part-matrices.json').read_text());rows=[o for o in doc['objects']if o.get('group')==ASSET];parts=sorted(o['id']for o in rows);assert parts==[f'building-{i:03d}'for i in range(5,12)];pivots=[mat[o['id']]['matrix'][12:15]for o in rows];assert all(p==pivots[0]for p in pivots);pivot=pivots[0]
 plan=json.loads((R/'batch10-static-export-plan-v1/plan.json').read_text());cat=json.loads(Path(plan['catalog']).read_text());level=json.loads((OUT/'baseline/Croisement02.rhp.json').read_text());bpy.ops.wm.open_mainfile(filepath=str(MODEL));scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.window.scene=scene;bpy.context.view_layer.update();objects=[o for o in scene.objects if o.type=='MESH'and o.get('asset_group')==ASSET];records=[];original=[]
 for o in objects:
  images={n.image for m in o.data.materials if m and m.use_nodes for n in m.node_tree.nodes if n.type=='TEX_IMAGE'and n.image};original.append(dict(object=o.name,source_node=o.get('source_node'),matrix=[list(r)for r in o.matrix_world],vertices=len(o.data.vertices),polygons=len(o.data.polygons),images=[dict(name=i.name,packed_sha256=hashlib.sha256(bytes(i.packed_file.data)).hexdigest()if i.packed_file else None)for i in images]));o.data=o.data.copy();records.extend(convert(o))
 folder=D/'exact';folder.mkdir();report=export_editor.export_editor('Croisement02',folder/'model.glb',asset_id=ASSET,standalone_pivot=pivot,include_hidden_objects=[o.name for o in objects if o.hide_render],catalog=cat,level=level)
 raw=(folder/'model.glb').read_bytes();length,kind=struct.unpack_from('<II',raw,12);gltf=json.loads(raw[20:20+length]);count=finalize_document(gltf)if records else 0;tail=raw[20+length:];encoded=json.dumps(gltf,separators=(',',':')).encode();encoded+=b' '*(-len(encoded)%4);(folder/'model.glb').write_bytes(struct.pack('<III',0x46546c67,2,20+len(encoded)+len(tail))+struct.pack('<II',len(encoded),kind)+encoded+tail)
 assert sorted(p['node']for p in report['asset']['parts'])==parts;assert sha(MODEL)=='3204d38f8b94a3ef78662b4633b7219b131ff8d61450d0fa8b8c562be64b0475'
 write_json(D/'export.json',dict(status='Private exact GLB export; derivative and actual review pending',source_model=str(MODEL),source_sha256=sha(MODEL),approval_sha256=sha(approval),model_sha256=sha(folder/'model.glb'),asset_id=ASSET,parts=parts,pivot=pivot,source_receivers=original,conversion_records=records,complementary_material_count=count,export=report,authority_files={str(p):sha(p)for p in [placement/'static-placement-pins.json',placement/'part-matrices.json',Path(plan['catalog']),OUT/'restart7-fence-residual/frozen-wall101-appearance-card-v1/root-review.json',OUT/'restart7-fence-residual/frozen-wall101-appearance-card-v1/freeze-receipt.json']},source_model_unchanged=True,publication=False));print(D,flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
