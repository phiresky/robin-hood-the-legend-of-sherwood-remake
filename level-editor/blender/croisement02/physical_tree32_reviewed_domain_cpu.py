"""Construct a private Tree32 volume using the reviewed nine-pixel domain.

The receipt excludes nine nonwood pixels only. It grants no geometry approval.
"""
import hashlib
import json
from pathlib import Path
import numpy as np
from PIL import Image
import physical_wood_sections_cpu as constructor

BASE=Path(__file__).resolve().parents[3]/'level-editor/work/croisement02-refinement/wood-sweep-cpu-review/tree32-nine-pixel-domain-proposal'
REVIEW_SHA='9d5890d66c7f9f67e3f39e93740072cf47adbbd2228b398a4d640fe9bb7949ba'
def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()
def reviewed_source(index,box):
    if index!=32:raise ValueError('Reviewed domain applies only to Tree32')
    review_path=BASE/'root-review.json'
    if sha(review_path)!=REVIEW_SHA:raise ValueError('Domain receipt changed')
    review=json.loads(review_path.read_text());proposal_path=BASE/'proposal.json'
    if sha(proposal_path)!=review['proposal_sha256']:raise ValueError('Domain proposal changed')
    proposal=json.loads(proposal_path.read_text());path=Path(proposal['proposed_mask'])
    if sha(path)!=proposal['proposed_mask_sha256'] or sha(Path(proposal['first_hit_evidence']))!=proposal['first_hit_evidence_sha256']:raise ValueError('Domain proof changed')
    bitmap=np.asarray(Image.open(path).convert('L'))>0;ox,oy=review['mask_offset'];x0,y0,x1,y1=box
    if not(ox<=x0<x1<=ox+bitmap.shape[1] and oy<=y0<y1<=oy+bitmap.shape[0]):raise ValueError('Requested box extends beyond reviewed source domain')
    return bitmap[y0-oy:y1-oy,x0-ox:x1-ox],path
if __name__=='__main__':
    constructor.source=reviewed_source
    constructor.main()
