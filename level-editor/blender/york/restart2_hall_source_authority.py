"""Freeze reviewed York hall artwork domains with state and component ownership."""
import hashlib
import json
from pathlib import Path
from PIL import Image,ImageChops,ImageDraw,ImageFilter

ROOT=Path(__file__).resolve().parents[3]
BASE=ROOT/'level-editor/work/york-refinement'
DEST=BASE/'restart2/hall-source-authority-v1'
STUDY=BASE/'restart2/hall-source-authority-study-v1'
if DEST.exists():raise FileExistsError(DEST)
sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest()
catalog_path=ROOT/'level-editor/refinement/catalogs/york.json'
catalog=json.loads(catalog_path.read_text())
for node in (790,810,830,831):
    if catalog['canonical_owners'][f'building-{node}']!='york-castle-main-keep':raise ValueError('Doorway owner changed')
inventory=BASE/'baseline/masks/manifest.json'
native_inventory=json.loads(inventory.read_text())['masks']
source0=BASE/'baseline/revealed.png';size=Image.open(source0).size
inputs={inventory,source0,catalog_path};cache={}
def native(index):
    if index not in cache:
        row=next(r for r in native_inventory if r['index']==index);p=inventory.parent/row['png'];inputs.add(p)
        im=Image.new('L',size);im.paste(Image.open(p).convert('L'),tuple(row['box_top_left']));cache[index]=im
    return cache[index]
def polygon(points):
    im=Image.new('L',size);ImageDraw.Draw(im).polygon(points,fill=255);return im
def union(*masks):
    result=Image.new('L',size)
    for mask in masks:result=ImageChops.lighter(result,mask)
    return result
def intersect(a,b):return ImageChops.multiply(a,b)
def subtract(a,b):return ImageChops.subtract(a,b)
floor_points=[(2756,594),(2855,569),(2942,660),(2942,684),(2922,693),(2896,702),(2861,711),(2747,607)]
body_points=[(2745,580),(2908,580),(2908,705),(2906,835),(2745,835)]
roof_points=[(2734,553),(2857,457),(2992,604),(2871,700)]
upper_points=[(2738,553),(2872,697),(2918,686),(2918,709),(2861,717),(2745,610)]
body=union(subtract(intersect(native(641),polygon(body_points)),native(597)),native(647))
rear=subtract(intersect(native(643),polygon([(2745,450),(2942,450),(2942,720),(2745,720)])),native(650))
furniture={824:631,825:634,826:633,827:640,828:632,829:636}
furniture_native=union(*(native(i) for i in list(furniture.values())+[635]))
base=union(body,rear,subtract(native(650),native(656)),furniture_native,
           polygon([(2756,594),(2825,574),(2896,629),(2942,677),(2942,684),(2922,693),(2896,702),(2861,711),(2747,607)]))
alphas={}
for patch,origin in [('001',(2544,324)),('002',(2751,530))]:
    path=BASE/f'geometry-pass-01/native-state-source-v1/patch-{patch}/row-0/frame-000.png';inputs.add(path)
    alpha=Image.new('L',size);alpha.paste(Image.open(path).convert('RGBA').getchannel('A'),origin);alphas[patch]=alpha
outer=[(2761,598),(2761,555),(2764,545),(2770,537),(2777,531),(2784,529),(2791,529),(2797,534),(2797,586)]
inner=[(2767,592),(2767,555),(2770,549),(2775,544),(2780,541),(2785,539),(2789,540),(2791,546),(2791,585)]
arch=union(native(646),polygon(outer+list(reversed(inner))))
crop=(2710,440,3060,840);entries=[];projections={};reviews=[]
DEST.mkdir()
def emit(stem,role,trace):
    full=trace.point(lambda v:255 if v else 0);known=full.filter(ImageFilter.MinFilter(3));uncertain=subtract(full,known)
    for suffix,im in [('',known),('-full-trace',full),('-uncertain-boundary',uncertain)]:im.crop(crop).save(DEST/f'{stem}-{role}{suffix}.png')
    index=len(entries);entries.append({'index':index,'kind':'authored-artwork-domain','box_top_left':list(crop[:2]),
        'box_size':[crop[2]-crop[0],crop[3]-crop[1]],'png':f'{stem}-{role}.png'})
    return index,sum(v>0 for v in full.crop(crop).getdata()),sum(v>0 for v in known.crop(crop).getdata())
