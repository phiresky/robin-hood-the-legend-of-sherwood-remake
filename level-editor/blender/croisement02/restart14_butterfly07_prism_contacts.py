"""CPU triangle/alpha witnesses for conservative butterfly07 footprint prisms."""
import collections
import argparse
import hashlib
import json
import math
from pathlib import Path
import numpy as np
from PIL import Image
import restart14_butterfly_canopy22_audit as reader


def clip(poly, axis, limit, keep_above):
    """Clip affine vertex attributes against a closed coordinate half-plane."""
    if not len(poly):
        return poly
    result = []
    previous = poly[-1]
    old = (previous[axis] - limit) * (1 if keep_above else -1)
    for current in poly:
        value = (current[axis] - limit) * (1 if keep_above else -1)
        if (old >= 0) != (value >= 0):
            intersection = previous + old / (old - value) * (current - previous)
            intersection[axis] = limit
            result.append(intersection)
        if value >= 0:
            result.append(current)
        previous, old = current, value
    return np.asarray(result).reshape(-1, poly.shape[1])


def box_clip(poly, bounds):
    for axis, low, high in bounds:
        poly = clip(poly, axis, low, True)
        poly = clip(poly, axis, high, False)
        if not len(poly):
            break
    return poly


def bilinear_maximum(poly, values, xaxis, yaxis):
    """Exact bilinear maximum on a convex polygon, including edge extrema."""
    c00, c10, c01, c11 = values
    bx, by, cross = c10-c00, c01-c00, c11-c10-c01+c00
    best, witness = -math.inf, None
    for a, b in zip(poly, np.roll(poly, -1, axis=0)):
        x, y = a[xaxis], a[yaxis]
        dx, dy = b[xaxis]-x, b[yaxis]-y
        linear = bx*dx + by*dy + cross*(x*dy+y*dx)
        quadratic = cross*dx*dy
        times = [0., 1.]
        if quadratic < 0:
            t = -linear/(2*quadratic)
            if 0 < t < 1:
                times.append(t)
        for t in times:
            p = a+t*(b-a)
            value = c00+bx*p[xaxis]+by*p[yaxis]+cross*p[xaxis]*p[yaxis]
            if value > best:
                best, witness = float(value), p.copy()
    return best, witness


def wrapped(index, size, mode):
    if mode == 33071:
        return min(max(index, 0), size-1)
    if mode == 10497:
        return index % size
    if mode == 33648:
        index %= 2*size
        return min(index, 2*size-1-index)
    raise ValueError(f'Unsupported texture wrap {mode}')


def alpha_maximum(poly, image, sampler, factor):
    if image is None:
        return factor, poly.mean(0), 0
    height, width = image.shape[:2]
    uvpoly = poly.copy()
    uvpoly[:, 3:5] = poly[:, 3:5]*[width, height]-.5
    low = np.floor(uvpoly[:, 3:5].min(0)).astype(int)
    high = np.floor(uvpoly[:, 3:5].max(0)).astype(int)
    if np.prod(high-low+1) > 100000:
        raise ValueError('Unexpectedly large UV contact; partition explicitly before proceeding')
    best, witness, cells = -math.inf, None, 0
    for y in range(low[1], high[1]+1):
        for x in range(low[0], high[0]+1):
            piece = box_clip(uvpoly, [(3, x, x+1), (4, y, y+1)])
            if not len(piece):
                continue
            piece[:, 3:5] -= [x, y]
            values = [image[wrapped(y+oy,height,sampler.get('wrapT',10497)),
                            wrapped(x+ox,width,sampler.get('wrapS',10497)),3]/255*factor
                      for ox,oy in [(0,0),(1,0),(0,1),(1,1)]]
            value, point = bilinear_maximum(piece, values, 3, 4)
            cells += 1
            if value > best:
                best, witness = value, point
                witness[3:5] = (witness[3:5]+[x+.5,y+.5])/[width,height]
    return best, witness, cells


