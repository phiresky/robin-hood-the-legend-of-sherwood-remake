"""Curved, closed canopy shells retaining all authored animated alpha frames.

The image gives the silhouette and leaf colour. Depth is an inferred clustered
shell, displaced along camera rays so the original projection stays registered.
TODO: replace concealed canopy colour with painted non-projective UVs if needed.
"""
import json
import math
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
for record in records:
    if record['kind']!='tree':
        continue
    ref=next(o for o in references.objects if o.get('profile')==record['profile'])
    material=ref.data.materials[0]
    texture=next(n for n in material.node_tree.nodes if n.type=='TEX_IMAGE')
    aw,ah=texture.image.size
    pixels=np.empty(aw*ah*4,dtype=np.float32)
    texture.image.pixels.foreach_get(pixels)
    alpha=pixels.reshape(ah,aw,4)[:,:,3]
    width,height=record['canvas']; gutter=record['gutter']
    union=np.zeros((height,width),dtype=bool)
    for row in range(4):
        for col in range(4):
            union |= alpha[row*(height+2*gutter)+gutter:row*(height+2*gutter)+gutter+height,
                           col*(width+2*gutter)+gutter:col*(width+2*gutter)+gutter+width]>.01
    union=union[::-1,:]
    nx,ny=math.ceil(width/7),math.ceil(height/7)
    cells=set()
    for y in range(ny):
        for x in range(nx):
            if union[int(y*height/ny):math.ceil((y+1)*height/ny),int(x*width/nx):math.ceil((x+1)*width/nx)].any():
                cells.add((x,y))
    # Diagonal-only cell contacts make four faces share a vertical shell edge.
    # Fill one transparent grid cell at each contact; the authored alpha still
    # controls its visibility, while the shell stays manifold.
    changed=True
    while changed:
        changed=False
        for y in range(ny-1):
            for x in range(nx-1):
                square=[(x,y),(x+1,y),(x+1,y+1),(x,y+1)]
                present=[p in cells for p in square]
                if present in ([True,False,True,False],[False,True,False,True]):
                    cells.add(square[present.index(False)])
                    changed=True
    corner_keys=set()
    for x,y in cells:
        corner_keys.update([(x,y),(x+1,y),(x+1,y+1),(x,y+1)])
    keys=sorted(corner_keys)
    lookup={p:i for i,p in enumerate(keys)}
    # Source quad order is top-left, top-right, bottom-right, bottom-left.
    p0,p1,p2,p3=[ref.matrix_world@v.co for v in ref.data.vertices]
    depth=min(width,height)*.48
    verts=[]
    for side in [1,-1]:
        for x,y in keys:
            u,v=x/nx,y/ny
            base=p0+(p1-p0)*u+(p3-p0)*v
            pixel_y=-base.y*SIN-base.z*COS
            gy=ground_depth(record['profile'],base.x)
            base=game_point(base.x,gy,gy-pixel_y)
            dome=max(0,1-((u-.5)/.65)**2-((v-.5)/.65)**2)**.5
            clusters=max(math.exp(-((u-cu)**2+(v-cv)**2)/(2*radius**2))
                         for cu,cv,radius in [(.18,.22,.24),(.48,.20,.26),(.78,.27,.24),
                                             (.27,.52,.25),(.62,.53,.26),(.80,.70,.20),
                                             (.27,.78,.20),(.52,.82,.22)])
            displacement=side*(3+depth*dome*clusters)
            verts.append(base+EYE*displacement)
    n=len(keys);faces=[]
    for x,y in sorted(cells):
        a,b,d,e=[lookup[p] for p in [(x,y),(x+1,y),(x+1,y+1),(x,y+1)]]
        faces.extend([(a,e,d,b),(a+n,b+n,d+n,e+n)])
        for pair,neighbor in [((a,b),(x,y-1)),((b,d),(x+1,y)),((d,e),(x,y+1)),((e,a),(x-1,y))]:
            if neighbor not in cells:
                i,j=pair;faces.append((i,j,j+n,i+n))
    data=bpy.data.meshes.new(record['profile']+' canopy shell')
    data.from_pydata(verts,[],faces);data.update()
    obj=bpy.data.objects.new(data.name,data);c.objects.link(obj)
    data.materials.append(material)
    uv=data.uv_layers.new(name='Authored animated canvas')
    for loop in data.loops:
        x,y=keys[loop.vertex_index%n]
        uv.data[loop.index].uv=(x/nx,1-y/ny)
    for polygon in data.polygons:polygon.use_smooth=True
    obj['profile']=record['profile'];obj['frame_count']=16
    obj['inferred']='Closed clustered shell depth; source alpha, offsets and animation preserved'
    obj['source_elevation']=record['elevation']
    obj['depth_anchor']='Supporting trunk footprints; foreground Arbre05 root inferred beyond image'
    ref.hide_render=True
    for scene in bpy.data.scenes:
        for layer in scene.view_layers:
            if ref.name in layer.objects:ref.hide_set(True,view_layer=layer)
    ref['replaced_by']=NAME
    counts[record['profile']]={'vertices':len(verts),'faces':len(faces)}

# The remaining authored water/fire/butterfly overlays remain separate assets.
ambient=collection('11 Authored ambient overlay references')
for ref in references.objects:
    if ref.get('bank')!='shertree':ambient.objects.link(ref)
result={'canopies':counts,'ambient_overlays':len(ambient.objects)}
