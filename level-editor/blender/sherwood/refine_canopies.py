"""Rounded foliage clusters retaining all authored animated alpha frames.

The image gives the silhouette and leaf colour. Depth is an inferred clustered
volume of separate leaf masses, anchored to the supporting trunks.
TODO: replace concealed canopy colour with painted non-projective UVs if needed.
"""
import json
import math
import random
import bmesh
import sys
from pathlib import Path
import bpy
import numpy as np
from mathutils import Vector

sys.path.insert(0,str(Path(__file__).parent))
from modeling import EYE, ROOT, DATA, SIN, COS, collection, game_point

NAME='10 Animated foliage - curved canopy shells'
c=collection(NAME)
records=json.loads((ROOT/'level-editor/work/sherwood-refinement/animation-references/manifest.json').read_text())['assets']
references=bpy.data.collections['07 Reference only - authored animated sprites']
level=json.loads((DATA/'Levels/Sherwood.rhp.json').read_text())
# Sprite elevation establishes draw placement, not the crown's ground depth.
# Anchor the reconstructed crown to the trees visible beneath its silhouette.
supports={'Sherwood - Arbre01':[27,30,32],
          'Sherwood - Arbre02':[46,35,29,31,24,25,27],
          'Sherwood - Arbre03':[44,42,45,36,37],
          'Sherwood - Arbre04':[34],
          'Sherwood - Arbre06':[40]}
def ground_depth(profile,x):
    if profile=='Sherwood - Arbre05':
        # The foreground trunk exits below the picture; its root is off-map.
        return 1120.0
    controls=[]
    for index in supports[profile]:
        points=level['sight_obstacles'][index]['points']
        controls.append(((min(p['x'] for p in points)+max(p['x'] for p in points))/2,
                         (min(p['y'] for p in points)+max(p['y'] for p in points))/2))
    controls.sort()
    if x<=controls[0][0]:return controls[0][1]
    for (ax,ay),(bx,by) in zip(controls,controls[1:]):
        if x<=bx:
            t=(x-ax)/(bx-ax)
            t=t*t*(3-2*t)
            return ay+(by-ay)*t
    return controls[-1][1]