def main(output_name='butterfly07-prism-contacts-v2'):
    base = reader.B
    output = base/output_name
    assert not output.exists(), 'Retain prior evidence; choose a fresh output'
    proposal_path = base/'footprint-path-proposal-v2/proposal.json'
    audit_path = base/'all7-footprint-audit-v2/report.json'
    proposal = json.loads(proposal_path.read_text())
    assert reader.sha(audit_path) == proposal['footprint_sha256']
    route = next(r for r in proposal['rows'] if r['sequence']==14)
    plan = json.loads((base/'all7-context-plan-v1/plan.json').read_text())
    sequence = next(r for r in plan['sequences'] if r['index']==14)
    groups, ray_sites = {}, set()
    for flag in route['potential_tree_collision_intervals']:
        phase = flag['phase']
        frame = sequence['path'][phase]
        assert frame['phase']==phase and reader.sha(Path(frame['source']))==frame['sha256']
        source = np.asarray(Image.open(frame['source']).convert('RGBA'))
        ys,xs = np.nonzero(source[:,:,3])
        pixels = np.c_[xs+frame['bbox'][0]+.5,ys+frame['bbox'][1]+.5]
        height = route['world_zup_knots'][phase][2]
        assert np.allclose(flag['butterfly_envelope'],[height-8,height+8])
        groups[(phase,flag['owner'])] = dict(flag=flag,pixels=pixels,candidates=0,contacts=0,
            alpha_rejected=0,cells=0,center_contact_count=0,witnesses=[],contact_pixels=set(),
            receiver_height_range=[math.inf,-math.inf],node_counts=collections.Counter(),
            radius_contacts={radius:0 for radius in (0.,.5,1.,2.,4.,8.)})
        ray_sites.update(map(tuple,pixels))
    rays = [{'screen':list(p),'hits':[]} for p in sorted(ray_sites)]

    def inspect(placed,node,node_index,primitive_index,triangles,uvs,material,alpha,texture_record):
        relevant = [((phase,asset),r) for (phase,asset),r in groups.items() if asset==placed['id']]
        if not relevant:
            return
        screen = np.stack((triangles[:,:,0],reader.SIN*triangles[:,:,2]-reader.COS*triangles[:,:,1]),axis=2)
        if uvs is None:
            assert not material.get('pbrMetallicRoughness',{}).get('baseColorTexture')
            uvs = np.zeros((*triangles.shape[:2],2))
        attributes = np.concatenate((screen,triangles[:,:,1:2],uvs),axis=2)
        low,high = attributes[:,:,:3].min(1),attributes[:,:,:3].max(1)
        opaque = material.get('alphaMode','OPAQUE')=='OPAQUE'
        image,sampler,factor = (None,{},1.) if opaque else texture_record(material)
        cutoff = material.get('alphaCutoff',.5) if material.get('alphaMode')=='MASK' else .01
        for (phase,asset),r in relevant:
            zlow,zhigh = r['flag']['butterfly_envelope']
            xy = r['pixels'];bboxlow=xy.min(0)-.5;bboxhigh=xy.max(0)+.5
            eligible = np.flatnonzero(np.all(high[:,:2]>=bboxlow,axis=1)&np.all(low[:,:2]<=bboxhigh,axis=1)&(high[:,2]>=zlow)&(low[:,2]<=zhigh))
            for ti in eligible:
                slab = box_clip(attributes[ti],[(2,zlow,zhigh)])
                if not len(slab):
                    continue
                inside = np.flatnonzero(np.all(xy+.5>=slab[:,:2].min(0),axis=1)&np.all(xy-.5<=slab[:,:2].max(0),axis=1))
                for pix in inside:
                    x,y = xy[pix]
                    piece = box_clip(slab,[(0,x-.5,x+.5),(1,y-.5,y+.5)])
                    if not len(piece):
                        continue
                    r['candidates'] += 1
                    value,witness,cells = alpha_maximum(piece,image,sampler,factor)
                    r['cells'] += cells
                    if value < cutoff:
                        r['alpha_rejected'] += 1
                        continue
                    checked = 1. if opaque else alpha(material,witness[3:5],True)
                    assert abs(value-checked)<1e-7, (value,checked)
                    r['contacts'] += 1;r['contact_pixels'].add(int(pix));r['node_counts'][node.get('name','')]+=1
                    for radius in r['radius_contacts']:
                        center = (zlow+zhigh)/2
                        narrow = box_clip(piece,[(2,center-radius,center+radius)])
                        if len(narrow) and alpha_maximum(narrow,image,sampler,factor)[0] >= cutoff:
                            r['radius_contacts'][radius] += 1
                    r['receiver_height_range'][0]=min(r['receiver_height_range'][0],float(piece[:,2].min()))
                    r['receiver_height_range'][1]=max(r['receiver_height_range'][1],float(piece[:,2].max()))
                    if len(r['witnesses'])<6:
                        sx,sy,z,u,v=map(float,witness)
                        r['witnesses'].append(dict(node=node.get('name'),node_index=node_index,primitive_index=primitive_index,
                            triangle_index=int(ti),source_pixel_center=[float(x),float(y)],screen=[sx,sy],world_zup=[sx,-(sy+reader.COS*z)/reader.SIN,z],uv=[u,v],alpha=value,cutoff=cutoff,
                            source_height_margin=[z-zlow,zhigh-z],material=material.get('name')))

    def finish(rays,records,map_path):
        rows=[]
        for key,r in groups.items():
            phase,asset=key;zlo,zhi=r['flag']['butterfly_envelope']
            for ray in rays:
                if tuple(ray['screen']) not in set(map(tuple,r['pixels'])):
                    continue
                r['center_contact_count'] += sum(h['asset']==asset and h['passes_alpha_only'] and zlo<=h['world_yup'][1]<=zhi for h in ray['hits'])
            rows.append(dict(sequence=14,butterfly=7,phase=phase,receiver=asset,
                source_pixel_count=len(r['pixels']),envelope=[zlo,zhi],exact_triangle_pixel_prism_contacts=r['contacts'],
                intersecting_source_pixels=len(r['contact_pixels']),center_ray_contacts=r['center_contact_count'],
                transparent_triangle_pixel_prisms=r['alpha_rejected'],tested_triangle_pixel_prisms=r['candidates'],tested_bilinear_cells=r['cells'],
                status='RECEIVER_INTERSECTS_CONSERVATIVE_PRISM' if r['contacts'] else 'CONSERVATIVE_INTERVAL_FALSE_POSITIVE',
                receiver_height_range=r['receiver_height_range'] if r['contacts'] else None,node_counts=dict(r['node_counts']),
                contacts_by_uniform_half_depth=r['radius_contacts'],witnesses=r['witnesses']))
        report=dict(status='EXACT_RECEIVER_PRISM_CONTACTS_NOT_ANATOMY_OR_PATH_APPROVAL',map_sha256=reader.sha(map_path),
            proposal_sha256=reader.sha(proposal_path),footprint_audit_sha256=reader.sha(audit_path),assets=records,rows=rows,
            recipe_sha256=reader.sha(Path(__file__)),reader_sha256=reader.sha(Path(reader.__file__)),
            method='Exact transformed receiver triangles clipped to each full positive-alpha source pixel square and conservative +/-8 world-Z slab. Bilinear level0 alpha maximum computed on each clipped UV polygon, including all edge stationary extrema. Both physical sides tested.',
            native_composition='Separate adapter obligation. No camera-visibility altitude lift, path or model change.',
            disk_scope='Root explicitly assigned CPU-only below8GiB: >=7GiB reserve plus1MiB bounded new evidence. No render/model/API.',
            limits=['Butterfly07 anatomy is unbuilt. Contact with conservative source-pixel prisms is not proof of contact with a future actual butterfly mesh.',
                    'Only the six flagged owner/phase pairs tested; this is not a continuous swept-path proof.',
                    'Uniform half-depth sensitivity is diagnostic, not a butterfly anatomy prescription; zero denotes a horizontal source-footprint plane.',
                    'Level0 bilinear alpha is exact here; view-dependent GPU mip alpha and source pixel partial coverage remain outside this test.'])
        reader.guard(7*1024**3,1024**2)
        payload=json.dumps(report,indent=2)+'\n';assert len(payload.encode())<1024**2
        output.mkdir();(output/'report.json').write_text(payload)
        print(json.dumps([(r['phase'],r['receiver'],r['exact_triangle_pixel_prism_contacts'],r['center_ray_contacts'])for r in rows]),flush=True)
        return report

    reader.main(ray_records=rays,postprocess=finish,output=output,asset_ids={k[1]for k in groups},
                triangle_callback=inspect,query_margin=.5,minimum_free_bytes=7*1024**3,output_limit_bytes=1024**2)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-name',default='butterfly07-prism-contacts-v2')
    main(parser.parse_args().output_name)
