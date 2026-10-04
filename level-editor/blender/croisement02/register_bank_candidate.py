"""Bind the self-reviewed bank candidate for gallery/staging, without approval."""
import json
import sys
from pathlib import Path
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT,reviewed_catalog
from evidence_io import sha,write_json


def main():
    root=OUT/'terrain-bank-candidate';asset='croisement02-north-woodland-bank';worker=root/'assets'/asset
    group=next(g for g in json.loads(reviewed_catalog().read_text())['groups'] if g['id']==asset)
    if sorted(p.get('obstacle',-1) for p in group['parts'])!=list(range(5)):raise ValueError('Bank scope changed')
    review=json.loads((worker/'inspection/visual-review.json').read_text())
    model_hash=sha(worker/'model.blend')
    if not review.get('ready_for_geometry_review') or review['model_sha256']!=model_hash:raise ValueError('Bank self-review is absent or stale')
    for relative,expected in review['reviewed_images'].items():
        if sha(root/relative)!=expected:raise ValueError('Reviewed bank image changed')
    # Gallery notes describe the actual model, never a user approval.
    review['notes']=review['findings']
    write_json(worker/'inspection/visual-review.json',review)
    comparison=worker/'inspection/source-comparison';comparison.mkdir(exist_ok=True)
    crop=(0,0,1505,650)
    source=Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA').crop(crop)
    rendered=Image.open(root/'integration/source-camera.png').convert('RGBA').crop(crop)
    overlay=Image.alpha_composite(source,rendered)
    sheet=Image.new('RGB',(1505,2022),'#777');draw=ImageDraw.Draw(sheet)
    for i,(name,im) in enumerate([('Original source',source),('Exact source camera; unknown surfaces gray',rendered),('Bank geometry over original source',overlay)]):
        draw.text((4,i*674+4),name,fill='white');sheet.paste(im,(0,i*674+24),im.getchannel('A'))
    sheet.save(comparison/'comparison.png')
    write_json(comparison/'report.json',dict(model_sha256=model_hash,comparison_sha256=sha(comparison/'comparison.png'),crop=crop))
    files=[worker/'model.blend',worker/'workspace.json',worker/'source-masks.json',worker/'inspection/visual-review.json',worker/'inspection/refinement.json',worker/'inspection/saved-model-audit.json',worker/'inspection/actual-materials/evidence.json',worker/'inspection/actual-materials/sheet.png',worker/'modified/solid.png',worker/'modified/textured.png',worker/'modified/context.png',worker/'modified/views.json',root/'source-proposal.json',root/'bank-source-domain.png',root/'inventory.json',root/'integration/evidence.json',root/'integration/source-coverage-difference.png',root/'root-contacts/ramp3-evidence.json',root/'root-contacts/ramp3-detail-sheet.png',comparison/'comparison.png',comparison/'report.json']
    files += [root/relative for relative in review['reviewed_images']]
    coverage=json.loads((root/'integration/evidence.json').read_text())
    receipt=dict(source_coverage={key:coverage[key] for key in ['known_bank_pixels','first_hit_bank_pixels','missing_known_pixels']},version=1,asset_id=asset,status='geometry candidate exposed for user review; no approval implied',model_sha256=model_hash,part_ids=[f'building-{i:03}' for i in range(5)],files={str(p.relative_to(root)):sha(p) for p in files},ramp_detail=str(root/'root-contacts/ramp3-detail-sheet.png'),source_difference=str(root/'integration/source-coverage-difference.png'))
    write_json(worker/'inspection/bank-candidate.json',receipt)
    print(worker)


if __name__=='__main__':main()
