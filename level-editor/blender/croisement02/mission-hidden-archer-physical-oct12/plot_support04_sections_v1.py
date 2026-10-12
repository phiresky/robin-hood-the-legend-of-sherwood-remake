"""Plot exact local mesh cross-sections where full receiver surfaces obscure a join."""
import json, math, mmap, struct, sys
from pathlib import Path
import numpy as np
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
from matplotlib.collections import LineCollection
HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import qualify as q
S, C = math.sin(math.radians(35)), math.cos(math.radians(35))

def receiver_triangles(candidate, witnesses):
    path = q.LIB / candidate['model']
    assert q.pin(path)['sha256'] == candidate['model_sha256']
    with path.open('rb') as stream, mmap.mmap(stream.fileno(), 0, access=mmap.ACCESS_READ) as raw:
        size = struct.unpack_from('<I', raw, 12)[0]
        doc = json.loads(raw[20:20+size]); offset = 28+size
        def acc(i):
            a = doc['accessors'][i]; v = doc['bufferViews'][a['bufferView']]
            assert v.get('buffer', 0) == 0 and 'sparse' not in a
            dt = np.dtype({5126:'<f4',5125:'<u4',5123:'<u2'}[a['componentType']])
            width = {'SCALAR':1,'VEC3':3}[a['type']]
            return np.ndarray((a['count'],width), dtype=dt, buffer=raw,
                offset=offset+v.get('byteOffset',0)+a.get('byteOffset',0),
                strides=(v.get('byteStride',dt.itemsize*width),dt.itemsize))
        t = candidate['placement']['transform']; assert t['rot_deg'] == 0
        world = np.eye(4); world[:3,3] = [t['dx'],t['dz'],t['dy']/S]
        transforms = {}
        def walk(i, parent):
            transforms[i] = parent @ q.matrix(doc['nodes'][i])
            for child in doc['nodes'][i].get('children',[]): walk(child, transforms[i])
        for i in doc['scenes'][doc.get('scene',0)]['nodes']: walk(i, world)
        result = []
        for w in witnesses:
            ni = next(i for i,n in enumerate(doc['nodes']) if n.get('name') == w['node'])
            node = doc['nodes'][ni]
            p = doc['meshes'][node['mesh']]['primitives'][w['primitive_index']]
            assert doc['materials'][p['material']].get('alphaMode','OPAQUE') == 'OPAQUE'
            ti = w['triangle_index']; indices = acc(p['indices']).ravel()[ti*3:ti*3+3]
            v = acc(p['attributes']['POSITION'])[indices].copy()
            v = (np.c_[v,np.ones(3)] @ transforms[ni].T)[:,:3]
            result.append(np.c_[v[:,0], S*v[:,2]-C*v[:,1], C*v[:,2]+S*v[:,1]])
        return result

def camera(vertices):
    v = np.asarray(vertices)
    return np.c_[v[:,0], -S*v[:,1]-C*v[:,2], -C*v[:,1]+S*v[:,2]]

def cut(vertices, axis, value):
    hits = []
    for a,b in zip(vertices, np.roll(vertices,-1,axis=0)):
        da,db = a[axis]-value,b[axis]-value
        if abs(da) < 1e-7: hits.append(a)
        if da*db < 0: hits.append(a+(b-a)*(-da)/(db-da))
    if len(hits) < 2: return None
    # Polygon edges are convex here; choose the most separated intersections.
    a,b = max(((a,b) for a in hits for b in hits), key=lambda ab: np.linalg.norm(ab[0]-ab[1]))
    return np.array([a,b])[:,[1-axis,2]]

def main():
    base = q.OUT/'support04-saved-junction-readback-v1'
    saved = json.loads((base/'report.json').read_text())
    sweep = json.loads((q.OUT/'geometry-v8-support04-sweep-v4/report.json').read_text())
    qualification = json.loads((q.OUT/'qualification.json').read_text())['rows'][3]
    dest = q.OUT/'support04-junction-sections-v1'; assert not dest.exists(); dest.mkdir()
    reviews = []
    for row in saved['records']:
        construction = row['construction']; target = construction['candidate']['target']
        witness = next(w for w in sweep['rows'] if (w['phase'],w['component']) == (row['phase'],row['component']))
        assert set(witness['counts']) == {'TARGET_WOOD_REQUIRES_JUNCTION_REVIEW'}
        receiver = next(c for c in qualification['bounds_candidates'] if c['id'] == target['asset'])
        wood = receiver_triangles(receiver, witness['witnesses'])
        support = camera(row['vertices'])
        leaf = [camera(t['vertices']) for t in row['source_foliage_triangles']]
        sx,sy = construction['candidate']['screen']; depth = target['camera_depth']; gap = construction['candidate']['gap']
        fig, axes = plt.subplots(1,2,figsize=(10,4),layout='constrained')
        for axis,ax in enumerate(axes):
            counts = {}
            for label, polygons, color, width in [
                ('Foliage',leaf,'#477647',1.3),('Target wood',wood,'#805326',2),
                ('Saved support',[support[f] for f in row['faces']],'#ed6b17',2.5)]:
                segments = [cut(p,axis,[sx,sy][axis]) for p in polygons]
                segments = [s-np.array([[sy,sx][axis],depth]) for s in segments if s is not None]
                counts[label] = len(segments)
                if segments: ax.add_collection(LineCollection(segments,colors=color,linewidths=width,label=label))
            assert counts['Saved support'] > 0 and counts['Foliage'] > 0 and counts['Target wood'] > 0, counts
            ax.set_xlim(-1.5,1.5); ax.set_ylim(-.6,gap+1.0); ax.set_aspect('equal',adjustable='box')
            ax.set_xlabel(('Native Y','Native X')[axis]+' offset (units)')
            ax.set_ylabel('Depth ahead of target hit (units)');ax.grid(alpha=.25)
            ax.set_title(('Slice at native X','Slice at native Y')[axis]); ax.legend(fontsize=8)
        fig.suptitle(f"{row['phase']} / component {row['component']} — gap {gap:.3f}, radius 0.200")
        path = dest/f"{row['phase']}-{row['component']}.png";fig.savefig(path,dpi=110);plt.close(fig)
        reviews.append(dict(phase=row['phase'],component=row['component'],target=target['node'],sheet=q.pin(path)))
    (dest/'report.json').write_text(json.dumps(dict(status='EXACT_SAVED_JUNCTION_SECTIONS_PENDING_REVIEW',
        model=saved['model'],readback=q.pin(base/'report.json'),sweep=q.pin(q.OUT/'geometry-v8-support04-sweep-v4/report.json'),
        reviews=reviews,recipe=q.pin(Path(__file__)),limits=[
        'Diagnostic mesh cross-sections, not appearance renders. Exact local saved foliage/support and target wood triangles.',
        'Other receivers excluded here; full finite sweep separately checked them.',
        'Target wood may be oblique or curved; these two local slices do not claim a reconstructed branch structure.']),indent=2)+'\n')
    print(dest)

if __name__ == '__main__': main()
