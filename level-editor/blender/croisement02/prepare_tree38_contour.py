"""Reproduce the private reviewed tree38 contour packet without canonical edits."""
import argparse,json,sys
from pathlib import Path
import numpy as np
from PIL import Image
sys.path[:0]=[str(Path(__file__).parent),str(Path(__file__).resolve().parents[2]/'refinement/blender')]
from catalog import OUT,tree_workspace
from evidence_io import sha,write_json

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--output',type=Path,required=True);args=parser.parse_args()
    output=args.output.resolve();output.mkdir(parents=True,exist_ok=False)
    old=tree_workspace(38);proposal=OUT/'understory-candidates/mixed75-91-boundary-review/75-proposed-wood38.png'
    if sha(proposal)!='441379b075b1b5f6b96bfbcbf8a352b20d2b276afe7e5915d16628541a3ebbdb':raise ValueError('Reviewed proposal changed')
    pixels=np.asarray(Image.open(proposal).convert('L'))>0;y,x=np.indices(pixels.shape)
    uncertain=pixels&(y>=711)&(x<=1573);accepted=pixels&~uncertain
    if int(accepted.sum())!=344 or int(uncertain.sum())!=65:raise ValueError('Reviewed source partition changed')
    contour=output/'trunk-basal-contour.png';Image.fromarray(accepted.astype('uint8')*255).save(contour)
    Image.fromarray(uncertain.astype('uint8')*255).save(output/'distal-root-inference.png')
    manifest=json.loads((old/'source-masks.json').read_text());inventory=json.loads(Path(manifest['mask_inventory']).read_text())
    if any(r['index']==3800 for r in inventory['masks']):raise ValueError('Private domain3800 already allocated')
    row=next(r for r in inventory['masks'] if r['index']==38);native=Image.new('L',(1792,1152));native.paste(Image.open(row['png']).convert('L'),tuple(row['box_top_left']))
    Image.fromarray(((np.asarray(native)>0)|accepted).astype('uint8')*255).save(output/'wood38-plus-reviewed-contour.png')
    inventory['masks'].append(dict(index=3800,layer=0,png=str(contour),box_top_left=[0,0],box_size=[1792,1152],provenance='Private344-pixel basal contour subset;65 uncertain distal pixels withheld.'))
    write_json(output/'inventory.json',inventory);manifest['mask_inventory']=str(output/'inventory.json')
    assignments=manifest['projections']['exterior']['assignments'];owner=next(r for r in assignments if r.get('asset_group')==old.name)
    if owner['mask_indices']!=[38]:raise ValueError('Unexpected native wood assignment')
    owner['mask_indices']=[38,3800];write_json(output/'source-masks.json',manifest)
    write_json(output/'source-review.json',dict(status='Private source packet; no canonical mutation',proposal=str(proposal),proposal_sha256=sha(proposal),authored_pixels=344,uncertain_distal_pixels_withheld=65,authored_domain3800_sha256=sha(contour),source_packet_sha256=sha(output/'source-masks.json')))
if __name__=='__main__':main()
