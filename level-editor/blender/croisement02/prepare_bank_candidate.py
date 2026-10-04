"""Fresh bank 0–4 terrain candidate with explicit source-space ownership.

The map projection is x = world_x, y = -world_y*sin(35)-world_z*cos(35).
Native ramp planes are retained; unseen north/west continuation is inferred.
"""
import json
import math
import sys
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(Path(__file__).parent))
sys.path.insert(0, str(ROOT/'level-editor/refinement'))
sys.path.insert(0, str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT, reviewed_catalog
from evidence_io import sha, write_json

DEST = OUT/'terrain-bank-candidate'
ASSET = 'croisement02-north-woodland-bank'
WORKER = DEST/'assets'/ASSET
SIN, COS = math.sin(math.radians(35)), math.cos(math.radians(35))


def footprint(index, level, extend=False):
    points = [dict(p) for p in level['sight_obstacles'][index]['points']]
    if index == 0 and extend:
        # Retain the visible west/south corner and northeast ramp junction.
        # Only the unseen north/west border is continued beyond the source.
        z = points[12]['z_top']
        points[12:13] = [dict(x=-120., y=points[11]['y'], z_bottom=0., z_top=z),
            dict(x=-120., y=-120., z_bottom=0., z_top=z),
            dict(x=points[13]['x'], y=-120., z_bottom=0., z_top=z)]
    return points


def mesh_data(points):
    n = len(points)
    vertices = [(p['x'], -p['y']/SIN, p[z]/COS) for z in ('z_bottom','z_top') for p in points]
    faces = [tuple(reversed(range(n))),tuple(range(n,2*n))]
    faces += [(i,(i+1)%n,(i+1)%n+n,i+n) for i in range(n)]
    return vertices,faces


def sculpt_bank_base(points):
    """Keep plateau crests; trace the two exposed escarpment feet from masks."""
    inventory=json.loads((OUT/'review-mask-inventory.json').read_text())['masks']
    bottom=np.full(1792,-1.,dtype=float)
    for row in inventory:
        if row['index'] not in {125,126}:continue
        alpha=np.asarray(Image.open(row['png']).convert('L'))>0
        ox,oy=row['box_top_left']
        for x in range(alpha.shape[1]):
            ys=np.flatnonzero(alpha[:,x])
            if len(ys):bottom[ox+x]=max(bottom[ox+x],oy+int(ys.max())+1)
    expanded=[]
    for a,b in zip(points,points[1:]+points[:1]):
        n=max(1,int(math.ceil(math.hypot(a['x']-b['x'],a['y']-b['y'])/12)))
        for j in range(n):
            t=j/n;expanded.append({key:a[key]*(1-t)+b[key]*t for key in a})
    n=len(expanded);vertices=[]
    for p in expanded:
        x=int(round(p['x']));y=p['y']
        # Only the exposed southern cliff edge can accept this source trace.
        # Distant plateau edges sharing x coordinates must stay untouched.
        if 0<=x<1792 and y-2<=bottom[x]<=y+75:y=max(y,bottom[x])
        vertices.append((p['x'],-y/SIN,p['z_bottom']/COS))
    vertices += [(p['x'],-p['y']/SIN,p['z_top']/COS) for p in expanded]
    faces=[tuple(reversed(range(n))),tuple(range(n,2*n))]
    faces += [(i,(i+1)%n,(i+1)%n+n,i+n) for i in range(n)]
    return vertices,faces


def prepare_domains():
    DEST.mkdir(exist_ok=True)
    level = json.loads((OUT/'baseline/Croisement02.rhp.json').read_text())
    source = OUT/'animation-references/composite-frame-0.png'
    inventory = json.loads((OUT/'review-mask-inventory.json').read_text())
    bank = Image.new('L',(1792,1152))
    draw = ImageDraw.Draw(bank)
    for index in range(5):
        vertices,faces = mesh_data(footprint(index,level))
        for face in faces:
            draw.polygon([(vertices[i][0],-vertices[i][1]*SIN-vertices[i][2]*COS) for i in face],fill=255)
    bank = np.asarray(bank)>0
    excluded = np.zeros_like(bank)
    rows=[]
    # 125/126 are bank escarpment masks; all other covered-state scenery is
    # excluded regardless of whether its final authored geometry exists yet.
    applied_only={138,139,140,141}
    for row in inventory['masks']:
        if row['index'] in {125,126}|applied_only or not row.get('png'):continue
        a=np.asarray(Image.open(row['png']).convert('L'))>0
        x,y=row['box_top_left'];h,w=a.shape
        excluded[max(0,y):min(1152,y+h),max(0,x):min(1792,x+w)] |= a[max(0,-y):min(h,1152-y),max(0,-x):min(w,1792-x)]
        rows.append(dict(index=row['index'],sha256=sha(row['png'])))
    observed_bank = bank.copy()
    for row in inventory['masks']:
        if row['index'] not in {125,126}:continue
        x,y=row['box_top_left'];a=np.asarray(Image.open(row['png']).convert('L'))>0
        h,w=a.shape;observed_bank[y:y+h,x:x+w] |= a
    known=observed_bank & ~excluded
    for name,domain in [('bank-source-domain',known),('bank-projection',bank),('foreground-exclusion',excluded)]:
        Image.fromarray(domain.astype('uint8')*255).save(DEST/f'{name}.png')
    rgb=np.asarray(Image.open(source).convert('RGB'))
    overlay=rgb.astype(float)*.4
    overlay[known]=rgb[known]*.6+np.array([0,220,220])*.4
    overlay[bank & excluded]=rgb[bank & excluded]*.6+np.array([220,20,220])*.4
    Image.fromarray(overlay.astype('uint8')).save(DEST/'source-domain-overlay.png')
    rgba=np.dstack((rgb,known.astype('uint8')*255))
    Image.fromarray(rgba).save(DEST/'source-known.png')
    inventory['masks'].append(dict(index=420,layer=0,png=str(DEST/'bank-source-domain.png'),box_top_left=[0,0],box_size=[1792,1152],provenance='Bank0–4 projected native surfaces minus covered-state foreground; candidate receiver domain.'))
    write_json(DEST/'inventory.json',inventory)
    write_json(DEST/'source-masks.json',dict(version=1,mask_inventory=str(DEST/'inventory.json'),projections=dict(exterior=dict(state='Initial covered source state; applied-only scenery excluded.',source_sha256=sha(source),assignments=[dict(asset_group=ASSET,mask_indices=[420],reviewed=True)]))))
    # Freeze a native-only catalog for the native-only source scene. Authored
    # vegetation has a separate staging lane and remains an explicit exclusion.
    catalog=json.loads(reviewed_catalog().read_text())
    groups=[]
    for group in catalog['groups']:
        group=dict(group);group['parts']=[p for p in group['parts'] if 'obstacle' in p]
        if group['parts']:groups.append(group)
    catalog['groups']=groups
    if 'canonical_owners' in catalog:
        catalog['canonical_owners']={k:v for k,v in catalog['canonical_owners'].items() if k.startswith('building-')}
    assert sorted(p['obstacle'] for g in groups for p in g['parts'])==list(range(150))
    assert sorted(p['obstacle'] for g in groups if g['id']==ASSET for p in g['parts'])==list(range(5))
    write_json(DEST/'catalog.json',catalog)
    write_json(DEST/'grouping-review.json',dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(DEST/'catalog.json'),inventory_sha256=sha(OUT/'forest-v4-inventory/inventory.json'),evidence=['Current native ownership retained; authored nodes omitted from this frozen native-only scene. Bank0–4 only;37 belongs to westrock.']))
    write_json(DEST/'source-proposal.json',dict(status='candidate; visual review pending',source_sha256=sha(source),source_domain_sha256=sha(DEST/'bank-source-domain.png'),native_level_sha256=sha(OUT/'baseline/Croisement02.rhp.json'),foreground_masks=rows,known_pixels=int(known.sum()),bank_projection_pixels=int(bank.sum()),retained_bank_masks=[125,126],limitations=['This is a bank receiver candidate, not a global ground-domain approval.','Native foliage silhouettes gate the source; their authored geometry still needs joint review.','New north/west return is outside source and remains unknown.']))


