"""Export native patch appearances on clipped copies of their approved physical receivers."""
import sys,json,struct,math,hashlib
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,COS,RAY
from restart2_export_net_appearances import handoff_tick
BASE=OUT/'restart2-state/receiver-rebind-v2';DEST=OUT/'restart2-state/receiver-surfaces-v1'
def screen(p):return np.array([p[0],-p[1]*SIN-p[2]*COS])
def clip_polygon(poly,axis,bound,keep):
 out=[]
 for a,b in zip(poly,poly[1:]+poly[:1]):
  va=(screen(a)[axis]-bound)*keep;vb=(screen(b)[axis]-bound)*keep
  if va>=-1e-9:out.append(a)
  if (va<0)!=(vb<0):out.append(a+(b-a)*(va/(va-vb)))
 return out
class GLB:
 def __init__(self):
  self.bin=bytearray();self.doc={'asset':{'version':'2.0','generator':'Physical receiver appearance proof'},'scene':0,'scenes':[{'nodes':[]}],'nodes':[],'meshes':[],'materials':[],'textures':[],'images':[],'samplers':[{'magFilter':9728,'minFilter':9728,'wrapS':33071,'wrapT':33071}],'bufferViews':[],'accessors':[],'extensionsUsed':['KHR_materials_unlit']};self.channels=[];self.samplers=[]
 def view(self,data,target=None):
  self.bin.extend(b'\0'*(-len(self.bin)%4));r={'buffer':0,'byteOffset':len(self.bin),'byteLength':len(data)}
  if target:r['target']=target
  self.doc['bufferViews'].append(r);self.bin.extend(data);return len(self.doc['bufferViews'])-1
 def accessor(self,array,kind,component=5126,target=None):
  a=np.asarray(array,dtype='<f4'if component==5126 else '<u4');view=self.view(a.tobytes(),target);r={'bufferView':view,'componentType':component,'count':len(a),'type':kind}
  if kind=='VEC3':r.update(min=a.min(axis=0).tolist(),max=a.max(axis=0).tolist())
  if kind=='SCALAR':r.update(min=[float(a.min())],max=[float(a.max())])
  self.doc['accessors'].append(r);return len(self.doc['accessors'])-1
 def layer(self,name,vertices,uv,png,extra,initial):
  points=np.array(vertices);positions=np.column_stack([points[:,0],points[:,2],-points[:,1]]);position=self.accessor(positions,'VEC3',target=34962);texcoord=self.accessor(uv,'VEC2',target=34962);indices=self.accessor(np.arange(len(points)),'SCALAR',5125,34963);image=len(self.doc['images']);self.doc['images'].append({'bufferView':self.view(png),'mimeType':'image/png'});texture=len(self.doc['textures']);self.doc['textures'].append({'sampler':0,'source':image});material=len(self.doc['materials']);self.doc['materials'].append({'name':name,'alphaMode':'MASK','alphaCutoff':.5,'doubleSided':False,'extensions':{'KHR_materials_unlit':{}},'pbrMetallicRoughness':{'baseColorTexture':{'index':texture},'metallicFactor':0,'roughnessFactor':1}});mesh=len(self.doc['meshes']);self.doc['meshes'].append({'primitives':[{'attributes':{'POSITION':position,'TEXCOORD_0':texcoord},'indices':indices,'material':material}]});node=len(self.doc['nodes']);self.doc['nodes'].append({'name':name,'mesh':mesh,'scale':[1,1,1]if initial else [0,0,0],'extras':extra});self.doc['scenes'][0]['nodes'].append(node);return node
 def animation(self,node,ticks,values):
  t=self.accessor(np.array(ticks)/25,'SCALAR');v=self.accessor(values,'VEC3');index=len(self.samplers);self.samplers.append({'input':t,'output':v,'interpolation':'STEP'});self.channels.append({'sampler':index,'target':{'node':node,'path':'scale'}})
 def save(self,path):
  self.doc['animations']=[{'name':'Native patch transition','samplers':self.samplers,'channels':self.channels}];self.doc['buffers']=[{'byteLength':len(self.bin)}];j=json.dumps(self.doc,separators=(',',':')).encode();j+=b' '*(-len(j)%4);self.bin.extend(b'\0'*(-len(self.bin)%4));path.write_bytes(struct.pack('<III',0x46546c67,2,28+len(j)+len(self.bin))+struct.pack('<II',len(j),0x4e4f534a)+j+struct.pack('<II',len(self.bin),0x004e4942)+self.bin)
