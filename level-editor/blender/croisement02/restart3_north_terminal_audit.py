"""Reopen a terminal cart and audit source coverage, receiver contact and support groups."""
import sys,json,hashlib
from pathlib import Path
import bpy,numpy as np
from mathutils.bvhtree import BVHTree
from scipy.spatial import ConvexHull
from PIL import Image
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from evidence_io import sha,write_json
from render_slots import acquire,release
from restart3_north_cart_support import audit


def main():
    worker=Path(sys.argv[sys.argv.index('--')+1]);model=worker/'worker.blend';dest=worker/'terminal-audit-v1';dest.mkdir(exist_ok=False)
    audit(worker,dest/'receiver-raw')
    # The reusable receiver diagnostic expects an intact four-wheel cart. Its raw
    # status is not applicable to the broken endpoint; assess its measurements here.
    raw=json.loads((dest/'receiver-raw/report.json').read_text());bpy.ops.wm.open_mainfile(filepath=str(model))
    objects=[o for o in bpy.context.scene.objects if o.type=='MESH'];trees={};mass={}
    meta=json.loads((worker/'manifest.json').read_text());ownership=json.loads((worker/'ownership.json').read_text())
    original=np.array(Image.open(meta['source_frame']['image']).convert('RGBA'));labels=np.array(Image.open(worker/'ownership-labels.png'))
    packed={hashlib.sha256(bytes(im.packed_file.data)).hexdigest() for im in bpy.data.images if im.packed_file}
    guard_rows=[]
    for index,row in enumerate(ownership['roles'],1):
        image=np.array(Image.open(row['image']).convert('RGBA'));active=image[:,:,3]>0
        exact=bool(np.array_equal(image[active,:3],original[active,:3]))
        owned=bool(np.array_equal(active,labels==index));guard_rows.append(dict(role=row['name'],native_rgb_exact=exact,exclusive_ownership_exact=owned,packed_image_exact=row['sha256'] in packed))
    material_pass=all(all(row[k] for k in ['native_rgb_exact','exclusive_ownership_exact','packed_image_exact']) for row in guard_rows)

    for obj in objects:
        vertices=np.array([list(obj.matrix_world@v.co) for v in obj.data.vertices],dtype=np.float64);obj.data.calc_loop_triangles();faces=[tuple(t.vertices) for t in obj.data.loop_triangles]
        trees[obj.name]=BVHTree.FromPolygons(vertices.tolist(),faces,all_triangles=True)
        origin=vertices.mean(axis=0);relative=vertices-origin
        a,b,c=[relative[np.array(faces)[:,i]] for i in range(3)];vol=np.einsum('ij,ij->i',a,np.cross(b,c))/6
        volume=float(vol.sum());centroid=origin+np.sum((a+b+c)/4*vol[:,None],axis=0)/volume
        mass[obj.name]=(abs(volume),centroid)
    edges={n:[] for n in trees}
    for i,a in enumerate(trees):
        for b in list(trees)[i+1:]:
            if trees[a].overlap(trees[b]):edges[a].append(b);edges[b].append(a)
    pending=set(edges);groups=[];measurements={r['object']:r for r in raw['objects']}
    while pending:
        connected={next(iter(pending))}
        while True:
            expanded=connected|set().union(*(set(edges[n]) for n in connected))
            if expanded==connected:break
            connected=expanded
        pending-=connected;contacts=[p for n in connected for p in measurements[n]['contacts']]
        volume=sum(mass[n][0] for n in connected);com=sum((mass[n][0]*mass[n][1] for n in connected))/volume
        margin=None
        if len(contacts)>=3:
            xy=np.unique(np.round(np.array(contacts)[:,:2],5),axis=0)
            if len(xy)>=3 and np.linalg.matrix_rank(xy-xy[0])==2:
                hull=ConvexHull(xy);margin=float(-np.max(hull.equations[:,:2]@com[:2]+hull.equations[:,2]))
        groups.append(dict(objects=sorted(connected),contacts=len(contacts),uniform_density_center=com.tolist(),support_hull_margin=margin))
    source=Image.open(worker/'source.png').convert('RGBA');actual=Image.open(worker/'native-actual.png').convert('RGBA');solid=Image.open(worker/'native-solid.png').convert('RGBA');source=source.resize(actual.size,Image.Resampling.NEAREST)
    sheet=Image.new('RGBA',(actual.width*3,actual.height));sheet.paste(source,(0,0));sheet.paste(actual,(actual.width,0));sheet.paste(solid,(actual.width*2,0));sheet.save(dest/'source-actual-solid.png')
    a=np.array(source)[:,:,3]>127;b=np.array(solid)[:,:,3]>127
    clear=all(r['minimum_clearance']>=-.05 and not r['receiver_misses'] for r in raw['objects']);supported=all(g['support_hull_margin'] is not None and g['support_hull_margin']>=-.05 for g in groups)
    report=dict(status='PASS' if clear and supported and material_pass else 'HOLD',model_sha256=sha(model),receiver_clearance_pass=clear,all_support_groups_pass=supported,
      material_guards_pass=material_pass,material_guards=guard_rows,receiver_receipt_sha256=sha(dest/'receiver-raw/report.json'),surface_connections=edges,support_groups=groups,
      native_solid_iou=float((a&b).sum()/(a|b).sum()),missing_native_pixels=int((a&~b).sum()),extra_solid_pixels=int((b&~a).sum()),
      limits=['Surface intersections do not distinguish intended joints from improper penetration; actual and solid views need independent review.',
              'Uniform-density support is a conservative diagnostic, not a claim about true cargo mass or dynamic stability.',
              'Coverage uses independently traced domains, leaving unassigned native regions outside physical claims.'])
    write_json(dest/'report.json',report);print(json.dumps(report,indent=2))

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
