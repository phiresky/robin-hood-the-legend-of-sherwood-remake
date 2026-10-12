"""Project reviewed native stone interiors onto saved rock faces in a private derivative."""
import sys,json,hashlib
from pathlib import Path
import bpy,numpy as np
from PIL import Image,ImageDraw
from scipy.ndimage import binary_opening
from mathutils import Vector
from mathutils.bvhtree import BVHTree
sys.path.insert(0,str(Path(__file__).resolve().parent))
import restart2_bank_lofts_saved as guard
import restart2_bank_full_v1 as base
B=guard.B;INPUT=B/'bank-boulder-fronts-saved-v3';OUT=B/'bank-boulder-source-saved-v2'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 global INPUT,OUT
 if '--input' in sys.argv:INPUT=Path(sys.argv[sys.argv.index('--input')+1]).resolve()
 if '--out' in sys.argv:OUT=Path(sys.argv[sys.argv.index('--out')+1]).resolve()
 assert not OUT.exists();guard.resources();guard.acquire(slots=2)
 try:
  guard.resources();OUT.mkdir();model=INPUT/'worker.blend';input_model_digest=sha(model);bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene
  sourcepath=B.parent/'baseline/covered.png';source=np.array(Image.open(sourcepath).convert('RGBA'));old=np.array(Image.open(B/'bank-exposed-rock-proposal-v1/candidate-rock-seeds.png'))>0;proposal=np.array(Image.open(B/'bank-face-source-proposal-v1/proposed-native-stone.png'))>0
  # Thin isolated color matches remain unassigned even inside a reviewed stone region.
  broad=binary_opening(proposal,structure=np.ones((3,3),bool));faces=json.loads((B/'bank-visible-face-traces-v1/faces.json').read_text())['faces'];expected={};perface=[]
  for f in faces:
   im=Image.new('L',(source.shape[1],source.shape[0]));ImageDraw.Draw(im).polygon([tuple(p)for p in f['native_outline']],fill=255);domain=(np.array(im)>0)&proposal
   perface.append(dict(id=f['id'],owner=f['owner'],proposed=int(domain.sum()),thin_or_boundary_rejected=int((domain&~broad).sum()),accepted=0,ray_rejected=[]))
   for y,x in zip(*np.nonzero(domain&broad)):expected[(int(x),int(y))]=(int(f['owner']),len(perface)-1)
  extra_proof=None
  if '--extra-proposal' in sys.argv:
   extra_path=Path(sys.argv[sys.argv.index('--extra-proposal')+1]).resolve();extra=json.loads(extra_path.read_text());assert extra['source_sha256']==sha(sourcepath);extra_proof={'path':str(extra_path),'sha256':sha(extra_path),'regions':[]}
   for region in extra['regions']:
    path=extra_path.parent/(region['id']+'.png');domain=np.array(Image.open(path))>0;assert int(domain.sum())==region['pixels'];perface.append(dict(id=region['id'],owner=region['owner'],proposed=int(domain.sum()),thin_or_boundary_rejected=0,accepted=0,ray_rejected=[]));extra_proof['regions'].append({'path':str(path),'sha256':sha(path)})
    for y,x in zip(*np.nonzero(domain&~old&~broad)):expected[(int(x),int(y))]=(int(region['owner']),len(perface)-1)
    broad|=domain
  full_source_proof=None;full_source_hits=[]
  if '--full-source-proposal' in sys.argv:
   full_path=Path(sys.argv[sys.argv.index('--full-source-proposal')+1]).resolve();full=json.loads((full_path/'receipt.json').read_text())
   for path,digest in full['pins'].items():assert sha(Path(path))==digest,path
   domain=np.array(Image.open(full_path/'additional-interior-proposal.png'))>0;native=np.array(Image.open(full_path/'native-foreground.png'))>0;ambiguous=np.array(Image.open(full_path/'manual-ambiguous.png'))>0
   assert int(domain.sum())==17773 and not np.any(domain&(native|ambiguous|old))
   old_accepted=json.loads((B/'bank-main-masses-source-v1/projection.json').read_text())['accepted_pixels'];assert not any(domain[y,x]for x,y in old_accepted)
   perface.append(dict(id='broader-reviewed-stone-interiors',owner=None,proposed=int(domain.sum()),thin_or_boundary_rejected=0,accepted=0,ray_rejected=[]))
   for y,x in np.argwhere(domain):
    expected[(int(x),int(y))]=(None,len(perface)-1)
   broad|=domain;full_source_proof={'path':str(full_path/'receipt.json'),'sha256':sha(full_path/'receipt.json'),'root_review_sha256':sha(full_path/'root-review.json'),'mask_sha256':sha(full_path/'additional-interior-proposal.png'),'samples':17773,'scope':'Private source projection experiment; current receiver is a hypothesis, not final component ownership.'}
  unresolved={};unresolved_receipt=None;unresolved_hits=[]
  if '--unresolved-receivers' in sys.argv:
   up=Path(sys.argv[sys.argv.index('--unresolved-receivers')+1]).resolve();ud=json.loads(up.read_text())
   for name,digest in ud['pins'].items():assert sha(Path(name))==digest,name
   unresolved={tuple(r['source_xy']):r for r in ud['failures']};assert len(unresolved)==102
   for xy,row in unresolved.items():
    assert row['expected_owner']=='53' and row['new_owner']=='52' and row['result']=='different_bank_owner'
    assert not old[xy[1],xy[0]] and xy in expected
   construction=json.loads((INPUT/'interface-construction.json').read_text())
   assert ud['pins'][construction['geometry_plan']]==construction['geometry_plan_sha256']
   revised=construction['private_revised53'];assert ud['pins'][revised['path']]==revised['sha256']
   unresolved_receipt={'path':str(up),'sha256':sha(up),'status':'Private source-continuity experiment only; all102 receiver identities remain unresolved.'}
  rows=[];geometry={}
  for obj in scene.objects:
   if obj.type!='MESH':continue
   mesh=obj.data;mesh.calc_loop_triangles();tri=list(mesh.loop_triangles);verts=[obj.matrix_world@v.co for v in mesh.vertices];rows.append((obj,BVHTree.FromPolygons(verts,[list(t.vertices)for t in tri],all_triangles=True),tri));geometry[obj.name]=([list(v)for v in verts],[list(p.vertices)for p in mesh.polygons])
  assignments={};accepted=[];oldhits=[]
  for y,x in zip(*np.nonzero(old|broad)):
   x,y=int(x),int(y);origin=Vector((x+.5,-(y+.5)/base.S,0))+base.RAY*5000;hits=[]
   for obj,bvh,tri in rows:
    p,n,f,d=bvh.ray_cast(origin,-base.RAY)
    if p is not None:hits.append((d,obj.name,tri[f].polygon_index))
   first=min(hits)if hits else None
   if old[y,x]:
    assert first and first[1].startswith('Candidate bank '),(x,y,first);oldhits.append([x,y,first[1],first[2]])
   else:
    owner,idx=expected[(x,y)]
    if (x,y)in unresolved:
     row=unresolved[(x,y)];assert first and first[1]=='Candidate bank '+row['new_owner'],(x,y,first)
     unresolved_hits.append({'source_xy':[x,y],'previous_private_receiver':owner,'saved_preview_receiver':first[1],'saved_face':first[2],'source_rgba':source[y,x].tolist(),'owner_status':'unresolved'})
    elif owner is None:
     if not first or not first[1].startswith('Candidate bank '):perface[idx]['ray_rejected'].append([x,y,None if first is None else first[1]]);continue
     full_source_hits.append({'source_xy':[x,y],'saved_preview_receiver':first[1],'saved_face':first[2],'source_rgba':source[y,x].tolist(),'owner_status':'proposed_current_receiver_not_final_owner'})
    elif not first or first[1]!=f'Candidate bank {owner}':perface[idx]['ray_rejected'].append([x,y,None if first is None else first[1]]);continue
    perface[idx]['accepted']+=1;accepted.append([x,y])
   assignments.setdefault((first[1],first[2]),[]).append((x,y))
  for (name,face),pixels in assignments.items():
   obj=scene.objects[name];xs,ys=zip(*pixels);l,t,r,b=min(xs),min(ys),max(xs)+1,max(ys)+1;rgba=source[t:b,l:r].copy();rgba[:,:,3]=0
   for x,y in pixels:rgba[y-t,x-l,3]=255
   path=OUT/f'{name.replace(" ","-")}-face-{face}.png';Image.fromarray(rgba).save(path);im=bpy.data.images.load(str(path));im.pack();mat=bpy.data.materials.new(f'Private reviewed stone {name}/{face}');mat.use_nodes=True;nodes=mat.node_tree.nodes;nodes.clear();tex=nodes.new('ShaderNodeTexImage');tex.image=im;tex.interpolation='Closest';tex.extension='CLIP';em=nodes.new('ShaderNodeEmission');gray=nodes.new('ShaderNodeBsdfPrincipled');gray.inputs['Base Color'].default_value=(.25,.25,.25,1);mix=nodes.new('ShaderNodeMixShader');out=nodes.new('ShaderNodeOutputMaterial');links=mat.node_tree.links;links.new(tex.outputs['Color'],em.inputs['Color']);links.new(tex.outputs['Alpha'],mix.inputs[0]);links.new(gray.outputs[0],mix.inputs[1]);links.new(em.outputs[0],mix.inputs[2]);links.new(mix.outputs[0],out.inputs['Surface']);obj.data.materials.append(mat);obj.data.polygons[face].material_index=len(obj.data.materials)-1;uv=obj.data.uv_layers.active or obj.data.uv_layers.new(name="Native stone coordinates")
   for li in obj.data.polygons[face].loop_indices:
    v=obj.matrix_world@obj.data.vertices[obj.data.loops[li].vertex_index].co;uv.data[li].uv=((v.x-l)/(r-l),1-(-v.y*base.S-v.z*base.C-t)/(b-t))
  assert len(unresolved_hits)==len(unresolved)
  retained_proof=None
  if '--retained-projection' in sys.argv:
   prior_path=Path(sys.argv[sys.argv.index('--retained-projection')+1]).resolve();prior=json.loads(prior_path.read_text());missing={tuple(p)for p in prior['accepted_pixels']}-{tuple(p)for p in accepted};retained_proof={'path':str(prior_path),'sha256':sha(prior_path),'samples':len(prior['accepted_pixels']),'missing':sorted(missing)};assert not missing,('Previously assigned exact-source interiors lost',retained_proof)
  if extra_proof:
   previous=json.loads((B/'bank-convex-source-saved-v1/projection.json').read_text());assert {tuple(p)for p in previous['accepted_pixels']}<={tuple(p)for p in accepted},'Previously retained stone sample lost'
  bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'worker.blend'));assert (OUT/'worker.blend').stat().st_size<8*1024**2
  bpy.ops.wm.open_mainfile(filepath=str(OUT/'worker.blend'))
  for name,(verts,faces0)in geometry.items():
   obj=bpy.context.scene.objects[name];assert [list(obj.matrix_world@v.co)for v in obj.data.vertices]==verts;assert [list(p.vertices)for p in obj.data.polygons]==faces0
  assert sha(model)==input_model_digest;assert len(oldhits)==3446
  provenance_path=INPUT/'interface-construction.json';provenance=json.loads(provenance_path.read_text());provenance['inherited_geometry_provenance']={'path':str(provenance_path),'sha256':sha(provenance_path),'reason':'Material-only derivative; saved vertex/face arrays independently asserted identical.'};(OUT/'interface-construction.json').write_text(json.dumps(provenance,indent=2)+'\n')
  report=dict(status='PRIVATE_SOURCE_PROJECTION_REQUIRES_SAVED_VISUAL_REVIEW',input_model_sha256=input_model_digest,model_sha256=sha(OUT/'worker.blend'),source_sha256=sha(sourcepath),protected3446_unchanged_geometry=True,protected_firsthits=oldhits,proposed=2516+(extra['total']if extra_proof else 0)+(17773 if full_source_proof else 0),accepted=len(accepted),accepted_pixels=accepted,extra_proposal=extra_proof,full_source_proposal=full_source_proof,full_source_receiver_hits=full_source_hits,retained_projection=retained_proof,unresolved_receiver_evidence=unresolved_receipt,unresolved_receivers=unresolved_hits,thin_or_boundary_rejected=int((proposal&~broad).sum()),faces=perface,limits=['Only directly reviewed broad stone interiors assigned experimentally; thin matches and wrong receivers remain unknown.','Opening is a conservative rejection filter, not independent ownership evidence.','Manual extra interiors remain subject to exact first-hit owner rejection; original5015 samples must survive.','Any explicit unresolved102 receiver list is a color-continuity preview, not an ownership assignment or approval.','No canonical writes or final appearance approval.']);(OUT/'projection.json').write_text(json.dumps(report,indent=2)+'\n');print({k:report[k]for k in ['status','accepted','thin_or_boundary_rejected']})
 finally:guard.release()
if __name__=='__main__':main()
