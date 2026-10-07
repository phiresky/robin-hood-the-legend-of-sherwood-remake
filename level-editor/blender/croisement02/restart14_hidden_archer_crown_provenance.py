"""Read-only material and ownership attribution for overlapping static crowns."""
import json,sys,io,math
from PIL import Image
import numpy as np
from pathlib import Path
import bpy
from mathutils import Vector
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from tree_geometry import RAY,SIN
from refinement_review import _tree
from render_slots import acquire,release

def main():
    root=OUT/'restart14-hidden-archer/audit-v1'
    authority=json.loads((root/'first-hit-v1/report.json').read_text())
    assert sha(authority['source'])==authority['source_sha256']
    bpy.ops.wm.open_mainfile(filepath=authority['source'])
    scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.window.scene=scene
    objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and ' / Crown' in o.name and not o.hide_render]
    needed={s['object'] for row in authority['profiles'] for s in row['samples'] if ' / Crown' in (s['object'] or '')}
    objects=[o for o in objects if o.name in needed]
    tree,owners,_=_tree(objects);offsets={};offset=0
    for obj in objects:
        assert not obj.modifiers
        obj.data.calc_loop_triangles();offsets[obj]=offset;offset+=len(obj.data.loop_triangles)
    records=[];images={}
    for row in authority['profiles']:
        samples=[]
        for s in row['samples']:
            if s['object'] not in needed:continue
            x,y=s['pixel'];point,_,index,_=tree.ray_cast(Vector((x+.5,-(y+.5)/SIN,0))+RAY*6000,-RAY)
            assert point is not None
            obj=owners[index];mesh=obj.data;triangle=mesh.loop_triangles[index-offsets[obj]];mat=mesh.materials[triangle.material_index]
            attrs={}
            for attr in mesh.color_attributes:
                ids=triangle.loops if attr.domain=='CORNER' else triangle.vertices
                attrs[attr.name]=[list(attr.data[i].color) for i in ids]
            shader=next(n for n in mat.node_tree.nodes if n.type=='BSDF_PRINCIPLED')
            texture=shader.inputs['Base Color'].links[0].from_node
            assert texture.type=='TEX_IMAGE',(mat.name,texture.type)
            im=texture.image
            if im.name not in images:
                assert im.packed_file
                images[im.name]=np.array(Image.open(io.BytesIO(im.packed_file.data)).convert('RGBA'))
            rgba=images[im.name];h,w=rgba.shape[:2]
            uvname=texture.inputs['Vector'].links[0].from_node.uv_map
            a,b,c=[obj.matrix_world@mesh.vertices[v].co for v in triangle.vertices]
            ab,ac,ap=b-a,c-a,point-a;aa,bb,cc=ab.dot(ab),ab.dot(ac),ac.dot(ac);den=aa*cc-bb*bb
            u=(cc*ap.dot(ab)-bb*ap.dot(ac))/den;v=(aa*ap.dot(ac)-bb*ap.dot(ab))/den
            coords=[mesh.uv_layers[uvname].data[i].uv for i in triangle.loops]
            uv=coords[0]*(1-u-v)+coords[1]*u+coords[2]*v
            tx,ty=math.floor(uv.x*w),h-1-math.floor(uv.y*h)
            assert 0<=tx<w and 0<=ty<h
            texture_sample=dict(image=im.filepath,image_size=[w,h],image_packed_sha256=__import__('hashlib').sha256(im.packed_file.data).hexdigest(),uv=list(uv),texel=[tx,ty],rgba=rgba[ty,tx].tolist(),interpolation=texture.interpolation)
            samples.append(dict(pixel=s['pixel'],texture_sample=texture_sample,object=obj.name,triangle=index-offsets[obj],material=mat.name,material_properties={k:mat[k] for k in mat.keys() if isinstance(mat[k],(str,bool,float,int))},color_attributes=attrs))
        records.append(dict(profile=row['profile'],samples=samples))
    write_json(root/'crown-provenance-v2.json',dict(status='Read-only provenance; no geometry or alpha edits',source=authority['source'],source_sha256=authority['source_sha256'],profiles=records))
if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
