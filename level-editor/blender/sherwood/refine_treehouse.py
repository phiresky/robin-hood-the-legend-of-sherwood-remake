"""Central oak fork and plank-built hut, fitted to the single Day reference.

Visible branch paths and roof apex are measured in map pixels. Concealed limb
depth, wall thickness and roof underside are inferred, not original geometry.
"""
import json
import math
import sys
from pathlib import Path
import bpy
from mathutils import Vector

sys.path.insert(0, str(Path(__file__).parent))
from modeling import DATA, COS, collection, game_point, mesh, pixel_point, retire, tube

NAME = '09 Central treehouse - fork walls and thatch'
c = collection(NAME)
level = json.loads((DATA / 'Levels/Sherwood.rhp.json').read_text())

def smooth_trace(name, trace, ground_y):
    controls = [pixel_point(x,y,ground_y) for x,y,r in trace]
    points, radii = [], []
    for i in range(len(controls)-1):
        p0,p1 = controls[max(0,i-1)],controls[i]
        p2,p3 = controls[i+1],controls[min(i+2,len(controls)-1)]
        for j in range(6):
            t=j/6
            points.append(.5*((2*p1)+(-p0+p2)*t+(2*p0-5*p1+4*p2-p3)*t*t+(-p0+3*p1-3*p2+p3)*t*t*t))
            radii.append(trace[i][2]*(1-t)+trace[i+1][2]*t)
    points.append(controls[-1]); radii.append(trace[-1][2])
    obj=tube(c,name,points,radii,20)
    obj['inferred']='Cross section and hidden depth; visible branch path traced from Day map'
    return obj

smooth_trace('Central oak - left cradle limb',[(985,425,34),(951,388,29),(917,333,22),(897,286,16),(881,228,10),(857,191,4)],631)
smooth_trace('Central oak - right cradle limb',[(1003,433,34),(1040,402,29),(1081,353,24),(1114,310,18),(1137,257,12),(1154,226,4)],628)
smooth_trace('Central oak - upper rear limb',[(975,310,25),(1001,243,22),(1021,183,20),(1066,117,16),(1112,66,10),(1175,29,3)],563)
retire(49,NAME)

def slab(name, top, offset):
    bottom=[p+offset for p in top]
    n=len(top)
    return mesh(c,name,top+bottom,[tuple(range(n)),tuple(reversed(range(n,2*n)))]+[(i,(i+1)%n,(i+1)%n+n,i+n) for i in range(n)])

footprint=level['sight_obstacles'][102]['points']
base=284.001
# Preserve the measured doorway recess and wall footprint. Separate narrow
# boards give the wall physical edges in oblique views.
for edge,(a,b) in enumerate(zip(footprint,footprint[1:]+footprint[:1])):
    p,q=game_point(a['x'],a['y'],base),game_point(b['x'],b['y'],base)
    direction=q-p
    normal=Vector((-direction.y,direction.x,0)).normalized()*2
    count=max(2,round(direction.length/5.5))
    for j in range(count):
        lo,hi=(j+.018)/count,(j+.982)/count
        u,v=p.lerp(q,lo),p.lerp(q,hi)
        height=(351-base+.6*math.sin(j*2.4+edge))/COS
        slab(f'Central hut - wall {edge+1} plank {j+1:02}',[u,v,v+Vector((0,0,height)),u+Vector((0,0,height))],normal)

# Asymmetric peaked roof instead of the original overlapping gabled blocks.
eaves=[game_point(p['x'],p['y'],351) for p in footprint[:5]]
apex=game_point(1035,610,394)
for edge,(a,b) in enumerate(zip(eaves,eaves[1:]+eaves[:1])):
    count=max(3,round((b-a).length/5))
    for j in range(count):
        u=a.lerp(b,j/count); v=a.lerp(b,(j+1)/count)
        # Uneven overhanging sticks are visible around the original roof edge.
        extend=1.02+.025*math.sin(j*3.7+edge)
        u=apex+(u-apex)*extend; v=apex+(v-apex)*extend
        tip=apex+(u-apex)*.018
        slab(f'Central hut - roof strip {edge+1}-{j+1:02}',[tip,u,v],Vector((0,0,-1.8)))
        tube(c,f'Central hut - thatch rib {edge+1}-{j+1:02}',[apex+(u-apex)*.16,u],[.65,.95],6)
retire(102,NAME); retire(103,NAME)

# Front porch guard rail, braces and the two ladder flights traced in pixels.
for label,trace,cy in [
    ('Porch front rail',[(1003,336,1.6),(1043,348,1.6)],656),
    ('Porch left post',[(1004,337,1.7),(1002,364,1.7)],656),
    ('Porch right post',[(1041,347,1.7),(1039,375,1.7)],656),
    ('Lower landing left brace',[(1029,410,2.4),(1009,441,2.4)],664),
    ('Lower landing right brace',[(1069,416,2.4),(1038,445,2.4)],666),
]:
    tube(c,label,[pixel_point(x,y,cy) for x,y,r in trace],[r for x,y,r in trace],8)

for label,lt,lb,rt,rb,cy,count in [
    ('Upper porch ladder',(1027,375),(1039,394),(1040,379),(1052,398),657,5),
    ('Ring ladder',(1030,411),(1010,446),(1042,415),(1022,450),665,8),
]:
    a,b,d,e=[pixel_point(x,y,cy) for x,y in [lt,lb,rt,rb]]
    tube(c,label+' left stile',[a,b],[1.1,1.1],8)
    tube(c,label+' right stile',[d,e],[1.1,1.1],8)
    for j in range(count):
        t=(j+.5)/count
        tube(c,f'{label} rung {j+1}',[a.lerp(b,t),d.lerp(e,t)],[1,1],8)

result={'collection':NAME,'objects':len(c.objects)}
