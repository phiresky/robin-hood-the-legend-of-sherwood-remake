"""Private same-lobe native RGB control with immutable physical/source guards."""
import sys,re,json,hashlib
from pathlib import Path
from array import array
import bpy,numpy as np
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from refinement_workspace import _geometry
from bake_reviewed_asset import _materials
from render_multiview_asset import render
from refinement_review import _tile

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def pixels(im):
 a=np.empty(len(im.pixels),np.float32);im.pixels.foreach_get(a);return a.reshape(im.size[1],im.size[0],4)
def main():
 e=ROOT/'level-editor/work/croisement03-refinement/restart2/texture-batch-v7/croisement03-tree-25/experiment';src=e/'baked-preserved-v5';out=e/'native-rgb-control-v1';assert not out.exists();assert sha(src/'worker.blend')=='324c3f747958806798d729f1f1a4bcc37a022bf95d5e7e580a6e6916b918689f';preflight=e/'native-leaf-transfer-preflight-v1/preflight.json';plan=json.loads(preflight.read_text());assert plan['all_unknown_alpha_matches_same_lobe'] and plan['all_unknown_atlases_have_same_lobe_donor'];acquire();bpy.ops.wm.open_mainfile(filepath=str(src/'worker.blend'));bpy.context.preferences.filepaths.save_version=0;manifest=json.loads((e/'views-grid8-v4.json').read_text());scene=bpy.data.scenes[manifest['scene_name']];asset='croisement03-tree-25';obj=next(o for o in scene.objects if o.type=='MESH' and o.get('asset_group')==asset);mesh=obj.data
 geometry={o.name:_geometry(o) for o in scene.objects};outside={o.name:_materials(o) for o in scene.objects if o.type=='MESH' and o.get('asset_group')!=asset};uv={u.name:[list(x.uv) for x in u.data] for u in mesh.uv_layers};flags_before=[tuple(x.color) for x in mesh.color_attributes['Source ownership'].data];slots_before=[f.material_index for f in mesh.polygons]
 images={im.name:hashlib.sha256(pixels(im).tobytes()).hexdigest() for im in bpy.data.images if im.size[0] and im.size[1]};records=[]
 for slot,mat in enumerate(mesh.materials):
  if not mat or not mat.get('foliage_physical_opacity'):continue
  faces=[f for f in mesh.polygons if f.material_index==slot];flags={mesh.color_attributes['Source ownership'].data[i].color[0] for f in faces for i in f.loop_indices};assert flags in ({0.},{1.});node=next(n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE' and n.image);records.append((slot,int(re.search(r'lobe(\d+)',mat.name)[1]),flags,mat,node,pixels(node.image)))
 changes=[];offmap=[];known=[]
 for slot,lobe,flags,mat,node,before in records:
  if flags=={1.}:known.append(slot);continue
  if lobe==8:offmap.append(slot);continue
  donors=[r for r in records if r[1]==lobe and r[2]=={1.}];assert len(donors)==1;donor=donors[0];assert before.shape==donor[5].shape and np.array_equal(before[...,3],donor[5][...,3]);eligible=(before[...,3]>=.5)&(np.ptp(donor[5][...,:3],axis=2)>1e-6);after=before.copy();after[eligible,:3]=donor[5][eligible,:3];assert np.array_equal(after[...,3],before[...,3]) and np.array_equal(after[~eligible],before[~eligible]);replacement=mat.copy();im=node.image.copy();im.pixels.foreach_set(after.ravel());im.pack();replacement.node_tree.nodes[node.name].image=im;replacement['texture_review_status']='private-native-rgb-control';replacement['native_rgb_donor_material']=donor[3].name;mesh.materials[slot]=replacement;stored=pixels(im);assert np.array_equal(stored[...,3],before[...,3]) and np.array_equal(stored[~eligible],before[~eligible]);assert np.max(np.abs(stored[eligible,:3]-after[eligible,:3]))<=1/255+1e-7;changes.append(dict(slot=slot,lobe=lobe,donor_slot=donor[0],donor_material=donor[3].name,eligible_native_rgb_texels=int(eligible.sum()),changed_rgb_texels=int(np.any(after[...,:3]!=before[...,:3],axis=2).sum()),physical_alpha_changes=0,known_face_changes=0))
 assert len(changes)==24 and len(offmap)==4 and len(known)==9
 assert geometry=={o.name:_geometry(o) for o in scene.objects};assert outside=={o.name:_materials(o) for o in scene.objects if o.name in outside};assert uv=={u.name:[list(x.uv) for x in u.data] for u in mesh.uv_layers};assert flags_before==[tuple(x.color) for x in mesh.color_attributes['Source ownership'].data];assert slots_before==[f.material_index for f in mesh.polygons]
 assert all(hashlib.sha256(pixels(bpy.data.images[name]).tobytes()).hexdigest()==digest for name,digest in images.items())
 for slot in known+offmap:assert mesh.materials[slot]==next(r[3] for r in records if r[0]==slot)
 out.mkdir();bpy.ops.wm.save_as_mainfile(filepath=str(out/'worker.blend'));render(e/'views-grid8-v4.json',out/'actual',width=384);buffers=[]
 for i in range(8):
  im=bpy.data.images.load(str(out/'actual'/f'view-{i}-textured.png'),check_existing=False);buf=array('f',[0])*len(im.pixels);im.pixels.foreach_get(buf);buffers.append(buf);bpy.data.images.remove(im)
 _tile(buffers,384,384,out/'actual/textured.png');report=dict(status='Mechanical PASS; private appearance control pending review',source_model_sha256=sha(src/'worker.blend'),model_sha256=sha(out/'worker.blend'),actual_sheet_sha256=sha(out/'actual/textured.png'),preflight_sha256=sha(preflight),recipe_sha256=sha(Path(__file__)),geometry_verified=True,uv_unchanged=True,source_ownership_unchanged=True,face_material_indices_unchanged=True,all_preexisting_image_rgba_unchanged=True,known_foliage_slots_unchanged=known,offmap_lobe08_slots_unchanged=offmap,outside_objects_unchanged=len(outside),physical_alpha_changes=0,changes=changes,limitations=['Native RGB is reused as inferred rear material, not evidence of actual unseen leaf arrangement.','Offmap lobe08 retains previous inferred appearance unchanged.','Geometry/UV/physical leaf coverage preserved; wind and actual terrain/neighbor joints unfinished.']);(out/'validation.json').write_text(json.dumps(report,indent=2)+'\n');release();print(json.dumps(report))
if __name__=='__main__':main()
