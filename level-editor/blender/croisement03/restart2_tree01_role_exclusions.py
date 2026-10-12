"""Inventory ambiguous foliage and phase-specific bark occlusion without assigning owners."""
import hashlib
import json
from collections import deque
from pathlib import Path
import numpy as np
from PIL import Image
R=Path(__file__).resolve().parents[3]
B=R/'level-editor/work/croisement03-refinement/restart2'
O=B/'tree01-role-exclusions-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
    assert not O.exists(),O
    O.mkdir()
    pins={}
    def mask(p,rgba=False):
        pins[str(p)]=sha(p)
        a=np.array(Image.open(p).convert('RGBA' if rgba else 'L'))
        return a[:,:,3]>0 if rgba else a>0
    static=mask(B/'tree01-static-leaf-proposal-v1/proposed-source-rgba.png',True)
    occupancy=mask(B.parent/'baseline/masks/000124.png')[:175,:175]
    phases=[mask(B/'tree01-native-fragment-v1'/f'{i:03}.png',True) for i in range(14)]
    union=np.logical_or.reduce(phases)
    yy,xx=np.indices(static.shape)
    rows=[];seen=np.zeros_like(static)
    for y,x in zip(*np.nonzero(static)):
        if seen[y,x]:continue
        q=deque([(int(y),int(x))]);seen[y,x]=True;points=[]
        while q:
            cy,cx=q.popleft();points.append((cy,cx))
            for dy in (-1,0,1):
                for dx in (-1,0,1):
                    ny,nx=cy+dy,cx+dx
                    if 0<=ny<175 and 0<=nx<175 and static[ny,nx] and not seen[ny,nx]:
                        seen[ny,nx]=True;q.append((ny,nx))
        ys,xs=np.array(points).T
        inside=int(occupancy[ys,xs].sum())
        rows.append({'cluster':len(rows),'pixels':len(points),'bbox':[int(xs.min()),int(ys.min()),int(xs.max()+1),int(ys.max()+1)],'inside_occupancy_proposal':inside,'outside_occupancy_excluded':len(points)-inside,'ownership':'UNASSIGNED','pixels_xy':[[x,y]for y,x in points]})
    assert int((static&occupancy).sum())==4254
    assert int((static&~occupancy).sum())==1777
    old=mask(B/'tree01-bark-seed-proposal-v1/proposed-visible-bark.png')
    upper=mask(B/'tree01-upper-bark-proposal-v1/proposed-upper-bark.png')
    hitpath=B/'tree01-upper-bark-proposal-v1/cpu-firsthits.json';pins[str(hitpath)]=sha(hitpath)
    misses={tuple(f['source']) for row in json.loads(hitpath.read_text())['checks'] for f in row['failures']}
    for x,y in misses:upper[y,x]=False
    assert int(upper.sum())==98
    bark=old|upper;unionfull=np.zeros_like(bark);unionfull[:175,:175]=union
    # This conservative eligibility mask never samples through a leaf in any phase.
    # It is still a proposal: lack of foreground alpha is not evidence of ownership.
    conservative=bark&~unionfull
    Image.fromarray(conservative.astype('uint8')*255).save(O/'all-phase-clear-bark-proposal.png')
    phase_rows=[]
    for i,p in enumerate(phases):
        foreground=np.zeros_like(bark);foreground[:175,:175]=p
        visible=bark&~foreground
        Image.fromarray(visible.astype('uint8')*255).save(O/f'phase-{i:02}-unoccluded-bark-proposal.png')
        phase_rows.append({'phase':i,'frame':str(B/'tree01-native-fragment-v1'/f'{i:03}.png'),'frame_sha256':pins[str(B/'tree01-native-fragment-v1'/f'{i:03}.png')],'foreground_excluded':int((bark&foreground).sum()),'unoccluded_proposal':int(visible.sum()),'native_outside_occupancy':int((p&~occupancy).sum())})
    original=B.parent/'baseline/covered.png';pins[str(original)]=sha(original)
    source=np.array(Image.open(original).convert('RGB'));marked=source.copy()
    area=marked[:175,:175];area[static&occupancy]=[255,190,45];area[static&~occupancy]=[255,40,180]
    sheet=Image.new('RGB',(700,350));sheet.paste(Image.fromarray(source).crop((0,0,350,350)));sheet.paste(Image.fromarray(marked).crop((0,0,350,350)),(350,0));sheet.save(O/'native-broad-context.png')
    report={'status':'UNASSIGNED_SOURCE_ROLE_AND_PHASE_OCCLUSION_EVIDENCE','pins':pins,'static':{'total':int(static.sum()),'inside_occupancy_proposal':4254,'outside_occupancy_explicit_exclusion':1777,'checkerboard_parity_counts':[int((static&((xx+yy)%2==i)).sum())for i in range(2)],'clusters':rows},'native_union_outside_occupancy':int((union&~occupancy).sum()),'bark':{'reviewed_spines':int(old.sum()),'upper_hit_consistent':98,'upper_explicit_misses_xy':sorted(misses),'all_phase_clear_proposal':int(conservative.sum()),'phases':phase_rows},'limits':['No source ownership assigned and no model, native frame, or texture modified.','Occupancy is not an ownership oracle: 1026 observed native union pixels lie outside this static mask across animation.','Each connected cluster remains ambiguous until broader context and alpha-aware neighboring ownership establish it; parity and adjacency alone do not suffice.','Per-phase bark masks identify absence of native foreground only; covered source image has no independently proven animation-state provenance. Do not use these masks to infer a hidden bark color.','All-phase-clear bark proposal conservatively excludes every native leaf crossing; remaining bark still needs exact source RGB and receiver validation.']}
    (O/'receipt.json').write_text(json.dumps(report,indent=2)+'\n')
    print({'clusters':len(rows),'static_parity':report['static']['checkerboard_parity_counts'],'all_phase_clear_bark':int(conservative.sum()),'native_union_outside':int((union&~occupancy).sum())})
if __name__=='__main__':main()
