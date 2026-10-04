"""Source-traced southwest rock profiles constrained by native relief heights."""
import json
import math
import numpy as np
from mathutils import Vector
from catalog import OUT
from tree_geometry import replace_mesh, SIN, COS, RAY
from rock_profile_geometry import resample


def profiles():
    return {
        43:[(311,900),(313,892),(315,877),(320,868),(328,861),(341,855),
            (343,848),(350,841),(360,835.5),(390,832.5),(415,832.5),(422,837),(424,848),(419,875),
            (423,900),(416,913),(404,920),(390,928),(380,936),(356,941),(344,938),(320,924),(311,910)],
        131:[(276,934),(280,925),(291,919),(308,911),(326,904),(343,912),
             (354,925),(346,938),(326,947),(302,953),(286,950),(278,944)],
        136:[(409,871),(414,850),(422,839),(430,841),(438,846),(446,850),(452,860),(457,867),(457,876),
             (449,896),(438,910),(423,914),(412,904)],
        137:[(433,879),(441,871),(449,868),(469,868),(481,871),(488,888),(484,895),
             (470,901),(456,910),(443,903),(434,895)],
    }


def build(obj):
    index=int(obj['source_node'].split('-')[-1])
    outline=profiles()[index];points=resample(outline)
    center=(points.min(axis=0)+points.max(axis=0))*.5
    direction=points-center
    cross=direction[:,0]*np.roll(direction[:,1],-1)-direction[:,1]*np.roll(direction[:,0],-1)
    if np.any(cross<=0):raise ValueError(f'Rock {index} profile reverses around its center')
    radii=(points.max(axis=0)-points.min(axis=0))*.5
    theta=np.arctan2(direction[:,1]/radii[1],direction[:,0]/radii[0])
    ellipse=center+np.column_stack((np.cos(theta)*radii[0],np.sin(theta)*radii[1]))
    native=json.loads((OUT/'baseline/Croisement02.rhp.json').read_text())['sight_obstacles'][index]['points']
    native_depth=(max(p['y'] for p in native)-min(p['y'] for p in native))/SIN
    target_height=max(p['z_top'] for p in native)/COS
    depth_radius=min(native_depth*.35,target_height/(2*SIN)*.8)
    source_vertical=np.asarray((0,-SIN,-COS));ray=np.asarray(RAY)
    d=(COS*center[1]+40)/SIN
    ring_count=48;count=len(points)
    source_points=[];depths=[]
    for j in range(1,ring_count):
        angle=math.pi*j/ring_count;radius=math.sin(angle)
        profile=ellipse+(points-ellipse)*radius**8
        xy=center+(profile-center)*radius
        source_points.extend(xy);depths.extend([depth_radius*math.cos(angle)]*count)
    source_points.extend([center,center]);depths.extend([depth_radius,-depth_radius])
    source_points=np.asarray(source_points);depths=np.asarray(depths)
    def positions(slope):
        # Source rays preserve the traced contour. Tilting the middle surface
        # toward horizontal retains the native low slabs' height and depth.
        along=d+depths+(1-slope)*COS*(source_points[:,1]-center[1])/SIN
        return (np.column_stack((source_points[:,0],np.zeros(len(source_points)),np.zeros(len(source_points))))
                +source_points[:,1,None]*source_vertical+along[:,None]*ray)
    low,high=0.,1.5
    for _ in range(40):
        middle=(low+high)*.5
        if np.ptp(positions(middle)[:,2])<target_height:low=middle
        else:high=middle
    slope=(low+high)*.5;vertices=positions(slope)
    vertices+=ray*(-vertices[:,2].min()/SIN)
    faces=[]
    for ring in range(ring_count-2):
        for k in range(count):
            n=(k+1)%count
            faces.append((ring*count+k,ring*count+n,(ring+1)*count+n,(ring+1)*count+k))
    front=len(vertices)-2;back=len(vertices)-1
    for k in range(count):
        n=(k+1)%count;faces.append((front,n,k));offset=(ring_count-2)*count
        faces.append((back,offset+k,offset+n))
    result=replace_mesh(obj,vertices.tolist(),faces,materials=list(obj.data.materials))
    for face in obj.data.polygons:face.use_smooth=True
    if result['nonmanifold_edges'] or result['degenerate_faces']:raise ValueError('Rock must be a closed volume')
    result.update(source_node=obj['source_node'],source_profile=outline,native_depth=native_depth,
        native_height=target_height,measured_height=float(np.ptp(vertices[:,2])),profile_vertical_slope=slope,
        inferred_depth_radius=depth_radius,method='Closed source profile with native-height constrained middle-surface slope',
        inference='Hidden lower/back profiles and rounded internal depth are inferred; native world height is retained')
    return result
