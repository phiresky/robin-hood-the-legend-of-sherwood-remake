"""Exact convex surface partition at nearest-filtered Boolean texture boundaries."""
import numpy as np

def area(poly):
 a=np.asarray(poly)[:,1:];return abs(np.dot(a[:,0],np.roll(a[:,1],-1))-np.dot(a[:,1],np.roll(a[:,0],-1)))

def clip(poly,values,bound,positive):
 output=[]
 for i,p in enumerate(poly):
  q=poly[(i+1)%len(poly)];a=values[i]-bound;b=values[(i+1)%len(poly)]-bound;inside=a>=-1e-13 if positive else a<=1e-13;next_inside=b>=-1e-13 if positive else b<=1e-13
  if inside:output.append(p)
  if inside!=next_inside:output.append(p+(q-p)*(a/(a-b)))
 if len(output)<3:return None
 result=np.asarray(output)
 return result if area(result)>1e-13 else None

def rectangle(poly,uv,x0,y0,x1,y1):
 for axis,bound,positive in [(0,x0,True),(0,x1,False),(1,y0,True),(1,y1,False)]:
  poly=clip(poly,(poly@uv)[:,axis],bound,positive)
  if poly is None:return None
 return poly

class Mask:
 def __init__(self,values,uv_name,extension):
  self.values=np.asarray(values,bool);self.uv_name=uv_name;self.extension=extension;self.height,self.width=self.values.shape
  assert extension in ['CLIP','EXTEND','REPEAT']
 def lookup(self,x,y):
  x,y=np.broadcast_arrays(x,y)
  if self.extension=='REPEAT':return self.values[y%self.height,x%self.width]
  values=self.values[np.clip(y,0,self.height-1),np.clip(x,0,self.width-1)]
  if self.extension=='CLIP':values=values&((x>=0)&(x<self.width)&(y>=0)&(y<self.height))
  return values
 def split(self,poly,uv):
  scaled=uv*np.array([self.width,self.height]);coords=poly@scaled;low=np.floor(coords.min(0)+1e-10).astype(int);high=np.ceil(coords.max(0)-1e-10).astype(int);high=np.maximum(high,low+1)
  if np.prod(high-low)>1000000:raise ValueError('Mask footprint exceeds bounded prototype')
  xs=np.arange(low[0],high[0]);ys=np.arange(low[1],high[1]);values=self.lookup(xs[None,:],ys[:,None]);unique=np.unique(values)
  if len(unique)==1:return [(bool(unique[0]),poly)]
  # Merge identical horizontal runs through adjacent scanlines.
  active={};rects=[]
  for offset,row in enumerate(values):
   y=int(low[1]+offset);edges=np.r_[0,np.flatnonzero(row[1:]!=row[:-1])+1,len(row)];runs={(int(low[0]+a),int(low[0]+b),bool(row[a]))for a,b in zip(edges[:-1],edges[1:])};next_active={}
   for key in runs:next_active[key]=(active[key][0],y+1)if key in active else(y,y+1)
   for key,(y0,y1)in active.items():
    if key not in runs:rects.append((key,y0,y1))
   active=next_active
  rects.extend((key,y0,y1)for key,(y0,y1)in active.items());result=[]
  for(x0,x1,value),y0,y1 in rects:
   p=rectangle(poly,scaled,x0,y0,x1,y1)
   if p is not None:result.append((value,p))
  assert abs(sum(area(p)for _,p in result)-area(poly))<1e-8
  return result

def condition(expr,poly,uvs,masks):
 kind=expr[0]
 if kind=='constant':return[(bool(expr[1]),poly)]
 if kind=='mask':return masks[expr[1]].split(poly,uvs[masks[expr[1]].uv_name])
 if kind=='not':return[(not flag,p)for flag,p in condition(expr[1],poly,uvs,masks)]
 assert kind=='or';result=[]
 for flag,p in condition(expr[1],poly,uvs,masks):
  if flag:result.append((True,p))
  else:result.extend(condition(expr[2],p,uvs,masks))
 return result

def colors(expr,poly,uvs,masks):
 if expr[0]=='leaf':return[(expr[1],poly)]
 assert expr[0]=='mix';result=[]
 for flag,p in condition(expr[1],poly,uvs,masks):result.extend(colors(expr[3]if flag else expr[2],p,uvs,masks))
 return result

