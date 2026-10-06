"""Assess same-tree leaf RGB reuse without editing geometry, alpha or model files."""
import sys,re,json,hashlib
from pathlib import Path
import bpy,numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement')]
from render_slots import acquire,release

def png(a):
 rgb=a[...,:3];srgb=np.where(rgb<=.0031308,12.92*rgb,1.055*np.maximum(rgb,0)**(1/2.4)-.055);return Image.fromarray(np.rint(np.clip(np.concatenate([srgb,a[...,3:4]],axis=2),0,1)[::-1]*255).astype('uint8'))
def main():
 e=ROOT/'level-editor/work/croisement03-refinement/restart2/texture-batch-v7/croisement03-tree-25/experiment';out=e/'native-leaf-transfer-preflight-v1';assert not out.exists();out.mkdir();model=e/'baked-preserved-v5/worker.blend';sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();before=sha(model);assert before=='324c3f747958806798d729f1f1a4bcc37a022bf95d5e7e580a6e6916b918689f';acquire();bpy.ops.wm.open_mainfile(filepath=str(model));obj=next(o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')=='croisement03-tree-25');mesh=obj.data;ownership=mesh.color_attributes['Source ownership'];records=[]
 for slot,mat in enumerate(mesh.materials):
  if not mat or not mat.get('foliage_physical_opacity'):continue
  flags={ownership.data[i].color[0] for f in mesh.polygons if f.material_index==slot for i in f.loop_indices};assert flags in ({0.},{1.});node=next(n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE' and n.image);a=np.empty(len(node.image.pixels),np.float32);node.image.pixels.foreach_get(a);a=a.reshape(node.image.size[1],node.image.size[0],4);lobe=int(re.search(r'lobe(\d+)',mat.name)[1]);records.append((slot,lobe,flags,mat.name,a))
 rows=[];panels=[]
 for slot,lobe,flags,name,a in records:
  if flags!={0.}:continue
  donors=[v for v in records if v[1]==lobe and v[2]=={1.}];assert len(donors)==1;dslot,_,_,dname,donor=donors[0];same_dimensions=a.shape==donor.shape;alpha_equal=same_dimensions and np.array_equal(a[...,3],donor[...,3]);eligible=(a[...,3]>=.5)&(np.ptp(donor[...,:3],axis=2)>1e-6) if same_dimensions else np.zeros(a.shape[:2],bool);candidate=a.copy()
  if same_dimensions:candidate[eligible,:3]=donor[eligible,:3]
  assert np.array_equal(candidate[...,3],a[...,3]);rows.append(dict(slot=slot,material=name,lobe=lobe,donor_slot=dslot,donor_material=dname,same_dimensions=same_dimensions,alpha_identical=bool(alpha_equal),nonneutral_native_rgb_pixels=int(eligible.sum()),opaque_target_pixels=int((a[...,3]>=.5).sum()),candidate_pixel_changes=int(np.any(candidate!=a,axis=2).sum()),alpha_changes=0,reference='Same tree and same native lobe; no other asset RGB sampled',limitation='Reuses observed leaf colors as inferred rear material. Does not establish hidden foliage arrangement or remove overlap caused by approved geometry.'))
  if lobe==4 and 'transverse' not in name:
   panels=[('Observed same-lobe RGB',donor),('Current generated RGB',a),('Proposed native-RGB control',candidate)];png(candidate).save(out/'lobe04-native-rgb-control.png')
 sheet=Image.new('RGB',(1200,440),(32,32,32));draw=ImageDraw.Draw(sheet)
 for i,(label,a) in enumerate(panels):
  im=png(a);ratio=min(390/im.width,390/im.height);im=im.resize((int(im.width*ratio),int(im.height*ratio)),Image.Resampling.NEAREST);sheet.paste(im,(i*400+5,35),im);draw.text((i*400+8,10),label,fill='white')
 sheet.save(out/'comparison.png');report=dict(status='Read-only preparation/control assessment; no model candidate saved',model_sha256=before,rows=rows,all_unknown_atlases_have_same_lobe_donor=all(x['same_dimensions'] for x in rows),all_unknown_alpha_matches_same_lobe=all(x['alpha_identical'] for x in rows),no_model_or_source_edits=True,limitations=['The control is same-coordinate RGB transfer, not generated new leaf arrangement.','Any neutral donor texels remain unchanged; no copying unknown gray as new evidence.','Appearance must still be tested in all original cameras before accepting material transfer.']);(out/'preflight.json').write_text(json.dumps(report,indent=2)+'\n');assert sha(model)==before;release();print(json.dumps(report))
if __name__=='__main__':main()