for a,c in [('initial','initial'),('initial','applied'),('applied','initial'),('applied','applied')]:
    stem=f'{a}-{c}';domain=base.copy()
    if a=='initial':domain=union(domain,intersect(native(625),polygon(roof_points)))
    if c=='initial':domain=subtract(union(domain,intersect(native(660),polygon(upper_points))),union(native(663),native(664)))
    else:domain=subtract(domain,native(656))
    domain=subtract(domain,native(597));domain=union(domain,polygon(floor_points))
    if c=='initial':
        domain=subtract(domain,union(native(663),native(664)))
        domain=union(domain,subtract(intersect(alphas['002'],polygon(roof_points)),union(native(663),native(664))))
    proposal=STUDY/f'{stem}-domain-proposal-v{5 if c=="initial" else 4}.png';inputs.add(proposal)
    if ImageChops.difference(domain,Image.open(proposal).convert('L')).getbbox():raise ValueError('Reviewed trace reproduction differs: '+stem)
    component_domains={}
    for number,index in furniture.items():component_domains[f'building-{number}']=intersect(native(index),domain)
    component_domains['scenery-york-great-hall-candle-stand']=intersect(native(635),domain)
    component_domains['scenery-york-great-hall-northwest-arch']=intersect(arch,domain)
    for node in component_domains:
        if c=='initial':component_domains[node]=Image.new('L',size)
        elif a=='initial':component_domains[node]=subtract(component_domains[node],alphas['001'])
    shell=subtract(domain,union(*component_domains.values()))
    assignments=[];roles=[]
    index,full_count,known_count=emit(stem,'shell',shell)
    shell_nodes=[f'building-{n}' for n in (769,791,793,795,799)]
    if c=='initial':shell_nodes.append('scenery-york-great-hall-upper-front-wall')
    for node in shell_nodes:assignments.append({'reviewed':True,'source_node':node,'mask_indices':[index],
        'review_evidence':'Root-reviewed hall domain, native furniture and explicit arch ring excluded. Layer-aware component visibility still required.'})
    roles.append({'role':'shell','full_trace_pixels':full_count,'confident_pixels':known_count,'source_nodes':shell_nodes})
    for node,trace in component_domains.items():
        index,full_count,known_count=emit(stem,node,trace)
        assignments.append({'reviewed':True,'source_node':node,'mask_indices':[index],
            'review_evidence':'Reviewed native furniture silhouette or source-traced stone arch ring; covered-state source pixels excluded.'})
        roles.append({'role':node,'full_trace_pixels':full_count,'confident_pixels':known_count})
    source=BASE/f'restart2/hall-cover-source-combinations-v1/patch001-{a}_patch002-{c}.png';inputs.add(source)
    projections['hall-semantic-'+stem]={'source_sha256':sha(source),'state':f'001{a}/002{c}', 'assignments':assignments}
    domain.crop(crop).save(DEST/f'{stem}-group-full-trace.png')
    overlay=Image.open(source).convert('RGB');overlay=Image.composite(Image.blend(overlay,Image.new('RGB',size,(30,220,100)),.4),overlay,domain)
    overlay.crop(crop).resize((1050,1200),Image.Resampling.NEAREST).save(DEST/f'{stem}-source-domain-overlay.png')
    reviews.append({'state':stem,'roles':roles,'proposal_sha256':sha(proposal)})
(DEST/'inventory.json').write_text(json.dumps({'version':1,'index_namespace':'Local authored domain indices, not native mask IDs','masks':entries},indent=2)+'\n')
(DEST/'source-masks.json').write_text(json.dumps({'version':1,'mask_inventory':str(DEST/'inventory.json'),'projections':projections},indent=2)+'\n')
(DEST/'review.json').write_text(json.dumps({'status':'Root group-domain PASS; component-constrained immutable packet preparation authorized, actual known-source coverage validation pending',
    'method':'Source artwork/native masks and explicit traced boundaries; no candidate visibility defines source authority',
    'rules':{'body_polygon':body_points,'floor_polygon':floor_points,'roof_envelope':roof_points,'upper_front_polygon':upper_points,
             'arch_outer':outer,'arch_inner':inner,'open_roof':'650 minus656','closed_tower_exclusion':[663,664],
             'foreground_turret_exclusion':597,'initial001_roof':'625 intersect roof envelope','initial002_roof':'Decoded RGBA alpha intersect roof envelope',
             'component_scope':'Visible furniture/candle native masks and arch ring reserved from shell; initial002 hides their source authority; initial001 alpha removes occluded component pixels',
             'uncertainty':'One-pixel erosion per component; full trace and uncertain ring retained separately'},
    'doorway_ownership':{'canonical_owner':'york-castle-main-keep','source_nodes':['building-831','building-790','building-810','building-830'],
                         'excluded':'Orange closed northwest door and dark open doorway backing are keep appearance, not omitted hall texture'},
    'input_hashes':{str(p.relative_to(ROOT)):sha(p) for p in sorted(inputs)},'states':reviews,
    'geometry_unchanged':True,'geometry_approval':'Batch-v3, separate from texture approval','texture_approval':'none'},indent=2)+'\n')
print(DEST)
