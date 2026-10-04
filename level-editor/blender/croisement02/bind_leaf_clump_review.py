"""Bind an explicitly self-reviewed native clump and its unchanged neighbourhood packet."""
import argparse,json,sys
from pathlib import Path
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from evidence_io import sha,write_json

SPECS={77:(16,'south77-v2'),78:(12,'southwest-small-v1'),83:(18,'southwest-thicket83-v1'),84:(12,'southwest-small-v1'),74:(15,'east-south-clumps-v1'),85:(15,'east-south-clumps-v1'),86:(15,'east-south-clumps-v1'),87:(20,'east87-v2'),88:(15,'east-south-clumps-v1'),89:(17,'south-boundary89-v1'),90:(15,'east-south-clumps-v1'),93:(22,'oak-base93-v2')}

def main(index,joint):
    round_number,proposal=SPECS[index];worker=OUT/f'understory-round-{round_number}/assets/croisement02-shrub-{index}';inspection=worker/'inspection';mh=sha(worker/'model.blend');source=OUT/'understory-candidates'/proposal;packet_folder=source/f'shrub-{index}'
    evidence=json.loads((joint/'evidence.json').read_text());rows=evidence['workers']
    if not any(Path(r['path'])==worker and r['model_sha256']==mh for r in rows):raise ValueError('Joint does not bind this exact worker')
    for row in rows:
        if sha(Path(row['path'])/'model.blend')!=row['model_sha256']:raise ValueError('Joint neighbour changed')
    for name in ('saved-model-audit.json','source-coverage/report.json','actual-materials/opacity-bounds.json'):
        proof=json.loads((inspection/name).read_text())
        if proof['model_sha256']!=mh:raise ValueError('Current worker proof missing')
    support_paths=sorted(packet_folder.glob('*/support.json')) or [packet_folder/'support.json']
    support=dict(model_sha256=mh,records=[dict(path=str(p),sha256=sha(p),report=json.loads(p.read_text())) for p in support_paths],status='Leaf-volume support hypothesis checked in exact neighbourhood; no observed root claim')
    write_json(inspection/'support-evidence.json',support)
    comparison=joint/'source-comparison.png'
    if not comparison.exists():
        board=Image.new('RGB',(1920,664),'#454545');draw=ImageDraw.Draw(board)
        for col,(name,title) in enumerate([('native-source.png','Native artwork; black is beyond map'),('source-plants.png','Isolated clump at exact native camera'),('source-overlay.png','Native artwork plus clump continuation')]):
            im=Image.open(joint/name).convert('RGBA');board.paste(im,(col*640,24),im);draw.text((col*640+5,5),title,fill='white')
        board.save(comparison)
    receipt=dict(model_sha256=mh,evidence=str(joint/'evidence.json'),evidence_sha256=sha(joint/'evidence.json'),sheet=str(joint/'sheet.png'),sheet_sha256=sha(joint/'sheet.png'),source_comparison=str(comparison),source_comparison_sha256=sha(comparison),label='Exact native source and unchanged scenery neighbours; eight obliques with diagnostic ground datum')
    if index in (85,93):receipt['label']='Exact native source and private corrected tree35 lower geometry with unchanged scenery; legacy crown separately held'
    write_json(inspection/'joint-neighbourhood.json',receipt)
    preservation=source/'source-rgb-validation.json'
    review=dict(reviewer='Codex',status='Native source, all eight isolated views and exact neighbourhood views manually reviewed; new user review pending',ready_for_geometry_review=True,model_sha256=mh,sheet_sha256=sha(inspection/'actual-materials/sheet.png'),joint_neighbourhood_sha256=sha(inspection/'joint-neighbourhood.json'),support_evidence_sha256=sha(inspection/'support-evidence.json'),preservation_evidence=str(preservation),preservation_evidence_sha256=sha(preservation),limitations=['Hidden volume and any beyond-map continuation are inferred from the same native foliage.','Diagnostic ground plane is a contact aid; complete terrain integration remains separate.','Existing neighbouring texture/geometry artifacts are preserved and not approved by this review.','No user approval, final texture approval or publication implied.'])
    if index==93:
        budget=inspection/'render-budget-evidence.json'
        report=json.loads(budget.read_text())
        if report['model_sha256']!=mh or evidence['transparent_bounces']<256:raise ValueError('Dense foliage render budget evidence changed')
        review['render_budget_evidence_sha256']=sha(budget)
    if index in (85,93):review['limitations'].append('Private corrected tree35 lower geometry is context only; its legacy crown remains separately held.')
    fill=inspection/'inferred-fill-evidence.json'
    if fill.exists():review['inferred_fill_evidence_sha256']=sha(fill)
    write_json(inspection/'visual-review.json',review);print(worker)

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('index',type=int,choices=sorted(SPECS));parser.add_argument('joint',type=Path);args=parser.parse_args();main(args.index,args.joint.resolve())
