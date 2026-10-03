"""Rounded hay mound fitted to the visible crest and ground contact."""
import math
from tree_geometry import SIN, replace_mesh


def haystack(objects):
    if {o['source_node'] for o in objects} != {'building-140', 'building-141'}:
        raise ValueError('Haystack canonical parts differ')
    rings, segments = 12, 32
    for obj in objects:
        left = obj['source_node'] == 'building-140'
        start = math.pi/2 if left else -math.pi/2
        vertices = [(956, -1036/SIN+30, 60)]
        faces = []
        for ring in range(1, rings+1):
            r = ring/rings
            for j in range(segments+1):
                angle = start+j*math.pi/segments
                uneven = 1+.025*math.sin(5*angle)+.015*math.cos(9*angle)
                x = 956+58*r*math.cos(angle)*uneven
                y = -1036/SIN+55*r*math.sin(angle)*uneven+30*(1-r)
                z = 60*(1-r**1.7)**1.5
                z += 1.1*math.sin(7*angle+r*8)*math.sin(math.pi*r)
                vertices.append((x,y,z))
        for j in range(segments):
            faces.append((0,1+j,2+j))
        for ring in range(rings-1):
            a = 1+ring*(segments+1)
            b = a+segments+1
            for j in range(segments):
                faces.append((a+j,b+j,b+j+1,a+j+1))
        base = 1+(rings-1)*(segments+1)
        faces.append(tuple(base+j for j in reversed(range(segments+1))))
        # Close each half along its shared section; no detached open sheets.
        section = [0]+[1+r*(segments+1) for r in range(rings)]
        section += [1+r*(segments+1)+segments for r in reversed(range(rings))]
        faces.append(tuple(reversed(section)))
        report = replace_mesh(obj,vertices,faces,materials=list(obj.data.materials))
        if report['nonmanifold_edges'] or report['degenerate_faces']:
            raise ValueError('Haystack surface is not closed')
        for face in obj.data.polygons:
            face.use_smooth = face.index < rings*segments
