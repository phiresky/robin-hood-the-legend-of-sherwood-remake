"""Trial broad convex upper-bank masses, retaining source creases and closed contacts."""
import hashlib
import json
import math
from collections import Counter, defaultdict
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

from restart2_bank_boulder_faces import inside, project

R = Path(__file__).resolve().parents[3]
B = R / 'level-editor/work/active/croisement03/restart2'
O = B / 'bank-main-masses-cpu-v1'
S, C = math.sin(math.radians(35)), math.cos(math.radians(35))
RAY = np.array([0., -C, S])
MASSES = [
    {'id': 'broad-west-upper-stone', 'amplitude': 16.,
     'outline': [[153,216],[167,193],[184,177],[211,170],[239,173],[268,177],
                 [291,188],[292,215],[280,239],[258,255],[235,265],[209,264],
                 [184,251],[168,238]],
     'evidence': 'Exposed rounded warm stone on left upper bank; eastern edge partly under foliage.'},
    {'id': 'broad-central-upper-body', 'amplitude': 12.,
     'outline': [[274,180],[305,164],[352,161],[402,163],[451,168],[495,166],
                 [541,178],[576,188],[600,201],[601,228],[580,247],[538,255],
                 [485,260],[425,260],[368,265],[316,259],[284,239]],
     'evidence': 'Upper enclosing bank body; most lower edge is foreground-covered and explicitly inferred.'},
]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def distances(points, a, b):
    delta = b-a
    t = np.clip(np.sum((points-a)*delta, axis=1)/np.dot(delta, delta), 0, 1)
    return np.linalg.norm(points-a-t[:, None]*delta, axis=1)


def main():
    assert not O.exists()
    source = B / 'bank-convex-masses-cpu-v1/geometry.json'
    data = json.loads(source.read_text())
    mesh = data['52']
    old = np.array(mesh['vertices'])
    vertices = old.copy()
    faces = mesh['faces']
    top = [all(i < mesh['source_vertex_count'] for i in face) for face in faces]
    segments = [(old[a], old[b]) for trace in mesh['traces']
                for a, b in zip(trace['vertices'], trace['vertices'][1:])]
    for iteration in range(2):
        points = vertices.tolist()
        midpoints = {}
        new, labels = [], []
        def middle(a, b):
            key = tuple(sorted((a,b)))
            if key not in midpoints:
                midpoints[key] = len(points)
                points.append(((vertices[a]+vertices[b])*.5).tolist())
            return midpoints[key]
        for (a,b,c), is_top in zip(faces, top):
            ab,bc,ca = middle(a,b),middle(b,c),middle(c,a)
            new.extend([[a,ab,ca],[ab,b,bc],[ca,bc,c],[ab,bc,ca]])
            labels.extend([is_top]*4)
        vertices, faces, top = np.array(points), new, labels
    before = vertices.copy()
    edge_faces = defaultdict(list)
    for index, face in enumerate(faces):
        for a,b in zip(face,face[1:]+face[:1]):
            edge_faces[tuple(sorted((a,b)))].append(index)
    fixed = {i for edge, parents in edge_faces.items() if not all(top[f] for f in parents) for i in edge}
    active = sorted({i for face,label in zip(faces,top) if label for i in face}-fixed)
    xy = project(vertices)
    trace_distance = np.full(len(vertices), np.inf)
    for a,b in segments:
        pa,pb = project(np.array([a,b]))
        trace_distance = np.minimum(trace_distance, distances(xy,pa,pb))
    displacement = np.zeros(len(vertices))
    mass_counts = []
    for mass in MASSES:
        polygon = mass['outline']
        included = np.array([i for i in active if inside(xy[i],polygon)],dtype=int)
        if not len(included):
            raise AssertionError(mass['id'])
        distance = np.full(len(included),np.inf)
        for a,b in zip(polygon,polygon[1:]+polygon[:1]):
            distance=np.minimum(distance,distances(xy[included],np.array(a),np.array(b)))
        radial=np.clip(distance/distance.max(),0,1)
        # Complete broad convex profiles, fading at retained source creases.
        curve=np.sqrt(radial*(2-radial))*mass['amplitude']
        curve*=np.minimum(1,trace_distance[included]/6)
        displacement[included]=np.maximum(displacement[included],curve)
        mass_counts.append({'id':mass['id'],'vertices':len(included),'max_shift':float(curve.max())})
    displacement[list(fixed)] = 0
    displacement[trace_distance < 1e-7] = 0
    vertices += displacement[:,None]*RAY
    assert np.max(np.abs(project(vertices)-xy)) < 1e-9
    assert np.array_equal(vertices[list(fixed)],before[list(fixed)])
    for trace in mesh['traces']:
        assert np.max(np.abs(vertices[trace['vertices']]-old[trace['vertices']])) < 1e-9
    counts=Counter(tuple(sorted((a,b))) for face in faces for a,b in zip(face,face[1:]+face[:1]))
    assert set(counts.values()) == {2}
    assert len(vertices)-len(counts)+len(faces)==2
    volume=sum(np.dot(vertices[f[0]],np.cross(vertices[f[1]],vertices[f[2]]))/6 for f in faces)
    assert volume>0
    mesh.update(vertices=vertices.tolist(),faces=faces,signed_volume=float(volume),edge_count=len(counts),euler=2)
    mesh['main_mass_revision']={'source_sha256':sha(source),'mass_proposals':MASSES,'mass_counts':mass_counts,
        'changed_vertices':int((displacement>1e-8).sum()),'fixed_boundary_vertices':len(fixed),
        'depth_provenance':'New depths are hypotheses. Broad visible western stone convexity is source-supported; central covered body is inferred. No source material ownership inferred.',
        'old_world':before.tolist(),'native_ray_displacements':displacement.tolist(),
        'source_2d_error':float(np.max(np.abs(project(vertices)-xy))),
        'required':['Independent full source/crest/neighbor guards','Saved Boolean/closed/contact guards','Actual native/solid8/oblique foreground-context review']}
    O.mkdir()
    (O/'geometry.json').write_text(json.dumps(data,separators=(',',':'))+'\n')
    image=Image.open(B.parent/'baseline/covered.png').convert('RGB')
    draw=ImageDraw.Draw(image)
    for mass in MASSES:
        draw.line([tuple(p)for p in mass['outline']+[mass['outline'][0]]],fill='#00ffff',width=1)
    image.crop((130,150,635,330)).resize((1515,540),Image.Resampling.NEAREST).save(O/'source-mass-outline-proposal.png')
    print({'vertices':len(vertices),'faces':len(faces),'changes':int((displacement>1e-8).sum())})


if __name__ == '__main__':
    main()
