"""Closed rounded rock lobes lofted from reviewed source silhouettes."""
import json
import math
from pathlib import Path
import numpy as np
from PIL import Image
from mathutils import Vector
from catalog import OUT
from tree_geometry import replace_mesh, SIN, COS, RAY


def profiles():
    domain=np.asarray(Image.open(OUT/'west-rock-source-revision/domain-350.png'))>0
    top=np.where(domain.any(axis=0),domain.argmax(axis=0),-1)
    def crest(a,b):
        # One-pixel margin contains the native antialiased boundary. The
        # conservative minimum retains small source-supported crest peaks.
        return [(x,float(top[max(0,x-1):min(1792,x+2)].min())-.75)
                for x in range(a,b+1,2)]
    return {
        41:[(-8,352),(-4,342),(-1,336),*crest(0,0),*crest(4,96),
            (102,348),(98,372),(84,389),(60,399),(30,398),(3,383),(-4,369)],
        40:[(65,344),(70,325),(80,318),(88,320),(96,327),(106,339),
            (110,355),(105,368),(91,376),(76,368)],
        42:[(84,363),(87,351),(98,346),(110,353),(122,357),(129,368),
            (126,383),(116,395),(101,398),(90,387)],
        39:[(111,387),(115,366),*crest(116,176),(185,370),(190,390),
            (184,411),(171,426),(148,433),(129,430),(117,418)],
        38:[(183,377),(186,365),*crest(188,224),(232,378),(236,399),
            (227,414),(210,418),(193,409),(186,397)],
        37:[(220,381),(225,362),*crest(226,268),(274,380),(272,395),
            (260,405),(241,412),(225,402)],
    }


def resample(outline, spacing=2.):
    result=[]
    for a,b in zip(outline,outline[1:]+outline[:1]):
        distance=math.dist(a,b)
        for k in range(max(1,math.ceil(distance/spacing))):
            t=k/max(1,math.ceil(distance/spacing))
            result.append((a[0]*(1-t)+b[0]*t,a[1]*(1-t)+b[1]*t))
    return np.asarray(result)


def build(obj):
    index=int(obj['source_node'].split('-')[-1])
    outline=profiles()[index]
    points=resample(outline)
    center=(points.min(axis=0)+points.max(axis=0))*.5
    direction=points-center
    cross=direction[:,0]*np.roll(direction[:,1],-1)-direction[:,1]*np.roll(direction[:,0],-1)
    if np.any(cross<=0):
        raise ValueError('A radial rock profile must not reverse around its center')
    radii=(points.max(axis=0)-points.min(axis=0))*.5
    theta=np.arctan2((points[:,1]-center[1])/radii[1],(points[:,0]-center[0])/radii[0])
    ellipse=center+np.column_stack((np.cos(theta)*radii[0],np.sin(theta)*radii[1]))
    native=json.loads((OUT/'baseline/Croisement02.rhp.json').read_text())['sight_obstacles'][index]['points']
    native_depth=(max(p['y'] for p in native)-min(p['y'] for p in native))/SIN
    depth_radius=max(16.,min(42.,native_depth*.42))
    # 40/41 are adjoining lobes of the same large left boulder, not distinct
    # invented rocks. Their closed surfaces overlap below the observed seam.
    if index==41:depth_radius=34.
    source_vertical=Vector((0,-SIN,-COS))
    d=(COS*center[1]+40)/SIN
    vertices=[]
    ring_count=24;count=len(points)
    for j in range(1,ring_count):
        angle=math.pi*j/ring_count
        radius=math.sin(angle)
        # Smooth elliptical poles blend into the exact observed outline. A
        # homothetic copy of an irregular contour at every ring makes a cone
        # with radial seams at its center instead of a rounded boulder.
        profile=ellipse+(points-ellipse)*radius**4
        xy=center+(profile-center)*radius
        depth=d+depth_radius*math.cos(angle)
        for x,y in xy:
            vertices.append(tuple(Vector((float(x),0,0))+source_vertical*float(y)+RAY*depth))
    front=len(vertices);vertices.append(tuple(Vector((float(center[0]),0,0))+source_vertical*float(center[1])+RAY*(d+depth_radius)))
    back=len(vertices);vertices.append(tuple(Vector((float(center[0]),0,0))+source_vertical*float(center[1])+RAY*(d-depth_radius)))
    faces=[]
    for ring in range(ring_count-2):
        for k in range(count):
            n=(k+1)%count
            faces.append((ring*count+k,ring*count+n,(ring+1)*count+n,(ring+1)*count+k))
    for k in range(count):
        n=(k+1)%count
        faces.append((front,n,k));offset=(ring_count-2)*count
        faces.append((back,offset+k,offset+n))
    # Contact the base plane by translating along the source ray. The observed
    # outline and texture coordinates are unchanged by this depth placement.
    lift=-min(v[2] for v in vertices)/SIN
    vertices=[tuple(Vector(v)+RAY*lift) for v in vertices]
    result=replace_mesh(obj,vertices,faces,materials=list(obj.data.materials))
    for face in obj.data.polygons:face.use_smooth=True
    if result['nonmanifold_edges'] or result['degenerate_faces']:
        raise ValueError('Rock profile must form a closed nondegenerate volume')
    result.update(source_node=obj['source_node'],source_profile=outline,
        native_depth=native_depth,inferred_depth_radius=depth_radius,
        method='Closed radial loft from source-supported upper outline and inferred lower/back shape',
        inference='Lower silhouette hidden by plants, rounded depth and left off-map continuation are inferred')
    return result