if __name__=='__main__':
 triangle=np.eye(3);uv=np.array([[0.,0.],[1.,0.],[0.,1.]]);mask=Mask([[1,0],[0,1]],'a','CLIP');parts=mask.split(triangle,uv);assert abs(sum(area(p)for _,p in parts)-1)<1e-10
 masks={'a':mask,'b':Mask([[0,1],[1,0]],'b','REPEAT')};uvs={'a':uv,'b':uv[:,::-1]*1.3-.2};expr=('mix',('or',('mask','a'),('not',('mask','b'))),('leaf',0),('leaf',1));parts=colors(expr,triangle,uvs,masks);assert abs(sum(area(p)for _,p in parts)-1)<1e-9
 for leaf,p in parts:
  point=p.mean(0);a=point@uvs['a'];b=point@uvs['b'];expected=bool(mask.lookup(int(np.floor(a[0]*2)),int(np.floor(a[1]*2))))or not bool(masks['b'].lookup(int(np.floor(b[0]*2)),int(np.floor(b[1]*2))));assert leaf==int(expected)
 print('PASS checkerboard boundaries, distinct UV union/complement, partition area and branch identity',len(parts))

class Compiler:
 """Accept only the observed binary/color expression vocabulary, failing closed."""
 def __init__(self):self.masks={};self.maskids={};self.leaves={};self.leafids={};self.imagecache={}
 def pixels(self,im):
  if im.name not in self.imagecache:
   a=np.empty(len(im.pixels),np.float32);im.pixels.foreach_get(a);self.imagecache[im.name]=a.reshape(im.size[1],im.size[0],4)
  return self.imagecache[im.name]
 def uv(self,node):
  links=node.inputs['Vector'].links;assert len(links)==1 and links[0].from_node.type=='UVMAP';return links[0].from_node.uv_map
 def factor(self,socket):
  if not socket.is_linked:
   assert socket.default_value in (0,1);return('constant',bool(socket.default_value))
  link=socket.links[0];node=link.from_node
  if node.type=='MATH':
   if node.operation=='MAXIMUM':return('or',self.factor(node.inputs[0]),self.factor(node.inputs[1]))
   assert node.operation=='SUBTRACT'and not node.inputs[0].is_linked and node.inputs[0].default_value==1;return('not',self.factor(node.inputs[1]))
  assert node.type=='TEX_IMAGE'and node.interpolation=='Closest';assert link.from_socket.name in ('Alpha','Color');channel=3 if link.from_socket.name=='Alpha'else 0;data=self.pixels(node.image)[:,:,channel];assert set(np.unique(data))<={0.,1.}
  if channel==0:assert np.array_equal(self.pixels(node.image)[:,:,0],self.pixels(node.image)[:,:,1])and np.array_equal(data,self.pixels(node.image)[:,:,2])
  key=(node.image.name,self.uv(node),channel,node.extension)
  if key not in self.maskids:self.maskids[key]=len(self.masks);self.masks[self.maskids[key]]=Mask(data>0,self.uv(node),node.extension)
  return('mask',self.maskids[key])
 def color(self,socket):
  if not socket.is_linked:key=('constant',tuple(socket.default_value))
  else:
   link=socket.links[0];node=link.from_node
   if node.type=='MIX_RGB':
    assert node.blend_type=='MIX';return('mix',self.factor(node.inputs[0]),self.color(node.inputs[1]),self.color(node.inputs[2]))
   if node.type=='RGB':key=('constant',tuple(node.outputs[0].default_value))
   else:assert node.type=='TEX_IMAGE'and link.from_socket.name=='Color';key=('image',node.image.name,self.uv(node),node.interpolation,node.extension)
  if key not in self.leafids:self.leafids[key]=len(self.leaves);self.leaves[self.leafids[key]]=key
  return('leaf',self.leafids[key])
 def evaluate_factor(self,expr,point,uvs):
  if expr[0]=='constant':return expr[1]
  if expr[0]=='not':return not self.evaluate_factor(expr[1],point,uvs)
  if expr[0]=='or':return self.evaluate_factor(expr[1],point,uvs)or self.evaluate_factor(expr[2],point,uvs)
  mask=self.masks[expr[1]];xy=point@uvs[mask.uv_name];return bool(mask.lookup(int(np.floor(xy[0]*mask.width)),int(np.floor(xy[1]*mask.height))))
 def evaluate_color(self,expr,point,uvs):return expr[1]if expr[0]=='leaf'else self.evaluate_color(expr[3]if self.evaluate_factor(expr[1],point,uvs)else expr[2],point,uvs)
 def material(self,leaf,name):
  import bpy,hashlib
  value=self.leaves[leaf];mat=bpy.data.materials.new(name+f' / exact region {leaf}');mat.use_nodes=True;nodes=mat.node_tree.nodes;links=mat.node_tree.links;shader=nodes.get('Principled BSDF');shader.inputs['Base Color'].default_value=(0,0,0,1);shader.inputs['Metallic'].default_value=0;shader.inputs['Roughness'].default_value=1;shader.inputs['Emission Strength'].default_value=1
  if value[0]=='constant':shader.inputs['Emission Color'].default_value=value[1]
  else:
   _,image,uvname,interpolation,extension=value;tex=nodes.new('ShaderNodeTexImage');tex.image=bpy.data.images[image];tex.interpolation=interpolation;tex.extension=extension;uv=nodes.new('ShaderNodeUVMap');uv.uv_map=uvname;links.new(uv.outputs['UV'],tex.inputs['Vector']);links.new(tex.outputs['Color'],shader.inputs['Emission Color']);mat['original_rgb_packed_sha256']=hashlib.sha256(bytes(tex.image.packed_file.data)).hexdigest()
  mat['exact_binary_partition_leaf']=leaf;return mat

