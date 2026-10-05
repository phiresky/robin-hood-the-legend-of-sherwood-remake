"""Finite native-mask and manually surveyed landmark audit; never infer rigid identity."""
import hashlib,json
from pathlib import Path
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/croisement02-refinement'
DEST=OUT/'restart3-rock-motion-source'

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')

def main():
    DEST.mkdir(exist_ok=True)
    target_path=OUT/'state-target-evidence/rock-trap/manifest.json';motion_path=target_path.parent/'full-motion/manifest.json';endpoint_path=OUT/'rock-trap-state-candidate-v14/manifest.json';model=endpoint_path.parent/'worker.blend'
    source=json.loads(target_path.read_text());motion=json.loads(motion_path.read_text());endpoint=json.loads(endpoint_path.read_text())
    assert sha(model)==endpoint['model_sha256']=='81caa8332b5d8f436c2804425a8f456841ca8b7af219ea6771a64586b837d216'
    assert len(source['parts'])==1
    part=source['parts'][0];box=source['bbox'];width,height=box[2]-box[0],box[3]-box[1];frames=part['frames'];assert len(frames)==len(motion['records'])==35
    masks=DEST/'native-alpha';masks.mkdir(exist_ok=False);rows=[]
    for frame,record in zip(frames,motion['records']):
        p=Path(frame['image']);assert sha(p)==frame['image_sha256'];im=Image.open(p).convert('RGBA');canvas=Image.new('RGBA',(width,height));x,y=[int(part['position'][i]+frame['offset'][i]-box[i])for i in [0,1]];canvas.alpha_composite(im,(x,y));full=motion_path.parent/record['image'];original=Image.open(full).convert('RGBA');assert canvas.tobytes()==original.tobytes(),record
        assert hashlib.sha256(canvas.tobytes()).hexdigest()==record['rgba_sha256']
        assert frame['ticks']==frame['delay']+1==3
        mask=masks/record['image'];canvas.getchannel('A').save(mask)
        rows.append(dict(frame=frame['index'],first_tick=record['first_tick'],last_tick=record['last_tick'],start_seconds=record['first_tick']/25,source_image=str(p),source_sha256=sha(p),composite_sha256=sha(full),alpha_mask=str(mask),alpha_mask_sha256=sha(mask),nonzero_alpha_pixels=sum(a>0 for a in canvas.getchannel('A').getdata()),sound_id=frame['sound_id']))
    landmarks={33:{'0':[322,389]},42:{'0':[316,393],'1':[331,411],'4':[332,434]},60:{'0':[302,394],'1':[313,419],'4':[319,438],'2+3':[336,403]},78:{'0':[285,397],'1':[305,415],'4':[319,437],'2+3':[327,402]},102:{'0':[284,399],'1':[307,417],'2':[321,400],'3':[335,408],'4':[319,439]}}
    colors={'0':(255,140,100),'1':(100,220,255),'2':(220,140,255),'3':(255,180,240),'4':(255,255,80),'2+3':(220,140,255)}
    sheet=Image.new('RGB',(width*5*len(landmarks),height*5+45),(55,55,55));draw=ImageDraw.Draw(sheet)
    for col,(tick,points)in enumerate(landmarks.items()):
        im=Image.open(motion_path.parent/f'{tick:03}.png').convert('RGBA').resize((width*5,height*5),Image.Resampling.NEAREST);ox=col*width*5;sheet.paste(im,(ox,0),im)
        for name,point in points.items():
            px=ox+(point[0]-box[0])*5;py=(point[1]-box[1])*5;draw.ellipse((px-7,py-7,px+7,py+7),outline=colors[name],width=2);draw.text((px+9,py-9),name,fill=colors[name])
        draw.text((ox+5,height*5+5),f'Tick {tick}; approximate visible-lobe centers',fill='white')
    sheet.save(DEST/'annotated-landmarks.png')
    bodies=[dict(id=0,terminal_center=[284,399],earliest_partial_candidate_tick=15,first_conservatively_readable_tick=33,supported_visible_lobe_interval=[33,102],ambiguous_interval=[0,32],note='Leading crescent becomes a readable rounded upper-left lobe. Earlier sparse fragments do not prove initial-body identity.'),dict(id=1,terminal_center=[307,417],earliest_partial_candidate_tick=33,first_conservatively_readable_tick=42,supported_visible_lobe_interval=[42,102],ambiguous_interval=[0,41],note='Lower/front lobe emerges from the connected central cluster. Partial texture before42 cannot uniquely identify terminal body1.'),dict(id=2,terminal_center=[321,400],first_conservatively_readable_tick=None,supported_visible_lobe_interval=None,ambiguous_interval=[0,102],note='Only joint2+3 right-cluster path supported. Internal lobe apparent in late78+frames is not an independently tracked rigid body.'),dict(id=3,terminal_center=[335,408],first_conservatively_readable_tick=None,supported_visible_lobe_interval=None,ambiguous_interval=[0,102],note='Only joint2+3 right-cluster path supported; no independent emergence event establishes terminal3 identity.'),dict(id=4,terminal_center=[319,439],earliest_partial_candidate_tick=39,first_conservatively_readable_tick=42,supported_visible_lobe_interval=[42,102],ambiguous_interval=[0,41],note='Small detached lower stone readable from42; tiny39sample is a candidate only. Initial parent identity unsupported.')]
    report=dict(status='Finite source-only audit complete; physical motion remains unproven',source_manifest_sha256=sha(target_path),motion_manifest_sha256=sha(motion_path),endpoint_manifest_sha256=sha(endpoint_path),endpoint_model_sha256=sha(model),source_bbox=box,tick_rate=25,unique_native_phases=35,phase_duration_ticks=3,terminal_frame_entry_tick=102,nominal_row_duration_ticks=105,terminal_freeze='Stops on entry to frame34 at tick102; do not add its nominal delay before terminal clamp.',metadata_swap_tick=source['metadata_swap_tick'],metadata_distinction='Tick60 metadata swap is separate from visible completion at102 and does not prove fragmentation timing.',source_composition_exact_all35=True,phase_records=rows,terminal_body_observations=bodies,joint_right_cluster=dict(ids=[2,3],sparse_candidate_interval=[51,59],readable_cluster_interval=[60,102],late_internal_lobes_interval=[78,102],independent_body_assignment=False),landmarks=landmarks,landmark_semantics='Manually surveyed visible-lobe centers, approximately3native pixels uncertainty. Neither center of mass nor exact body-owned alpha mask. Terminal landmark coordinates bind endpoint hypothesis only.',initial_hypothesis=endpoint['covered_body_hypothesis'],conclusion='Two initial visible cap regions do not determine the count of hidden bodies, their terminal correspondence, or a fracture event. Preserve source-supported visible intervals and treat early disappearance/reappearance and the right cluster as ambiguous. A2-to5rigid identity or breakage mapping cannot be derived from this finite source pass.',supported_trajectory_scope='2D visible-lobe trajectories for0/1/4 and joint2+3cluster only. Source-ray depth, hidden rotations, mass, contacts and early body correspondences remain inferred.',limitations=['Conservative first-readable ticks are inspection thresholds, not exact physical birth or fracture times.','Native target alpha intentionally omits large portions during early motion. It does not reveal whether an absent pixel is occluded, outside sprite silhouette, or artist-omitted.','Complete per-phase alpha masks are exact source masks, not per-boulder ownership partitions.','No NCC optimization, geometry mutation, renderer invocation, API, canonical state change or motion export performed.','Old geometry array includes historical initial3 entries; current initial_pair and covered_body_hypothesis explicitly specify2. Do not mistake historical array entries for current body count.'])
    write(DEST/'report.json',report);print(DEST/'report.json')
if __name__=='__main__':main()