counts={}
UP=Vector((0,SIN,COS))
for record in records:
    if record['kind']!='tree':
        continue
    ref=next(o for o in references.objects if o.get('profile')==record['profile'])
    material=ref.data.materials[0].copy()
    material.name=record['profile']+' - clustered animated foliage'
    nodes=material.node_tree.nodes;links=material.node_tree.links
    texture=next(n for n in nodes if n.type=='TEX_IMAGE')
    # An atlas clips at its outer border, not each frame. Clip canvas UVs
    # explicitly so rounded crowns cannot sample the neighboring frame.
    uv_node=next(n for n in nodes if n.type=='TEX_COORD')
    split=nodes.new('ShaderNodeSeparateXYZ');links.new(uv_node.outputs['UV'],split.inputs[0])
    mask=None
    for axis in ['X','Y']:
        for operation,limit in [('GREATER_THAN',0),('LESS_THAN',1)]:
            node=nodes.new('ShaderNodeMath');node.operation=operation;node.inputs[1].default_value=limit
            links.new(split.outputs[axis],node.inputs[0])
            if mask is None:mask=node.outputs[0]
            else:
                product=nodes.new('ShaderNodeMath');product.operation='MULTIPLY'
                links.new(mask,product.inputs[0]);links.new(node.outputs[0],product.inputs[1]);mask=product.outputs[0]
    product=nodes.new('ShaderNodeMath');product.operation='MULTIPLY'
    links.new(mask,product.inputs[0]);links.new(texture.outputs['Alpha'],product.inputs[1])
    mix=next(n for n in nodes if n.type=='MIX_SHADER')
    links.new(product.outputs[0],mix.inputs[0])
    aw,ah=texture.image.size
    pixels=np.empty(aw*ah*4,dtype=np.float32);texture.image.pixels.foreach_get(pixels)
    alpha=pixels.reshape(ah,aw,4)[:,:,3]
    width,height=record['canvas'];gutter=record['gutter']
    union=np.zeros((height,width),dtype=bool)
    for row in range(4):
        for col in range(4):
            union |= alpha[row*(height+2*gutter)+gutter:row*(height+2*gutter)+gutter+height,
                           col*(width+2*gutter)+gutter:col*(width+2*gutter)+gutter+width]>.01
    union=union[::-1,:]
    p0,p1,p2,p3=[ref.matrix_world@v.co for v in ref.data.vertices]
    # Independent rounded leaf masses have real backs and no extruded mask
    # walls. Only cluster centers follow trunk depth, never individual vertices.
    rng=random.Random(record['profile'])
    step=38
    nx,ny=math.ceil(width/step),math.ceil(height/step)
    template=bmesh.new();bmesh.ops.create_icosphere(template,subdivisions=4,radius=1)
    template.verts.ensure_lookup_table();template.verts.index_update()
    unit=[v.co.copy() for v in template.verts]
    unit_faces=[[v.index for v in f.verts] for f in template.faces]
    template.free()
    vertices=[];faces=[];clusters=0
    for iy in range(ny):
        for ix in range(nx):
            x0,x1=int(ix*width/nx),math.ceil((ix+1)*width/nx)
            y0,y1=int(iy*height/ny),math.ceil((iy+1)*height/ny)
            if not union[y0:y1,x0:x1].any():continue
            px=(ix+.5)*width/nx+rng.uniform(-step*.36,step*.36)
            py=(iy+.5)*height/ny+rng.uniform(-step*.36,step*.36)
            base=p0+(p1-p0)*(px/width)+(p3-p0)*(py/height)
            screen_y=-base.y*SIN-base.z*COS
            gy=ground_depth(record['profile'],base.x)
            crown_y=-p0.y*SIN-p0.z*COS+height*.5
            # Start on a camera-normal plane through the trunk-anchored crown
            # center, then populate its depth. A constant ground-y per vertex
            # would recreate the previous vertical wall of foliage.
            center=game_point(base.x,gy,gy-crown_y)+UP*(crown_y-screen_y)
            rx=width/nx*rng.uniform(.80,1.15)
            ry=height/ny*rng.uniform(.76,1.12)
            rz=(rx+ry)*rng.uniform(.45,.78)
            vertical=(py/height-.5)/.58
            dome=math.sqrt(max(.08,1-vertical*vertical))
            volume_depth=min(width,height)*.40*dome
            center+=EYE*rng.uniform(-volume_depth,volume_depth)
            phase=rng.uniform(0,math.tau)
            start=len(vertices)
            for v in unit:
                # Low-amplitude lobes avoid perfect repeated balls.
                relief=(1+.13*math.sin(v.x*7+phase)*math.sin(v.y*6-phase)*math.cos(v.z*5)
                        +.055*math.sin(v.z*9+phase)*math.cos(v.x*8-phase))
                vertices.append(center+Vector((v.x*rx*relief,0,0))+UP*(v.y*ry*relief)+EYE*(v.z*rz*relief))
            faces.extend([tuple(start+i for i in f) for f in unit_faces])
            clusters+=1
    data=bpy.data.meshes.new(record['profile']+' rounded leaf clusters')
    data.from_pydata(vertices,[],faces);data.update()
    obj=bpy.data.objects.new(data.name,data);c.objects.link(obj)
    data.materials.append(material)
    uv=data.uv_layers.new(name='Authored animated canvas')
    left=p0.x;top=-p0.y*SIN-p0.z*COS
    for loop in data.loops:
        v=data.vertices[loop.vertex_index].co
        uv.data[loop.index].uv=((v.x-left)/width,1-(-v.y*SIN-v.z*COS-top)/height)
    for polygon in data.polygons:polygon.use_smooth=True
    obj['profile']=record['profile'];obj['frame_count']=16
    obj['leaf_clusters']=clusters
    obj['inferred']='Independent rounded foliage masses; source animated color/alpha projected onto clusters'
    obj['source_elevation']=record['elevation']
    obj['depth_anchor']='Supporting trunk footprints; foreground Arbre05 root inferred beyond image'
    ref.hide_render=True
    for scene in bpy.data.scenes:
        for layer in scene.view_layers:
            if ref.name in layer.objects:ref.hide_set(True,view_layer=layer)
    ref['replaced_by']=NAME
    counts[record['profile']]={'clusters':clusters,'vertices':len(vertices),'faces':len(faces)}

# The remaining authored water/fire/butterfly overlays remain separate assets.
ambient=collection('11 Authored ambient overlay references')
for ref in references.objects:
    if ref.get('bank')!='shertree':ambient.objects.link(ref)
result={'canopies':counts,'ambient_overlays':len(ambient.objects)}
