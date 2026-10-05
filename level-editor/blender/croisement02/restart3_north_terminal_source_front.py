"""Attribute native front gaps and gray surfaces to exact traced source roles."""
import sys,json,collections
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from scipy.ndimage import distance_transform_edt
from mathutils.bvhtree import BVHTree
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from tree_geometry import RAY
from log_trap_state_candidate import point
from evidence_io import sha,write_json
from render_slots import acquire,release


def main():
    worker=Path(sys.argv[sys.argv.index('--')+1]);dest=worker/'source-front-v1';dest.mkdir(exist_ok=False)
    meta=json.loads((worker/'manifest.json').read_text());owners=json.loads((worker/'ownership.json').read_text());labels=np.array(Image.open(worker/'ownership-labels.png'));native=np.array(Image.open(meta['source_frame']['image']).convert('RGBA'));depths={i:distance_transform_edt(labels==i) for i in range(1,len(owners['roles'])+1)}
    bpy.ops.wm.open_mainfile(filepath=str(worker/'worker.blend'));vertices=[];faces=[];mapping=[];images={}
    for obj in bpy.context.scene.objects:
        if obj.type!='MESH':continue
        start=len(vertices);vertices.extend(obj.matrix_world@v.co for v in obj.data.vertices)
        for face in obj.data.polygons:faces.append(tuple(start+i for i in face.vertices));mapping.append((obj,face.index))
        for mat in obj.data.materials:
            for node in mat.node_tree.nodes:
                if node.type=='TEX_IMAGE':images[mat.name]=(Path(bpy.path.abspath(node.image.filepath)).stem,np.array(Image.open(bpy.path.abspath(node.image.filepath)).convert('RGBA')))
    tree=BVHTree.FromPolygons(vertices,faces);rows=[];counts=collections.Counter();roles=collections.defaultdict(collections.Counter);heat=np.zeros_like(native);heat[:,:,3]=255
    colors={'known-native':[30,170,50],'assigned-gray-or-foreign':[255,190,0],'assigned-no-geometry':[255,30,30],'unassigned':[70,130,255]}
    for y,x in np.argwhere(native[:,:,3]>0):
        role=int(labels[y,x]);expected=owners['roles'][role-1]['name'] if role else 'unassigned';hit=tree.ray_cast(point(1202+x+.5,220+y+.5,0)+RAY*2000,-RAY,4000)
        objname=None;material=None;actualrole=None;known=False
        if hit[0] is not None:
            obj,fi=mapping[hit[2]];face=obj.data.polygons[fi];objname=obj.name;material=obj.data.materials[face.material_index].name
            if material in images:
                actualrole,img=images[material];known=bool(img[y,x,3]>0)
        kind='unassigned' if not role else 'assigned-no-geometry' if hit[0] is None else 'known-native' if known else 'assigned-gray-or-foreign'
        interior=bool(role and depths[role][y,x]>=2);counts[kind]+=1;roles[expected][kind]+=1
        if interior:roles[expected][kind+'-interior-distance2']+=1
        heat[y,x,:3]=colors[kind];rows.append([int(x),int(y),expected,kind,interior,objname,actualrole,material])
    Image.fromarray(heat).resize((1008,568),Image.Resampling.NEAREST).save(dest/'classification.png')
    write_json(dest/'rays.json',dict(columns=['x','y','expected_role','kind','interior2','hit_object','hit_role','hit_material'],rows=rows))
    write_json(dest/'report.json',dict(model_sha256=sha(worker/'worker.blend'),counts=dict(counts),roles={k:dict(v) for k,v in roles.items()},legend=colors,method='Exact saved-model first hit at each original native pixel center; inspect actual assigned material and role-specific source alpha; interior is distance>=2 pixels from traced role boundary',limitations=['Single pixel-center attribution does not prove subpixel absence or exhaustively bound edge rasterization.','Role domains are manually inferred; unassigned pixels require independent source interpretation.']))
    print(dict(counts));print({k:dict(v) for k,v in roles.items()})

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
