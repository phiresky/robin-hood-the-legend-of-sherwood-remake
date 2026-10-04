"""Propose contextual receiver roles for reserved wattle and oak edge pixels."""
import sys
from pathlib import Path
import numpy as np
from PIL import Image
from scipy.ndimage import label
from catalog import OUT
sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'level-editor/refinement/blender'))
from evidence_io import sha,write_json


def main():
    directory=OUT/'mixed-wood-audit/boundary-roles76-93-v1';directory.mkdir(exist_ok=False)
    sourcepath=OUT/'animation-references/composite-frame-0.png';source=Image.open(sourcepath).convert('RGB');records=[]
    for native,wood_ids,crop in [(76,[5,6,8,9,10,12,14,16,17,22,23],(576,802,698,924)),(93,[1,2,3,5,8,9,10,11,12,13],(1346,728,1514,863))]:
        original=OUT/f'mixed-wood-audit/{native}-reserved-wood-edge.png';mask=np.asarray(Image.open(original).convert('L'))>0;components,_=label(mask,np.ones((3,3)));wood=mask&np.isin(components,wood_ids);leaves=mask&~wood;overlay=np.asarray(source).copy()
        for role,array,color in [('wood',wood,(255,0,180)),('foliage',leaves,(0,255,100))]:
            receiver=('wattle99' if native==76 else 'wood35') if role=='wood' else f'foliage{native}'
            path=directory/f'{native}-{receiver}.png';Image.fromarray(array.astype('uint8')*255).save(path);overlay[array]=color
            records.append(dict(native_mask=native,receiver=receiver,pixels=int(array.sum()),mask=str(path),mask_sha256=sha(path),certainty='inferred contextual boundary role; coordinator review pending',reason=('Continuous post/rail silhouette, including mossy antialiased edge; no widened wood over detached flowers.' if native==76 else 'Continuous trunk/branch contour adjoining visible bark; preserve adjacent foliage and uncertain ground-level source separation.') if role=='wood' else 'Detached flower/leaf specks and lower foreground plant edges; separate from existing clear-leaf source domain.',original_reserved_mask=str(original),original_reserved_sha256=sha(original)))
        assert np.array_equal(wood|leaves,mask) and not (wood&leaves).any()
        Image.fromarray(overlay).crop(crop).resize(((crop[2]-crop[0])*5,(crop[3]-crop[1])*5),Image.Resampling.NEAREST).save(directory/f'{native}-roles.png')
    write_json(directory/'proposal.json',dict(status='Source-only inferred proposal; no geometry or clear-leaf domain mutation',source_sha256=sha(sourcepath),records=records,review_basis='Native crop and contiguous post/trunk silhouette; green color alone does not distinguish mossy wood from leaves. Disconnected lower plant tips remain foliage receivers. Existing170/147 reservations preserved as historical evidence.',limitations=['Role inference does not establish physical receiver coverage.','Clear observed502/503 domains remain unchanged; these boundary roles are separate.']))
    print([(r['receiver'],r['pixels']) for r in records])

if __name__=='__main__':main()
