"""Bind current receiver caps to exact focused hole contracts privately."""
import json,hashlib
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/active/croisement02';DEST=BASE/'restart39-current-hole-apertures-v1';END=BASE/'restart38-approved-hole-export-v1'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def pin(p):return dict(path=str(p),sha256=sha(p))
def read(p):return json.loads(Path(p).read_text())
def main():
 partition=read(DEST/'receiver-partition.json');config=read(END/'reader-config.json');assert sha(partition['map']['path'])==partition['map']['sha256'];receivers=partition['receivers'];assert len(receivers)==2
 for receiver in receivers:
  for k in ('source','descriptor','model','provenance'):assert sha(receiver[k]['path'])==receiver[k]['sha256']
 controls=[];sites=[]
 for h in partition['holes']:
  rows=[r for r in config['bindings']if r['id']in h['triggers']];assert len(rows)==len(h['triggers']);placements={s:rows[0]['physical'][s][0]['position']for s in ('initial','applied')};assert placements['initial']==placements['applied']
  for row in rows:
   source=read(row['contract']['path']);assert sha(row['contract']['path'])==row['contract']['sha256'];assert len(source['families'])==1 and source['families'][0]['patch_ids']==[row['id']]
   assert all(row['physical'][s][0]['position']==placements[s]for s in placements);controls.append(dict(id=row['id'],mission=row['mission'],family=source['families'][0]['id'],patches=[row['id']],site=h['id'],terminal_tick=row['terminal_tick'],transition_duration=row['transition_duration'],source_contract=row['source_contract'],contract=row['contract']))
  sites.append(dict(id=h['id'],patches=h['triggers'],display_position=h['position'],support_z=h['support_z'],endpoint_position=placements['initial'],receiver_asset='croisement02-terrain'if h['support_z']==0 else'croisement02-north-woodland-bank'))
 report=dict(status='PRIVATE_CURRENT_APERTURES_FOCUSED_CONTRACTS_BROWSER_PENDING',map=partition['map'],receiver_partition=pin(DEST/'receiver-partition.json'),endpoint_config=pin(END/'reader-config.json'),endpoint_approval=config['approval'],receivers=receivers,endpoints={s:dict(model=config['models'][s],scene='endpoint-'+s)for s in ('initial','applied')},sites=sites,controls=controls,native_reader=config['native_reader'],asset_handle=config['asset_handle'],placement_frame='Models are Y-up. Prepared root rotates +90 degrees about X into the viewport map Z-up frame. Receiver model scenes contain exact source placement; endpoint translations are runtime Y-up.',lifecycle={'initial':'all caps closed; current mission covered endpoints visible','applied':'selected cap open; selected opened endpoint visible','force_reset':'restore selected cap and initial endpoint','focus_or_mission_change':'retire old session and reset caps; display only initial sites used in selected mission','native_mode':'restore original receiver visibility','dispose':'restore original receiver visibility and release prepared root'},canonical_writes=False,browser_verified=False,recipe=pin(Path(__file__).resolve()),limitations=['Current surface and material preservation are CPU-verified; actual browser appearance and depth remain to be reviewed.','Exact existing per-control contracts are retained. Combining native phase domains is unsupported where restoration rectangles overlap.','Other same-mission sites remain in initial state; selected focus is the only live phase controller. No physical interpolation is inferred.'])
 p=DEST/'integration-focused-v2.json';assert not p.exists();p.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(dict(sites=len(sites),controls=len(controls),integration=pin(p))))
if __name__=='__main__':main()