def build():
    import bpy
    from tree_geometry import replace_mesh
    from refinement_workspace import prepare, modified
    from render_slots import acquire,release
    from audit_candidates import audit
    from render_tree import render_workspace
    if WORKER.exists():raise FileExistsError(WORKER)
    prepare_domains()
    level=json.loads((OUT/'baseline/Croisement02.rhp.json').read_text())
    catalog=json.loads((DEST/'catalog.json').read_text())
    owners={f"building-{p['obstacle']:03}":g for g in catalog['groups'] for p in g['parts']}
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(OUT/'forest-v4-input.blend'))
        bpy.context.preferences.filepaths.save_version=0
        collection=bpy.data.collections['Croisement02 Working']
        for obj in collection.all_objects:
            if obj.type=='MESH' and obj.get('source_node') in owners:
                g=owners[obj['source_node']];obj['asset_group']=g['id'];obj['asset_name']=g['name']
        prepare(WORKER,asset_id=ASSET,scene_name='Croisement02 Refinement',collection_name=collection.name,
            source_path=OUT/'animation-references/composite-frame-0.png',grouping_manifest=DEST/'catalog.json',
            inventory_path=OUT/'forest-v4-inventory/inventory.json',review_path=DEST/'grouping-review.json',
            source_mask_manifest=DEST/'source-masks.json',width=640,height=384,framing_padding=1.12,
            lighting=dict(toward_sun=[-.45,-.55,.70],ambient=.22,diffuse=.78,shadow_epsilon=.05))
        parts=[]
        for obj in collection.all_objects:
            if obj.type!='MESH' or obj.get('asset_group')!=ASSET:continue
            index=int(obj['source_node'].split('-')[-1])
            points=footprint(index,level,extend=True)
            vertices,faces=sculpt_bank_base(points) if index==0 else mesh_data(points)
            result=replace_mesh(obj,vertices,faces,materials=list(obj.data.materials))
            if result['nonmanifold_edges']:raise ValueError(result)
            result.update(source_node=obj['source_node'],native_top_heights=[p['z_top'] for p in footprint(index,level)],inferred_boundary_extension=index==0,escarpment_base_source_trace=index==0)
            parts.append(result)
        modified(WORKER)
        (WORKER/'inspection').mkdir(exist_ok=True)
        write_json(WORKER/'inspection/refinement.json',dict(asset_id=ASSET,model_sha256=sha(WORKER/'model.blend'),parts=parts,source_packet=str(DEST/'source-known.png'),status='private terrain geometry candidate; review pending',limitations=['Native ramp planes retained; cliff faces still require source/oblique review.','Unknown north/west returns extend120map units; they are not observed evidence.','Ground below bank remains contextual and is not approved for publication.']))
        audit(WORKER)
        render_workspace(WORKER,512,release_slot=False)
    finally:release()


if __name__=='__main__':
    if '--domains-only' in sys.argv:prepare_domains()
    else:build()