def main():
 if DEST.exists():raise FileExistsError(DEST)
 ledger=json.loads((BASE/'report.json').read_text());acquire()
 try:
  DEST.mkdir();meshes={}
  for model in ledger['models']:
   path=Path(model['path']);assert sha(path)==model['sha256'];bpy.ops.wm.open_mainfile(filepath=str(path))
   if model['receiver']=='bank':bpy.context.window.scene=bpy.data.scenes[json.loads((path.parent/'workspace.json').read_text())['scene_name']]
   bpy.context.view_layer.update()
   for index,row in enumerate(ledger['objects']):
    if row['receiver']!=model['receiver']:continue
    obj=bpy.data.objects[row['name']];assert np.max(abs(np.array(obj.matrix_world)-np.array(row['matrix_world'])))<1e-6;obj.data.calc_loop_triangles();points=np.array([obj.matrix_world@v.co for v in obj.data.vertices]);meshes[index]=(points,np.array([t.vertices[:]for t in obj.data.loop_triangles]))
  reports=[]
  for assembly in ['log-trap','rock-trap']:
   rows={}
   for row in ledger['frames']:
    if row['assembly']!=assembly:continue
    key=(row['state'],row['frame'])
    if key in rows:assert rows[key]['source_sha256']==row['source_sha256']and rows[key]['bbox']==row['bbox']
    else:rows[key]=row
   transitions=[rows[k]for k in sorted(rows)if k[0]=='transition'];terminal=handoff_tick(transitions);starts=np.cumsum([0]+[r['delay']+1 for r in transitions[:-1]]).tolist();glb=GLB();bindings=[]
   for (state,phase),row in sorted(rows.items()):
    raw=np.array(Image.open(row['source']).convert('RGBA'));assert sha(Path(row['source']))==row['source_sha256'];x,y,w,h=row['bbox'];initial=state=='initial'
    for receiver in row['receivers']:
     owner=receiver['object'];domain=np.array(Image.open(receiver['domain']))>0;rgba=raw.copy();rgba[:,:,3]=np.where(domain,raw[:,:,3],0);image=DEST/f'{assembly}-{state}-{phase:03}-receiver-{owner}.png';Image.fromarray(rgba).save(image);verts=[];uv=[];points,triangles=meshes[owner]
     for ids in triangles:
      poly=[points[i]for i in ids];projected=np.array([screen(p)for p in poly])
      if projected[:,0].max()<x or projected[:,0].min()>x+w or projected[:,1].max()<y or projected[:,1].min()>y+h:continue
      for axis,bound,keep in [(0,x,1),(0,x+w,-1),(1,y,1),(1,y+h,-1)]:
       poly=clip_polygon(poly,axis,bound,keep)
       if not poly:break
      for i in range(1,len(poly)-1):
       tri=[poly[0],poly[i],poly[i+1]]
       if np.linalg.norm(np.cross(tri[1]-tri[0],tri[2]-tri[0]))<1e-8:continue
       for p in tri:
        sx,sy=screen(p);verts.append(p+np.array(RAY)*.002);uv.append([(sx-x)/w,(sy-y)/h])
     if not verts:raise ValueError('No receiver geometry for opaque domain')
     node=glb.layer(f'{assembly} {state} {phase:03} receiver{owner}',verts,uv,image.read_bytes(),{'native_state':state,'native_frame':phase,'receiver':owner,'source_sha256':row['source_sha256']},initial)
     if initial:glb.animation(node,[0,terminal],[[0,0,0],[0,0,0]])
     else:
      start=starts[phase];end=starts[phase+1]if phase+1<len(starts)else terminal+1;ticks=sorted({0,start,min(end,terminal),terminal});glb.animation(node,ticks,[[1,1,1]if start<=t<end else [0,0,0]for t in ticks])
     bindings.append({'state':state,'phase':phase,'receiver':owner,'triangles':len(verts)//3,'source_pixels':int(domain.sum()),'image':image.name,'image_sha256':sha(image),'node':node})
   file=DEST/(assembly+'.glb');glb.save(file);reports.append({'assembly':assembly,'file':file.name,'sha256':sha(file),'initial_frames':1,'transition_frames':len(transitions),'phase_start_ticks':starts,'terminal_tick':terminal,'layers':bindings})
  write_json(DEST/'manifest.json',{'status':'Private surface-conforming appearance export; browser/native-pixel proof pending','receiver_ledger_sha256':sha(BASE/'report.json'),'models':ledger['models'],'records':reports,'source_preserving_depth_offset':.002,'limits':['Only approved receiver surfaces are duplicated/clipped; no flat substitute geometry.','No target body, metadata or script clock is inferred.','Base atlas restore and canonical instance activation remain integration work.']})
 finally:release()
if __name__=='__main__':main()
