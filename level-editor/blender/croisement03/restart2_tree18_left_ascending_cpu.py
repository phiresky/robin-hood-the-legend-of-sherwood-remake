"""Restore the separately visible ascending fork as a private source hypothesis."""
import copy
import hashlib
import json
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw
import restart2_tree18_cpu_wood as sweeps
import restart2_tree18_root_connectivity as topology

B=Path(__file__).resolve().parents[3]/'level-editor/work/active/croisement03/restart2'


def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()


def main():
    source=B/'tree18-root-connectivity-cpu-v1/geometry.json';original=json.loads(source.read_text());out=B/'tree18-five-track-cpu-v2';assert not out.exists()
    world_y=float(np.median([v[1]for p in original['parts']for v in p['vertices']]))
    # Broad strip follows visible ascending bark. Upper leaf crossings are
    # explicitly inferred continuation, never accepted exposed bark pixels.
    samples=[(419,436,390,'visible_ascending_bark_envelope'),(414,429,374,'visible_ascending_bark_envelope'),(413,427,358,'bark_with_leaf_crossings'),(408,423,344,'bark_with_leaf_crossings'),(405,419,329,'inferred_leaf_obscured_continuation')]
    sections=[]
    for left,right,y,role in samples:
        x=(left+right)/2;sections.append({'endpoints_source_xy':[[left,y],[right,y]],'center_world_hypothesis':[x,world_y,(-world_y*sweeps.S-y)/sweeps.C],'projected_width_pixels':right-left,'source_role':role})
    part=sweeps.build({'track':'left_ascending','native_obstacle_context':[33,34],'sections':sections},-world_y*sweeps.S)
    part['source_endpoint_role']='New manual source-envelope proposals, not measured pixel ownership or approved guards.'
    part['source_section_roles']=[s['source_role']for s in sections]
    crossings=topology.crossing_pairs(part);assert not crossings,crossings
    result=copy.deepcopy(original);result['parts'].append(part)
    assert result['parts'][:4]==original['parts'];result['status']='PRIVATE_FIVE_TRACK_PROPOSAL_UNION_AND_CONTEXT_PENDING'
    result['missing_branch_revision']={'pins':{str(p):sha(p)for p in [source,Path(__file__),Path(sweeps.__file__),Path(topology.__file__)]},'reason':'Prior four tracks included the lower trunk, so only three upper stems were constructed. Source shows an additional ascending left stem distinct from the horizontal left fork.','new_track':'left_ascending','sections':sections,'old44_endpoint_vertices_unchanged':True,'self_crossing_pairs':crossings,'limits':['Upper leaf-obscured depth/continuation remains inferred. Do not texture this full envelope as exposed bark.','Saved union must preserve old first hits except where the newly source-supported branch legitimately occupies a previously missing surface; record changes rather than ignore them.','Source2D geometry proposals do not authorize native alpha clipping or determine editor-plane physical depth.']}
    out.mkdir();(out/'geometry.json').write_text(json.dumps(result,indent=2)+'\n')
    image=Image.open(B.parent/'baseline/covered.png').convert('RGB');box=(350,265,485,415);crop=image.crop(box).resize((675,750),Image.Resampling.NEAREST);panel=Image.new('RGB',(1380,780),'#222222');panel.paste(crop,(10,20));panel.paste(crop,(695,20));draw=ImageDraw.Draw(panel)
    for section in sections:
        a,b=section['endpoints_source_xy'];a=((a[0]-box[0])*5+695,(a[1]-box[1])*5+20);b=((b[0]-box[0])*5+695,(b[1]-box[1])*5+20);draw.line([a,b],fill='cyan'if section['source_role'].startswith('visible')else'orange',width=2)
    points=(np.array([sweeps.project(v)for v in part['vertices']])-box[:2])*5+np.array([695,20])
    for f in part['faces']:draw.line([tuple(points[i])for i in f+[f[0]]],fill='#ff80ff',width=1)
    panel.save(out/'source-unmarked-and-new-branch.png');print({'parts':len(result['parts']),'new_branch_faces':len(part['faces']),'crossings':len(crossings)})


if __name__=='__main__':main()
