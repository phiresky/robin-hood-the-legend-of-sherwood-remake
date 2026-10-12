"""Prepare inferred regional plant volumes and explicit support hypotheses."""
import hashlib
import json
import math
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw

R = Path(__file__).resolve().parents[3]
B = R / 'level-editor/work/active/croisement03/restart2'
S, C = math.sin(math.radians(35)), math.cos(math.radians(35))
RAY = np.array([0., -C, S])
UP = np.array([0., -S, -C])


def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def main():
    out = B / 'tree18-regional-green-cpu-v3'
    assert not out.exists()
    paths = [B / 'tree18-regional-plants-source-v2/receipt.json',
             B / 'bank-coupled-left-cpu-v3/geometry.json',
             B / 'bank53-mass-bulk-v2/geometry.json', Path(__file__)]
    proposal, bank, left = [json.loads(p.read_text()) for p in paths[:3]]
    pins = {str(p.resolve()): sha(p) for p in paths}
    source = Path(next(p for p in proposal['pins'] if p.endswith('/baseline/covered.png')))
    assert sha(source) == proposal['pins'][str(source)]
    pins[str(source)] = sha(source)
    triangles, owners = [], []
    for owner, geometry in [('52', bank['52']), ('53', left), ('54', bank['54'])]:
        vertices = np.array(geometry['vertices'])
        for face in geometry['faces']:
            for j in range(1, len(face)-1):
                triangles.append(vertices[[face[0], face[j], face[j+1]]]); owners.append(owner)
    xyz = np.array(triangles)
    projection = np.stack((xyz[:,:,0], -xyz[:,:,1]*S-xyz[:,:,2]*C), axis=2)
    lo, hi = projection.min(1), projection.max(1)

    def support(x, y):
        xy = np.array([x, y]); candidates = np.flatnonzero(np.all(lo <= xy, axis=1) & np.all(hi >= xy, axis=1))
        best = None
        for index in candidates:
            projected = projection[index]; a, b = projected[1]-projected[0], projected[2]-projected[0]
            determinant = a[0]*b[1]-a[1]*b[0]
            if abs(determinant) < 1e-10: continue
            q = xy-projected[0]; u = (q[0]*b[1]-q[1]*b[0])/determinant; v = (a[0]*q[1]-a[1]*q[0])/determinant
            if min(u, v, 1-u-v) < -1e-8: continue
            point = xyz[index,0]*(1-u-v)+xyz[index,1]*u+xyz[index,2]*v
            distance = float(point @ RAY)
            if best is None or distance > best[0]: best = distance, point, owners[index], int(index)
        return best

    reserved = Image.new('1', (1408,960)); draw = ImageDraw.Draw(reserved)
    for opening in proposal['reserved_openings_and_context']:
        if 'outline' in opening: draw.polygon([tuple(v) for v in opening['outline']], fill=1)
        else: draw.line([tuple(v) for v in opening['line']], fill=1, width=opening['review_width_px'])
    reserved_array = np.array(reserved)
    native_path = B / 'tree18-regional-plants-source-v1/native-union-priority-only.png'
    native_image = np.array(Image.open(native_path))
    native = native_image[:,:,3] > 0 if native_image.ndim == 3 and native_image.shape[2] == 4 else native_image > 0
    if native.ndim == 3: native = np.any(native, axis=2)
    pins[str(native_path)] = sha(native_path)
    assert int(native.sum()) == 22406
    stone_path=B/'bank-coupled-left-full-source-v3/projection.json';stone=json.loads(stone_path.read_text());pins[str(stone_path)]=sha(stone_path)
    stone_pixels={tuple(q)for q in stone['accepted_pixels']}|{tuple(q[:2])for q in stone['protected_firsthits']};assert len(stone_pixels)==29000
    protected=np.zeros_like(native)
    for x,y in stone_pixels:protected[y,x]=True
    rng = np.random.default_rng(18354)
    parts = []; contacts = []; preview = Image.open(source).convert('RGB'); pen = ImageDraw.Draw(preview)
    for region in proposal['regions'][:3]:
        region_mask = Image.new('1', (1408,960)); ImageDraw.Draw(region_mask).polygon([tuple(v) for v in region['outline']],fill=1)
        admissible = np.array(region_mask) & ~reserved_array & ~native & ~protected
        vertices, faces, clumps, rejected = [], [], [], 0
        xmin,ymin = np.min(region['outline'],axis=0); xmax,ymax = np.max(region['outline'],axis=0)
        ground = region['contacts'].get('ground_contact',region['contacts'].get('grass_contact',region['contacts'].get('lower_leaf_cover_to_grass')))
        ground = sorted(ground)
        candidates=rng.uniform([xmin+3,ymin+3],[xmax-3,ymax-3],size=(int((xmax-xmin)*(ymax-ymin)//9),2))
        admitted=[]
        for candidate in candidates:
            for x,y in [candidate]:
                if admitted and np.min(np.linalg.norm(np.array(admitted)-[x,y],axis=1))<4.8:continue
                if not admissible[int(y),int(x)]: continue
                # Whole projected clump must avoid reserved corridors. This is a
                # geometry-placement exclusion, never an accepted texture mask.
                rx,ry = rng.uniform(1.8,3.4), rng.uniform(2.2,4.2)
                extent=max(rx,ry)*1.12
                patch = admissible[max(0,int(y-extent)-1):int(y+extent)+2,max(0,int(x-extent)-1):int(x+extent)+2]
                if not patch.size or not patch.all(): rejected += 1; continue
                admitted.append([x,y])
                grounded_y = float(np.interp(x,[p[0]for p in ground],[p[1]for p in ground]))
                backing = support(x,y)
                if region['id'] == 'left-bank-climbing-ivy' and backing is not None:
                    point = backing[1]+RAY*2.5; method='draped_on_private_bank_firsthit'; owner=backing[2]; triangle=backing[3]
                else:
                    z=max(2.,(grounded_y-y)/C*.68)
                    point=np.array([x,(-y-z*C)/S,z]);method='inferred_plant_rise_from_source_ground_contact';owner=None;triangle=None
                # Several staggered leaf-sized closed lobes retain genuine rear
                # depth; there is no single opaque envelope plane or cylinder.
                depth = rng.uniform(9.,21.) if region['id']=='left-bank-climbing-ivy' else rng.uniform(16.,32.)
                for layer in range(3):
                    center=point-RAY*(depth*layer/2)
                    xx=rx*rng.uniform(.85,1.12);yy=ry*rng.uniform(.85,1.12);zz=.25+rng.random()*.65
                    angle=rng.uniform(0,2*math.pi);u=np.array([math.cos(angle),0,0])+UP*math.sin(angle);v=np.array([-math.sin(angle),0,0])+UP*math.cos(angle)
                    points=[center+u*xx,center-u*xx,center+v*yy,center-v*yy,center+RAY*zz,center-RAY*zz]
                    if min(p[2] for p in points)<0: continue
                    offset=len(vertices);vertices.extend([p.tolist() for p in points]);faces.extend([[offset+a,offset+c,offset+b] for a,b,c in [(0,2,4),(2,1,4),(1,3,4),(3,0,4),(2,0,5),(1,2,5),(3,1,5),(0,3,5)]])
                clumps.append({'source_xy':[x,y],'support_method':method,'support_owner_hypothesis':owner,'support_triangle':triangle,'front_center':point.tolist(),'rear_depth':depth})
                pen.ellipse((x-1,y-1,x+1,y+1),fill=(100,235,255))
        for kind, line in region['contacts'].items():
            for x,y in line:
                hit=support(x,y)
                contacts.append({'region':region['id'],'source_xy':[x,y],'contact_kind':kind,'private_bank_receiver':None if hit is None else {'owner':hit[2],'triangle':hit[3],'point':hit[1].tolist()},'authority':'Source2D role only; bank depth/contact is an inferred private-model hypothesis, not measured ground.'})
        points=np.array(vertices);assert len(points)>0
        triangles=points[np.array(faces)].reshape(-1,8,3,3)
        volumes=np.einsum('nfi,nfi->nf',triangles[:,:,0],np.cross(triangles[:,:,1],triangles[:,:,2])).sum(1)/6
        assert np.all(volumes>0), 'Each closed leaf lobe must face outward'
        assert len(vertices)//6==len(faces)//8
        parts.append({'id':region['id'],'vertices':vertices,'faces':faces,'clumps':clumps,'rejected_boundary_clumps':rejected,'bounds':[points.min(0).tolist(),points.max(0).tolist()],'material':'UNASSIGNED_GREEN_FOLIAGE_GEOMETRY_DIAGNOSTIC_ONLY','observed_source_assignments':0})
        pen.line([tuple(p) for p in region['outline']]+[tuple(region['outline'][0])], fill=(255,120,185),width=1)
    report={'status':'PRIVATE_REGIONAL_GREEN_GEOMETRY_PROPOSAL_REQUIRES_SAVED_CONTEXT_REVIEW','pins':pins,'parts':parts,'contacts':contacts,'reserved_openings':proposal['reserved_openings_and_context'],'native_actual_union_preserved':22406,'distribution_revision':'V3 removes the visible lattice: deterministic irregular point spacing, independently rotated thin leaf lobes and exact29000stone admission protection.','limits':['No source pixels assigned: envelopes contain mixed leaves, twigs and background. Exclusions are conservative geometry guidance, not accepted pixel roles.','All parts are inferred authored scenery; no invented gameplay obstacle or catalog ownership.','Bank support uses private52/53/54 CPU geometry only; saved contact with94–97, Tree18 wood and neighbors remains mandatory.','Plant ground profile is inferred from source contacts, not measured depth. Three rear layers are a starting hypothesis requiring oblique review.','Green material remains unassigned; gold Arbre05 donors cannot substitute for green foliage. No generation authorized by this preparation.','Lower-right left-ivy boundary is foreground occlusion, so final hidden completion needs neighbor context before physical closure approval.']}
    out.mkdir();(out/'geometry.json').write_text(json.dumps(report,separators=(',',':'))+'\n');preview.crop((95,320,610,600)).resize((1545,840),Image.Resampling.NEAREST).save(out/'source-support-preview.png')
    print(json.dumps({'parts':[{ 'id':p['id'],'clumps':len(p['clumps']),'faces':len(p['faces'])}for p in parts],'contacts':len(contacts),'bytes':sum(p.stat().st_size for p in out.iterdir())}))


if __name__=='__main__':main()