def convert(obj,provenance_path,*,maximum_triangles=800000):
 """Subdivide a private derivative only; each point retains its shader branch."""
 import bpy,hashlib
 old=obj.data;old.calc_loop_triangles();triangles=list(old.loop_triangles);compiler=Compiler();used={t.material_index for t in triangles};expressions={i:compiler.color(next(n for n in old.materials[i].node_tree.nodes if n.type=='OUTPUT_MATERIAL').inputs['Surface'])for i in used};uvs={u.name:np.array([x.uv[:]for x in u.data],float)for u in old.uv_layers};vertices=np.array([v.co[:]for v in old.vertices],float);corners=np.array([x.vector[:]for x in old.corner_normals],float);parts=[];max_area_error=0.;output_triangles=0
 for i,t in enumerate(triangles):
  if i%25000==0:print('Partition',obj.name,i,len(triangles),flush=True)
  triuv={name:a[list(t.loops)]for name,a in uvs.items()};expr=expressions[t.material_index];regions=colors(expr,np.eye(3),triuv,compiler.masks);error=abs(sum(area(p)for _,p in regions)-1);assert error<1e-8;max_area_error=max(max_area_error,error)
  for leaf,poly in regions:
   assert compiler.evaluate_color(expr,poly.mean(0),triuv)==leaf
   for j in range(1,len(poly)-1):parts.append((i,leaf,np.array([poly[0],poly[j],poly[j+1]])))
  output_triangles=len(parts)
  if output_triangles>maximum_triangles:raise ValueError('Exact partition growth exceeds bounded export limit')
 source_ids=np.array([p[0]for p in parts],np.int32);leaf_ids=np.array([p[1]for p in parts],np.int32);weights=np.array([p[2]for p in parts]);source_vertices=np.array([t.vertices[:]for t in triangles],np.int32);source_loops=np.array([t.loops[:]for t in triangles],np.int32);positions=np.einsum('tij,tjk->tik',weights,vertices[source_vertices[source_ids]]).reshape(-1,3);new=bpy.data.meshes.new(old.name+' / exact Boolean regions');new.from_pydata(positions,[],np.arange(len(positions)).reshape(-1,3));new.update()
 for name,a in uvs.items():
  uv=new.uv_layers.new(name=name);values=np.einsum('tij,tjk->tik',weights,a[source_loops[source_ids]]);uv.data.foreach_set('uv',values.astype(np.float32).ravel())
 mats={leaf:compiler.material(leaf,obj.name)for leaf in sorted(set(leaf_ids.tolist()))};slots={leaf:i for i,leaf in enumerate(mats)}
 for mat in mats.values():new.materials.append(mat)
 for i,p in enumerate(new.polygons):p.material_index=slots[int(leaf_ids[i])];p.use_smooth=old.polygons[triangles[int(source_ids[i])].polygon_index].use_smooth
 normals=np.einsum('tij,tjk->tik',weights,corners[source_loops[source_ids]]).reshape(-1,3);new.normals_split_custom_set(normals);obj.data=new
 for slot in obj.material_slots:slot.link='DATA'
 new.update();obj.update_tag(refresh={'DATA'});bpy.context.view_layer.update();np.savez_compressed(provenance_path,source_triangles=source_ids,leaf_ids=leaf_ids,barycentric=weights,source_vertex_indices=source_vertices,source_loop_indices=source_loops)
 return dict(object=obj.name,source_triangles=len(triangles),partition_triangles=len(parts),maximum_area_fraction_error=max_area_error,all_region_centroid_branches_exact=True,source_uv_names=list(uvs),all_rgb_images_original=True,leaves=compiler.leaves,provenance=str(provenance_path),provenance_sha256=hashlib.sha256(provenance_path.read_bytes()).hexdigest())
