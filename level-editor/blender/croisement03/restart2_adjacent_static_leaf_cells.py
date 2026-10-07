"""Private static foliage source cells behind separately retained native animation."""
import hashlib,json,math,shutil,sys
from pathlib import Path
import bpy,numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from render_views import render_views
B=ROOT/'level-editor/work/croisement03-refinement/restart2'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main(tree):
 assert tree in (12,14);assert shutil.disk_usage(ROOT).free>25*1024**3;out=B/f'tree{tree}-static-leaf-crown-v1';out.mkdir(exist_ok=False);proposal=B/'trio-tree-integration-v1/static-leaf-source-proposal-v2';source=B/f'tree{tree}-approved-wood-texture-v1/source-restored-fill-v1/worker.blend';guards={str(source):sha(source),str(proposal/'scope.json'):sha(proposal/'scope.json')};acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.data.scenes['Croisement03 Refinement'];bpy.context.window.scene=scene;before={o.name:hashlib.sha256(np.array([tuple(v.co) for v in o.data.vertices],np.float32).tobytes()).hexdigest() for o in scene.objects if o.type=='MESH'};images={im.name:hashlib.sha256(np.asarray(im.pixels[:],np.float32).tobytes()).hexdigest() for im in bpy.data.images if im.has_data};mask=np.array(Image.open(proposal/f'tree{tree}-candidate-static-foliage.png'))>0;yy,xx=np.nonzero(mask);box=[int(xx.min()),int(yy.min()),int(xx.max()+1),int(yy.max()+1)];impath=out/'proposed-static-source.png';Image.open(proposal/f'tree{tree}-proposed-source-rgba.png').crop(box).save(impath);im=bpy.data.images.load(str(impath));im.pack();mat=bpy.data.materials.new(f'Tree{tree} proposed static native foliage');mat.use_nodes=True;nodes=mat.node_tree.nodes;nodes.clear();tex=nodes.new('ShaderNodeTexImage');tex.image=im;tex.interpolation='Closest';tex.extension='CLIP';em=nodes.new('ShaderNodeEmission');trans=nodes.new('ShaderNodeBsdfTransparent');mix=nodes.new('ShaderNodeMixShader');end=nodes.new('ShaderNodeOutputMaterial');links=mat.node_tree.links;links.new(tex.outputs['Color'],em.inputs[0]);links.new(tex.outputs['Alpha'],mix.inputs[0]);links.new(trans.outputs[0],mix.inputs[1]);links.new(em.outputs[0],mix.inputs[2]);links.new(mix.outputs[0],end.inputs[0]);vertices=[];faces=[];uvs=[];sin,cos=math.sin(math.radians(35)),math.cos(math.radians(35));x0=946 if tree==12 else 1084;base=-400 if tree==12 else -310
  for x,y in zip(xx,yy):
   localx=x-x0;depth=base-42*math.sin(localx*.127+y*.079)-17*math.cos(localx*.263-y*.117)+3;ids=[]
   for px,py in ((x,y),(x+1,y),(x+1,y+1),(x,y+1)):
    ids.append(len(vertices));vertices.append((float(px),depth,(-depth*sin-py)/cos));uvs.append(((px-box[0])/(box[2]-box[0]),1-(py-box[1])/(box[3]-box[1])))
   faces.append(ids)
  mesh=bpy.data.meshes.new('Proposed static leaf source cells');mesh.from_pydata(vertices,[],faces);mesh.materials.append(mat);uv=mesh.uv_layers.new(name='UVMap')
  for f in mesh.polygons:
   for li in f.loop_indices:uv.data[li].uv=uvs[mesh.loops[li].vertex_index]
  o=bpy.data.objects.new(f'Tree{tree} proposed static canopy leaf samples',mesh);scene.collection.objects.link(o);o['asset_group']=f'croisement03-arbre06-fragment-tree{tree}-provisional';o['source_role']='NEW proposed STATIC foreground foliage; not animated Arbre06';o['source_proposal_sha256']=sha(proposal/'scope.json');o['geometry_approval']='NEW scope, pending user review';assert all(hashlib.sha256(np.array([tuple(v.co) for v in scene.objects[n].data.vertices],np.float32).tobytes()).hexdigest()==h for n,h in before.items());assert all(hashlib.sha256(np.asarray(bpy.data.images[n].pixels[:],np.float32).tobytes()).hexdigest()==h for n,h in images.items());scene.cycles.transparent_max_bounces=128;bpy.data.libraries.write(str(out/'worker.blend'),{scene},fake_user=True,compress=True);render_views(scene.name,{f'view-{i}':f'Tree13 view{i}' for i in range(8)},out/'actual',modes=('textured','solid'),width=384)
  for mode in ('textured','solid'):
   sheet=Image.new('RGB',(1536,768),'#333333')
   for i in range(8):
    frame=Image.open(out/'actual'/f'view-{i}-{mode}.png').convert('RGBA');bg=Image.new('RGBA',frame.size,'#333333');bg.alpha_composite(frame);sheet.paste(bg.convert('RGB'),((i%4)*384,(i//4)*384))
   sheet.save(out/'actual'/f'{mode}.png')
  (out/'receipt.json').write_text(json.dumps(dict(status='PRIVATE proposed static foliage geometry, not approved',source_guards=guards,model_sha256=sha(out/'worker.blend'),new_static_faces=len(faces),new_static_source_box=box,prior_geometry_vertices_exact=True,prior_images_rgba_exact=True,native_view_index=0,limits=['Existing approved wood and inferred crown components retained; added static sample surfaces are a fresh geometry scope.','Source material/membership proposal must be reviewed, then exact native first-hit guards including new samples.','No API, terrain fill or live integration. Separate dynamic geometry/state ownership remains unproven.']),indent=2)+'\n')
 finally:release()
if __name__=='__main__':main(int(sys.argv[sys.argv.index('--')+1]))
