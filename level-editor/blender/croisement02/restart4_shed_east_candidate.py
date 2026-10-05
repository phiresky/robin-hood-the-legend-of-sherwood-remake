"""Private complete east shed continuation beyond the cropped native artwork."""
import json,sys,math
from pathlib import Path
import bpy,bmesh,numpy as np
from PIL import Image
from mathutils import Vector
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,COS,RAY
from scenery_geometry import Mesh
from refinement_workspace import _geometry
from render_multiview_asset import render
from audit_scene_first_hit import full_mask
from refinement_review import _tree

def main():
 asset='croisement02-woodcutters-shed';base=OUT/'restart2-vegetation/shed-package-v1/assets'/asset;old=OUT/'restart2-textures/approved-prop-repairs-fill-v1'/asset/'stored-preparation-v3/experiment/native-front-retained-v1/worker.blend';digest='58a3cbc5183af47d652dff172ec015dc89bd64fc1004c56776367348076c5b24';assert sha(old)==digest
 out=OUT/'restart4-source-gaps/shed-east-v2';out.mkdir(parents=True,exist_ok=False);bpy.ops.wm.open_mainfile(filepath=str(old));bpy.context.preferences.filepaths.save_version=0;scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.window.scene=scene;bpy.context.view_layer.update();original=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH'];before={o.name:_geometry(o,True)for o in original}
 raw=json.loads((OUT/'baseline/Croisement02.rhp.json').read_text())['sight_obstacles'][138]['points'];p=[Vector((t['x'],-t['y']/SIN,t['z_top']/COS))for t in raw];along=p[0]-p[3];along.z=0;along.normalize();extension=along*16;back=p[0]+extension;front=Vector((1794,-431,0));depth=p[0]-p[1];depth.z=0;slope=(p[0].z-p[1].z)/depth.length;depth.normalize();front.z=p[1].z+(front-p[1]).dot(depth)*slope;up=Vector((0,0,1));m=Mesh()
 for j in range(15):
  a=j/15;b=(j+1)/15;q=[p[0].lerp(back,a),p[1].lerp(front,a),p[1].lerp(front,b),p[0].lerp(back,b)];start=len(m.vertices);m.vertices.extend(tuple(t-up*2)for t in q);m.vertices.extend(tuple(t)for t in q);m.faces.extend(tuple(start+i for i in f)for f in[(3,2,1,0),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7)])
 for a,b,count in [(p[1],front,18),(p[0],back,18),(front,back,18)]:
  axis=b-a;axis.z=0;length=axis.length;axis.normalize();side=Vector((-axis.y,axis.x,0))
  for j in range(count):
   q=a.lerp(b,(j+.5)/count);height=q.z-2;q.z=height/2;m.box(q,axis,side,length/count+.03,1.5,height)
 for q in [front,back]:m.tube(Vector((q.x,q.y,0)),q,2.2,n=8)
 mesh=bpy.data.meshes.new('Inferred complete east roof and side return');mesh.from_pydata(m.vertices,[],m.faces);mesh.update();bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));nonmanifold=sum(not e.is_manifold for e in bm.edges);degenerate=sum(f.calc_area()<1e-8 for f in bm.faces);bm.to_mesh(mesh);bm.free();mesh.update();obj=bpy.data.objects.new(mesh.name,mesh);bpy.data.collections['Croisement02 Working'].objects.link(obj)
 for k,v in {'asset_group':asset,'asset_name':'Woodcutters Shed','source_node':'building-138','part_name':'Inferred complete east continuation','projection_component':'east_continuation','source_role':'Native127 extreme-right thatch; complete out-of-map return inferred, original facade unchanged'}.items():obj[k]=v
 cfg=json.loads((base/'source-masks.json').read_text());ip=Path(cfg['mask_inventory']);lookup={r['index']:r for r in json.loads(ip.read_text())['masks']};domain=full_mask(lookup[127],ip)
 for n in [47,88,110]:domain&=~full_mask(lookup[n],ip)
 source=np.array(Image.open(base/'reference/source.png').convert('RGBA'));source[:,:,3]=domain*255;Image.fromarray(source).save(out/'native-domain.png');mat=bpy.data.materials.new('East continuation native127 excluding tree47 and underbrush');mat.use_nodes=True;nodes=mat.node_tree.nodes;nodes.clear();links=mat.node_tree.links;output=nodes.new('ShaderNodeOutputMaterial');tex=nodes.new('ShaderNodeTexImage');tex.image=bpy.data.images.load(str(out/'native-domain.png'));tex.image.pack();tex.interpolation='Closest';tex.extension='CLIP';uvnode=nodes.new('ShaderNodeUVMap');uvnode.uv_map='East continuation native projection';links.new(uvnode.outputs['UV'],tex.inputs['Vector']);mix=nodes.new('ShaderNodeMixRGB');mix.inputs[1].default_value=(.25,.25,.25,1);links.new(tex.outputs['Alpha'],mix.inputs[0]);links.new(tex.outputs['Color'],mix.inputs[2]);links.new(mix.outputs[0],output.inputs['Surface']);gray=bpy.data.materials.new('East continuation unknown reverse');gray.use_nodes=True;nodes2=gray.node_tree.nodes;nodes2.clear();em=nodes2.new('ShaderNodeEmission');em.inputs['Color'].default_value=(.25,.25,.25,1);o2=nodes2.new('ShaderNodeOutputMaterial');gray.node_tree.links.new(em.outputs[0],o2.inputs['Surface']);mesh.materials.append(mat);mesh.materials.append(gray);uv=mesh.uv_layers.new(name=uvnode.uv_map)
 for poly in mesh.polygons:
  poly.material_index=0 if poly.normal.dot(RAY)>.05 else 1
  for li in poly.loop_indices:
   q=mesh.vertices[mesh.loops[li].vertex_index].co;uv.data[li].uv=(q.x/1792,1-(-q.y*SIN-q.z*COS)/1152)
 assert before=={o.name:_geometry(o,True)for o in original};assert nonmanifold==0 and degenerate==0;bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'),compress=True);modelsha=sha(out/'model.blend')
 packet=json.loads((base/'modified/views.json').read_text());packet['object_names']=[o.name for o in original if o.get('asset_group')==asset]+[obj.name];packet.pop('render_object_names',None)
 for view in packet['views']:view['crop']={'width':packet['tile_size'][0],'height':packet['tile_size'][1]};view['ortho_scale']*=1.6
 write_json(out/'cameras.json',packet);scene.cycles.transparent_max_bounces=512;scene.cycles.samples=8;render(out/'cameras.json',out/'actual',width=384);images=[Image.open(out/f'actual/view-{i}-textured.png').convert('RGB')for i in range(8)];w,h=images[0].size;sheet=Image.new('RGB',(w*4,h*2))
 for i,im in enumerate(images):sheet.paste(im,((i%4)*w,(i//4)*h))
 sheet.save(out/'actual/sheet.png')
 crop=[1610,90,1850,305];l,t,r,b=crop;data=bpy.data.cameras.new('Native source east continuation');data.type='ORTHO';data.ortho_scale=r-l;data.clip_end=10000;cam=bpy.data.objects.new(data.name,data);scene.collection.objects.link(cam);scene.camera=cam;center=Vector(((l+r)/2,-(t+b)/2/SIN,0));cam.location=center+RAY*6000;cam.rotation_euler=(center-cam.location).to_track_quat('-Z','Y').to_euler();scene.render.resolution_x=(r-l)*3;scene.render.resolution_y=(b-t)*3;scene.render.resolution_percentage=100;scene.render.film_transparent=True;scene.render.image_settings.color_mode='RGBA'
 for o in scene.objects:
  if o.type=='MESH':o.hide_render=o.get('asset_group')!=asset
 scene.render.filepath=str(out/'native.png');bpy.ops.render.render(write_still=True);native=Image.open(base/'reference/source.png').convert('RGBA').crop(crop).resize(((r-l)*3,(b-t)*3),Image.Resampling.NEAREST);comparison=Image.new('RGBA',((r-l)*6,(b-t)*3),(32,32,32,255));comparison.paste(native,(0,0));comparison.paste(Image.alpha_composite(native,Image.open(out/'native.png').convert('RGBA')),((r-l)*3,0));comparison.convert('RGB').save(out/'source-comparison.png')
 tree,owners,_=_tree([o for o in original+[obj]if o.get('asset_group')==asset]);audit=json.loads((OUT/'restart3-scene-audit/coherent-batch-v3-v1/first-hit/audit.json').read_text());hits=[]
 for n in [12,20]:
  for sample in audit['components'][n-1]['samples']:
   x,y=sample['pixel'];hit,normal,index,dist=tree.ray_cast(Vector((x+.5,-(y+.5)/SIN,0))+RAY*6000,-RAY);hits.append(dict(region=n,pixel=[x,y],object=owners[index].name if hit is not None else None))
 write_json(out/'proposal.json',dict(status='Private source-supported east continuation; geometry/root/contact review pending',model_sha256=modelsha,parent_model_sha256=digest,original_geometry_materials_uv_exact=True,original_objects=list(before),added_object=obj.name,extension_length=16,extension_vector=list(extension),front_end=list(front),back_end=list(back),layout_evidence_sha256=sha(OUT/'restart4-source-gaps/shed-layout-research-v1/report.json'),native_masks=dict(accepted=[127],excluded=[47,88,110]),nonmanifold_edges=nonmanifold,degenerate_faces=degenerate,min_z=min(v.co.z for v in mesh.vertices),coverage=dict(target=416,hits=sum(h['object'] is not None for h in hits),samples=hits),limitations=['Complete east extent and return are inferred beyond original map; not cutoff at1792.','Original filled roof/front/side/stump payloads retained exactly;58a3 texture remains pending its own user decision.','New unknown/reverse geometry gray until separate geometry approval and fill.','Native47 foreground tree excluded from roof/side projection.']))
 assert sha(old)==digest
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
