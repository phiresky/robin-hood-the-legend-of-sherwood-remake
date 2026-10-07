"""Reconstruct only the three demonstrated overlapping rooted junction contacts."""
import json,shutil
from pathlib import Path
from restart2_tree08_remaining_forks import inputs,reconstruct
R=Path(__file__).resolve().parents[2]/'work/croisement01-refinement/restart2'
def main():
 meshes,adj,groups,origin=inputs(R)
 prior=json.loads((R/'tree08-v12-local-contact-plan.json').read_text())
 for a,b in prior['additional_contact_pairs']:adj[a].add(b);adj[b].add(a)
 contacts=[(7,91),(4,15),(5,15)];affected=sorted({i for pair in contacts for i in pair})
 for a,b in contacts:adj[a].add(b);adj[b].add(a)
 out=R/'tree08-v12-remaining-group0-v3';out.mkdir(exist_ok=False)
 (out/'scope.json').write_text(json.dumps(dict(group=0,sections=groups[0],additional_contacts=contacts,affected_sections=affected,reason='Local intersections across rooted junction neighborhoods connected through sections8and3; own native crop shows connected broad wood. Exact prior source surfaces retained; no held independent crossings joined.'),indent=2)+'\n')
 for i in groups[0]:
  if i not in affected:
   for ext in ['json','npz']:shutil.copy2(R/f'tree08-v12-remaining-group0-v2/part-{i}.{ext}',out/f'part-{i}.{ext}')
 for i in affected:print(reconstruct(i,meshes,adj,origin,R/'tree08-v12-local-fork-cpu-v1/cap-clip-v2',out),flush=True)
if __name__=='__main__':main()
