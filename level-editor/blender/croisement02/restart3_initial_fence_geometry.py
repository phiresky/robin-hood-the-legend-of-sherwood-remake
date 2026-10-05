"""Bounded initial-fence member correction; state geometry and ground remain separate."""
import sys,json,math
from pathlib import Path
import bpy,numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from render_slots import acquire,release
from evidence_io import sha,write_json
from tree_geometry import SIN,COS,RAY
from feedback_geometry import wattle_top
from review_bank_candidate import camera
from restore_ground75_source import geometry
from refinement_review import _tree
BASE=OUT/'texture-fill-round-1/croisement02-south-field-wattle-fence/experiment/bake-v1/worker.blend'
DEST=OUT/'restart3-initial-fence/geometry-v5'

def main():
 DEST.mkdir(exist_ok=False);assert sha(BASE)=='faaaa33ac0ad34c09af5ae65bcf244f8add09bd6a4630888c9a946f96b912d15'
 survey=json.loads((OUT/'restart3-initial-fence/member-survey-v1/survey.json').read_text())
 domain=np.array(Image.open(OUT/'feedback-wattle-domains/visible-weave.png').convert('L'))>0
 roles=np.array(Image.open(OUT/'restart3-initial-fence/source-domain-v1/fence-source.png').convert('L'))>0
 bottom={}
 for x in range(1018,1171):
  yy=np.flatnonzero(domain[811:940,x])+811
  bottom[x]=float(yy.max()) if len(yy) else wattle_top(x)+25
 bottom={x:float(np.percentile([bottom[k] for k in range(max(1018,x-8),min(1170,x+8)+1)],25)) for x in bottom}
 bpy.ops.wm.open_mainfile(filepath=str(BASE));bpy.context.preferences.filepaths.save_version=0;bpy.context.view_layer.update()
 scene=bpy.context.scene;objects=[bpy.data.objects[r['object']]for r in survey['objects']];before={o.name:geometry(o)for o in objects}
 sourcepath=OUT/'animation-references/composite-frame-0.png';source=bpy.data.images.load(str(sourcepath),check_existing=False);source.pack()
 Image.fromarray(roles.astype('uint8')*255).save(DEST/'source-domain.png');mask=bpy.data.images.load(str(DEST/'source-domain.png'),check_existing=False);mask.colorspace_settings.name='Non-Color';mask.pack()
 edits=[]
 for rec,obj in zip(survey['objects'],objects):
  changed=set();inv=obj.matrix_world.inverted();oldworld=[obj.matrix_world@v.co for v in obj.data.vertices]
  for component in rec['components']:
   ids=component['vertices'];lo,hi=np.array(component['bounds']);cx=component['center'][0]
   if hi[0]<1018 or lo[0]>1170:continue
   post=hi[2]-lo[2]>15
   if post and not 1025<cx<1165:continue
   for index in ids:
    p=oldworld[index].copy();weight=min(1,max(0,(p.x-1018)/8),max(0,(1170-p.x)/8))
    if weight<=0:continue
    if post:
     center=Vector(component['center']);post_scale={1047:2.0,1058:1.8,1076:1.55,1092:1.55,1114:1.55,1129:1.55,1155:1.55}[round(cx)];scale=1+(post_scale-1)*weight;p.x=center.x+(p.x-center.x)*scale;p.y=center.y+(p.y-center.y)*scale
     lean={1047:-12,1058:-3}.get(round(cx),0)*(1-max(0,min(1,p.z/hi[2])));p.x+=lean;p.y-=lean*.5018
    else:
     sy=-p.y*SIN-p.z*COS;top=wattle_top(p.x);fraction=(sy-top)/21.5
     desired=(top-1)*(1-fraction)+(bottom[int(np.clip(round(p.x),1018,1170))]-1)*fraction
     p.z=p.z-weight*(desired-sy)/COS
    if (p-oldworld[index]).length>1e-7:obj.data.vertices[index].co=inv@p;changed.add(index)
   minimum=min((obj.matrix_world@obj.data.vertices[i].co).z for i in ids)
   if minimum<0 and not post:
    for index in ids:
     p=obj.matrix_world@obj.data.vertices[index].co;p.z-=minimum;obj.data.vertices[index].co=inv@p;changed.add(index)
   edits.append(dict(object=obj.name,kind='post-width' if post else 'weave-height',center_x=cx,vertices=len(ids)))
  obj.data.update();uv=obj.data.uv_layers.new(name='Bounded native source projection')
  for loop in obj.data.loops:
   p=obj.matrix_world@obj.data.vertices[loop.vertex_index].co;uv.data[loop.index].uv=(p.x/1792,1-(-p.y*SIN-p.z*COS)/1152)
  materials={}
  for face in obj.data.polygons:
   if not any(i in changed for i in face.vertices) or (obj.matrix_world.to_3x3()@face.normal).normalized().dot(RAY)<.1:continue
   oldindex=face.material_index
   if oldindex not in materials:
    mat=obj.data.materials[oldindex].copy();mat.name+=' / bounded native reprojection';nodes=mat.node_tree.nodes;links=mat.node_tree.links
    shader=next((n for n in nodes if n.type in ('BSDF_PRINCIPLED','EMISSION','BSDF_DIFFUSE')),None);socket=shader.inputs['Base Color' if shader.type=='BSDF_PRINCIPLED' else 'Color'] if shader else next(n for n in nodes if n.type=='OUTPUT_MATERIAL').inputs['Surface'];oldlink=socket.links[0].from_socket if socket.links else None
    if not shader and (oldlink is None or oldlink.type!='RGBA'):raise ValueError('Unexpected baked material output '+str([(n.name,n.type)for n in nodes]))
    mix=nodes.new('ShaderNodeMixRGB');mix.blend_type='MIX';mix.inputs[1].default_value=socket.default_value if hasattr(socket,'default_value') else (.5,.5,.5,1)
    if oldlink:links.new(oldlink,mix.inputs[1])
    uvnode=nodes.new('ShaderNodeUVMap');uvnode.uv_map=uv.name
    for image,slot in [(source,2),(mask,0)]:
     tex=nodes.new('ShaderNodeTexImage');tex.image=image;tex.interpolation='Closest';tex.extension='CLIP';links.new(uvnode.outputs['UV'],tex.inputs['Vector']);links.new(tex.outputs['Color'],mix.inputs[slot])
    links.new(mix.outputs['Color'],socket);materials[oldindex]=len(obj.data.materials);obj.data.materials.append(mat)
   face.material_index=materials[oldindex]
  assert all((obj.matrix_world@obj.data.vertices[i].co-oldworld[i]).length<1e-6 for i in range(len(oldworld)) if not 1018<oldworld[i].x<1170), 'Outside applied cut footprint changed'
  assert all((obj.matrix_world@obj.data.vertices[i].co-oldworld[i]).length<1e-6 for i in range(len(oldworld)) if i not in changed)
  edits.append(dict(object=obj.name,changed_vertices=len(changed),unchanged_vertices=len(oldworld)-len(changed),old_geometry=before[obj.name],new_geometry=geometry(obj)))
 bpy.context.view_layer.update();bpy.ops.wm.save_as_mainfile(filepath=str(DEST/'model.blend'),compress=True)
 modelhash=sha(DEST/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(DEST/'model.blend'));bpy.context.view_layer.update();objects=[bpy.data.objects[r['object']]for r in survey['objects']]
 review=bpy.data.scenes.new('Initial fence bounded review');bpy.context.window.scene=review
 for obj in objects:
  review.collection.objects.link(obj);obj.hide_render=False;parent=obj.parent
  while parent:
   if parent.name not in review.objects:review.collection.objects.link(parent)
   parent=parent.parent
 bpy.context.view_layer.update();tree,owners,_=_tree(objects);coverage=np.zeros_like(domain)
 for y,x in zip(*np.nonzero(roles)):
  hit,_,_,_=tree.ray_cast(Vector((x+.5,-(y+.5)/SIN,0))+RAY*6000,-RAY);coverage[y,x]=hit is not None
 Image.fromarray((roles&~coverage).astype('uint8')*255).save(DEST/'remaining-fence-domain.png')
 target=Vector((1094,-870/SIN,0));camera(review,target,RAY,512,512,200);review.render.filepath=str(DEST/'native-close.png');bpy.ops.render.render(write_still=True,scene=review.name)
 points=[o.matrix_world@v.co for o in objects for v in o.data.vertices];center=sum(points,Vector())/len(points);images=[]
 for i in range(8):
  angle=i*math.pi/4;direction=RAY if i==0 else Vector((math.sin(angle)*COS,-math.cos(angle)*COS,SIN));camera(review,center,direction,640,384,760);review.render.filepath=str(DEST/f'view-{i}.png');bpy.ops.render.render(write_still=True,scene=review.name);images.append(Path(review.render.filepath))
 sheet=Image.new('RGB',(1280,384),'#303030')
 for i,path in enumerate(images):
  im=Image.open(path).convert('RGBA');bg=Image.new('RGBA',im.size,'#303030');bg.alpha_composite(im);sheet.paste(bg.convert('RGB').resize((320,192)),(i%4*320,i//4*192))
 sheet.save(DEST/'actual8.png')
 write_json(DEST/'validation.json',dict(status='Private geometry trial; visual review pending',base_sha256=sha(BASE),model_sha256=modelhash,scope='Initial parts019/020 only; no applied state, ground or neighbor edits',source_sha256=sha(sourcepath),source_pixels=5419,physical_covered=int((roles&coverage).sum()),remaining=int((roles&~coverage).sum()),edits=edits,source_camera_first=True,limitations=['Retained individual open weave; remaining gaps require classification before any floor fill.','Native reprojected front colors retained; existing inferred appearance on modified rear surfaces is provisional.','Post cross sections and hidden weave depth remain inferred.']))
 print('COVERAGE',int((roles&coverage).sum()),'REMAINING',int((roles&~coverage).sum()),flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
