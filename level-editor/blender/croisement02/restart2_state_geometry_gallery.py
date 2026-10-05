"""Package independently reviewed endpoint geometry without approving appearance or motion."""
import json,hashlib,sys,math
from pathlib import Path
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from build_review_gallery import build

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def native_first_sheet(base,dest,state,mode,net=False):
    """Keep native -Y/35 degree projection first without touching frozen renders."""
    order=[4,5,6,7,0,1,2,3]
    manifest=json.loads((base/'manifest.json').read_text())
    records=manifest['renders' if net else 'records']
    if not net:
        cameras=[r['camera'] for r in records if r['state']==state and r['mode']==mode]
        center=[sum(c[k] for c in cameras)/8 for k in range(3)]
        native=next(r['camera'] for r in records if r['state']==state and r['mode']==mode and r['view']==4)
        assert abs(native[0]-center[0])<.001 and native[1]<center[1]
        # All orbit cameras have a shared elevation; their horizontal radius fixes height.
        radius=math.hypot(native[0]-center[0],native[1]-center[1]);assert abs(radius-3000*math.cos(math.radians(35)))<.002
    sheet=Image.new('RGB',(2048,1088),(40,40,40));draw=ImageDraw.Draw(sheet);inputs=[]
    for tile,view in enumerate(order):
        name=f'{view:02}-{mode}.png' if net else f'{state}-{view:02}-{mode}.png'
        path=base/name;record=next(r for r in records if r['image']==name)
        assert sha(path)==record['sha256']
        image=Image.open(path).convert('RGBA');assert image.size==(512,512)
        x,y=(tile%4)*512,(tile//4)*544
        sheet.paste(image,(x,y+32),image)
        label='Original game camera / art view (35 deg)' if tile==0 else f'Orbit {tile}: native azimuth +{tile*45} deg'
        draw.text((x+8,y+8),label,fill='white');inputs.append(dict(tile=tile,original_view=view,path=str(path),sha256=sha(path)))
    result=dest/f'{state}-{mode}-native-first.png';sheet.save(result)
    return result,dict(output=str(result),sha256=sha(result),inputs=inputs,native_camera_direction=[0,-0.8191520442889918,0.573576436351046],render_recipe=str(Path(__file__).parent/('restart2_net_attached_endpoint.py' if net else ('render_log_endpoint_orbit.py' if 'log-trap' in str(base) else 'render_rock_endpoint_orbit.py'))))

def main():
    dest=OUT/'restart2-state/scoped-geometry-review-v2';dest.mkdir(exist_ok=True);root_reviews=OUT/'restart2-state/root-endpoint-reviews-v3.json';reviews=json.loads(root_reviews.read_text())['reviews'];items=[];camera_records=[]
    limits={'log-trap':'Approve the complete initial and fallen log endpoint geometry only. Gray ends/backs and source texture distortion still require appearance work. Native planar motion is separately verified; full 3D body correspondence, sound, receiver transitions and editor integration remain incomplete.','rock-trap':'Approve the two initial and five applied rock bodies only. Thin inferred rim and rear texture fill remain appearance work. Bounded outline shrink tests lost known native samples, so the reviewed geometry was retained. Temporal body identity, shadow receivers and editor integration remain incomplete.'}
    for assembly,terminal in [('log-trap',89),('rock-trap',104)]:
        base=OUT/f'{assembly}-state-candidate-v14';model=base/'worker.blend';review=next(r for r in reviews if r['assembly']==assembly);assert sha(model)==review['model_sha256'];endpoints=[]
        for state,tick in [('covered',-1),('applied',terminal)]:
            actual,proof=native_first_sheet(base/'endpoint-orbit',dest,state,'actual');camera_records.append(proof)
            solid,proof=native_first_sheet(base/'endpoint-orbit',dest,state,'solid');camera_records.append(proof)
            # Keep each assembly's immutable generated sheet distinct.
            actual_target=dest/f'{assembly}-{state}-actual-native-first.png';actual.rename(actual_target)
            solid_target=dest/f'{assembly}-{state}-solid-native-first.png';solid.rename(solid_target)
            camera_records[-2]['output']=str(actual_target);camera_records[-1]['output']=str(solid_target)
            endpoints.append(dict(id=state,status='Independently reviewed geometry; exact user approval pending',model_sha256=sha(model),solid=str(solid_target),textured=str(actual_target),context=str(OUT/f'state-target-evidence/{assembly}/tick-{tick:03}-context.png'),review=str(root_reviews),validation=str(base/(('covered-contact-audit.json' if state=='covered' else 'dense-contact-audit.json') if assembly=='log-trap' else 'contact-audit.json')),ownership=str(base/'manifest.json')))
        supplemental=[]
        if assembly=='rock-trap':
            for mode in ['actual','solid']:
                sheet=Image.new('RGB',(1536,542),(40,40,40));draw=ImageDraw.Draw(sheet)
                for j,view in enumerate([4,6,7]):
                    image=Image.open(base/(f'endpoint-orbit/applied-{view:02}-{mode}.png' if view==4 else f'endpoint-isolated-supplement/applied-{view:02}-{mode}.png')).convert('RGBA');assert image.size==(512,512);sheet.paste(image,(j*512,30),image);draw.text((j*512+8,8),('Original game camera / art view' if view==4 else f'Applied isolated view {view}: {mode}'),fill='white')
                sheet.save(dest/f'rock-applied-isolated-{mode}.png')
            supplemental=[dict(id='applied-isolated',name='Applied rocks without the bank hiding views 6 and 7',description='Supplemental views of the same exact applied bodies; no changed geometry.',solid=str(dest/'rock-applied-isolated-solid.png'),textured=str(dest/'rock-applied-isolated-actual.png'),context=str(OUT/'state-target-evidence/rock-trap/tick-104-context.png'))]
        items.append(dict(id=f'croisement02-{assembly}-endpoints',name=assembly.replace('-',' ').title()+' — initial and applied geometry',status='ready-for-user',technical_eligible=True,model=str(model),endpoint_reviews=endpoints,animation_reviews=supplemental,notes=limits[assembly],approval_scope='Only the exact endpoint geometry shown; not textures, temporal identity or integrated state completion.'))
    base=OUT/'restart2-state/net-attached-v1';model=base/'worker.blend';assert sha(model)==next(r for r in reviews if r['assembly']=='net-piege01-occupied-final-0')['model_sha256']
    net_actual,proof=native_first_sheet(base,dest,'net','actual',net=True);camera_records.append(proof)
    net_solid,proof=native_first_sheet(base,dest,'net','solid',net=True);camera_records.append(proof)
    items.append(dict(id='croisement02-net-piege01-occupied-endpoint',name='Occupied net — attached final pose 0',status='ready-for-user',technical_eligible=True,model=str(model),solid=str(net_solid),textured=str(net_actual),context=str(base/'native-body-context-phase0.png'),source_comparison=str(base/'joint-wood-only-v2/reverse.png'),source_comparison_label='Corrected reverse view with physical tree attachment',validation=str(base/'reopened-audit.json'),ownership=str(base/'wood-contact-audit-v2.json'),review=str(root_reviews),notes='Approve this attached occupied-net geometry at final pose 0 only. Six complete solids include an inferred local cloth indentation around the counterweight and two short branch ties. 109 native samples, three gray first hits, rear texture fill, other empty/occupied families and phases, mission activation and integration remain incomplete. This does not approve all net states.'))
    base=OUT/'fence-state-candidate-v2';model=base/'worker.blend';assert sha(model)==next(r for r in reviews if r['assembly']=='scoped-fence-clearing')['model_sha256']
    items.append(dict(id='croisement02-south-field-wattle-fence-cleared-state',name='South field wattle fence — cleared gap geometry',status='ready-for-user',technical_eligible=True,model=str(model),solid=str(base/'geometry-review/solid.png'),textured=str(base/'geometry-review/textured.png'),context=str(base/'source-comparison.png'),source_comparison=str(base/'comparison.png'),source_comparison_label='Covered geometry reference above; scoped cleared geometry below',review=str(root_reviews),ownership=str(base/'validation.json'),validation=str(base/'reopened-topology.json'),notes='Approve only the surviving fence runs and capped ends after the scoped gap clears. Unrelated fence geometry is retained. The exact terminal ground patch, transition timing, texture completion and exported playback remain separate.'))
    fence_views=json.loads((base/'geometry-review/views.json').read_text());assert fence_views['views'][0]['azimuth_degrees']==0 and fence_views['elevation_degrees']==35
    camera_records.append(dict(asset='cleared fence',status='Already native camera first; preserved unchanged',views_sha256=sha(base/'geometry-review/views.json'),first_view=fence_views['views'][0]))
    (dest/'camera-contract-verification.json').write_text(json.dumps(dict(status='PASS',scope='All four cards, both endpoints and actual/solid sheets; native camera top-left',native_projection='Orthographic, direction (0,-cos35,sin35), image x world +X, image up (0,sin35,cos35)',prior_gallery_preserved='scoped-geometry-review-v1',model_geometry_unchanged=True,records=camera_records),indent=2)+'\n')
    manifest=dict(map='Croisement02 scoped state geometry',review_kind='geometry',items=items,status_counts={'exact endpoint geometry awaiting user decision':4},scope='Top-left is always the original game camera / art view (35 degrees), identically ordered in solid and textured sheets. A decision applies only to the exact geometry hashes and endpoint(s) displayed on its card. Appearance, motion, receiver transitions and editor integration remain separate.',signs='Existing reusable sign geometry and standalone export are separately reviewed; sign full-scene presentation remains under reconciliation. No rejected wheel placement is included.',root_review_sha256=sha(root_reviews),texture_inputs_status='Source projection coverage and valid known/unknown ownership packets must be retained before API generation. Private preparation does not imply synthesis authorization.')
    path=dest/'review-candidates.json';path.write_text(json.dumps(manifest,indent=2)+'\n');build(path,dest/'gallery')
    html_path=dest/'gallery/index.html';html=html_path.read_text();sign=OUT/'state-sign-candidate/gallery/index.html';link='<p>Separate sign reference: <a href="/@fs/'+str(sign)+'">rotating sign geometry and standalone playback</a>. Full-scene sign ordering remains separate; this link does not extend the four endpoint decisions above.</p>';html=html.replace('</main>',link+'</main>') if '</main>'in html else html.replace('</body>',link+'</body>');html_path.write_text(html)
    evidence=json.loads((dest/'gallery/evidence.json').read_text());print('Built',len(items),'exact geometry cards at',dest/'gallery/index.html')
if __name__=='__main__':main()
