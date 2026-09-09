"""Smooth the measured river bluff and add modest inferred clearing relief."""
import math
import sys
from pathlib import Path
import bpy
import bmesh
from mathutils import Vector
sys.path.insert(0,str(Path(__file__).parent))
from modeling import COS,SIN,collection,game_point,mesh,retire,source

NAME='15 Terrain - river bluff and clearing relief'
c=collection(NAME)
original=source(111)
bm=bmesh.new()
for p in original.data.polygons:
    if (original.matrix_world.to_3x3()@p.normal).z>.1:
        bm.faces.new([bm.verts.new(original.matrix_world@original.data.vertices[i].co) for i in p.vertices])
bmesh.ops.remove_doubles(bm,verts=list(bm.verts),dist=.01)
bmesh.ops.subdivide_edges(bm,edges=list(bm.edges),cuts=7,use_grid_fill=True)
boundary={v for e in bm.edges if e.is_boundary for v in e.verts}
for iteration in range(12):
    updates={v:sum(e.other_vert(v).co.z for e in v.link_edges)/len(v.link_edges) for v in bm.verts if v not in boundary}
    for v,z in updates.items():v.co.z=v.co.z*.4+z*.6
top=list(bm.faces);edges=[e for e in bm.edges if e.is_boundary]
bottom={v:bm.verts.new(Vector((v.co.x,v.co.y,-.45))) for v in list(bm.verts)}
for f in top:bm.faces.new([bottom[v] for v in reversed(list(f.verts))])
for e in edges:
    a,b=e.verts
    bm.faces.new((a,b,bottom[b],bottom[a]))
bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.verts.index_update()
obj=mesh(c,'River bluff - continuous rounded upper slope',[v.co.copy() for v in bm.verts],[[v.index for v in f.verts] for f in bm.faces])
for p in obj.data.polygons:p.use_smooth=p.normal.z>.1
bm.free();retire(111,NAME)
obj['inferred']='Interior slope interpolation; original bluff perimeter and edge heights retained'

# Small relief in the empty grassy clearing, away from the camp furniture.
# A shallow field avoids inventing a large hill or moving measured structures.
nx,ny=160,94
vertices=[]
for y in range(ny+1):
    gy=1088*y/ny
    for x in range(nx+1):
        gx=1920*x/nx
        z=sum(amp*math.exp(-((gx-cx)/sx)**2-((gy-cy)/sy)**2)
              for cx,cy,sx,sy,amp in [(1260,670,100,120,7),(1210,820,95,85,4),(1160,980,140,95,2)])
        vertices.append(game_point(gx,gy,z))
faces=[(y*(nx+1)+x,(y+1)*(nx+1)+x,(y+1)*(nx+1)+x+1,y*(nx+1)+x+1) for y in range(ny) for x in range(nx)]
old=next(o for o in bpy.data.collections['01 Refinement - working copy'].objects if o.get('source_obstacle')=='ground')
# Use the pipeline's obstacle-filled ground texture, not the unmasked Day map:
# otherwise the houses and trees would be painted a second time on the floor.
ground=mesh(c,'Terrain - lightly undulating clearing',vertices,faces,old.data.materials[0])
for p in ground.data.polygons:p.use_smooth=True
ground['inferred']='Shallow clearing relief; no additional elevation reference exists'
ground['open_surface']=True
old.hide_render=True
for scene in bpy.data.scenes:
    for layer in scene.view_layers:
        if old.name in layer.objects:old.hide_set(True,view_layer=layer)
old['replaced_by']=NAME
result={'terrain_vertices':len(vertices),'bluff_faces':len(obj.data.polygons),'collection':NAME}
