"""Bind user endpoint decisions and bounded preview capability to the finite state inventory."""
import hashlib,json
from pathlib import Path
from catalog import OUT

def digest(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def evidence(name):return dict(path=name,sha256=digest(OUT/name))
def main():
    base=OUT/'restart2-state';old=base/'editor-state-integration-plan-v4.json';plan=json.loads(old.read_text());plan['prior_plan_sha256']=digest(old)
    plan['status']='Production loader and catalog preview playback verified; mission targets, receiver transitions and full-scene presentation remain open'
    plan['steps'][2].update(status='PARTIAL: asset catalog preview implemented and verified; placed mission/scene playback remains open',deliverable='Catalog preview has one clock, paused first frame, Play/Pause, frame seeking and explicit user-selected once/loop behavior. No mission semantics inferred from clip duration.',evidence='restart2-state/preview-browser-v1/verification.json',commit='873b948d3')
    plan['coordination']='Assigned preview/cache/controller and picker wrapper committed873b948d3. No MissionLayer/shared schema ownership assumed.'
    plan['isolated_player_module']['status']='Implemented with production loader retention and catalog preview controls; mission playback remains separate.'
    plan['loader_api']['limits'][-1]='Catalog preview owns one clock. No automatic mission or placed-scene playback.'
    plan['sign_context_supersession']['scope']='Prior unevaluated bank contexts remain superseded. Evaluated v6 proof exists; sibling is testing bounded shrub corrections across32 poses, not yet selected.'
    (base/'editor-state-integration-plan-v5.json').write_text(json.dumps(plan,indent=2)+'\n')
    old=base/'state-completion-checklist-v11/manifest.json';result=json.loads(old.read_text());result['prior_checklist_sha256']=digest(old)
    decisions=evidence('restart2-state/user-state-geometry-decisions-v2.json')
    for row in result['items']:
        if row['id']=='log-trap':
            row['verified_milestones']=[v for v in row['verified_milestones'] if not v.startswith('Root endpoint')]
            row['private_candidate_review']='USER REJECTED initial endpoint: requires triangular pile. Prior root initial geometry PASS superseded.'
            row['remaining'][0]='Finish source-constrained21-log triangular stack review. Private v5 has six supporting courses, closed volumes, source silhouette diagnostic and static support checks; visual render pending. Fallen endpoint unchanged; whole log card remains unapproved. No texture generation authorized.'
            row['evidence'].extend([decisions,evidence('restart2-state/log-triangular-pile-v5/reopened-support-audit.json'),evidence('restart2-state/log-triangular-pile-v5/axial-load-support.json')])
        if row['id']in ['rock-trap','net-rigging','scoped-fence-clearing']:
            row['evidence'].append(decisions)
            row['verified_milestones'].append('Exact scoped geometry approved by user in frozen v2 gallery response; this is not complete state or appearance approval.')
            row['remaining']=[v.replace('exact user approval remains pending.','exact user geometry approval is now recorded; guarded appearance fills are in progress.').replace('root geometry decision','user geometry decision') for v in row['remaining']]
        if row['id']=='rock-trap':row['verified_milestones']=[v for v in row['verified_milestones']if v!='Root endpoint geometry review PASS; no user or complete-state approval.']
        assert row['status']=='incomplete'
    result['integration_plan']=evidence('restart2-state/editor-state-integration-plan-v5.json')
    result['isolated_player']['status']='Production loaders retain clone-safe clips; catalog preview clock/control/cache integration PASS.47 focused tests and actual export browser lifecycle checks; mission playback still open.'
    result['preview_player']=dict(commit='873b948d3',evidence=evidence('restart2-state/preview-browser-v1/verification.json'),scope='Catalog preview only; no inferred mission semantics')
    result['geometry_review_request']=dict(status='User approves rock endpoints, occupied net and cleared fence; rejects initial logs. New log geometry requires new exact review.',evidence=[decisions])
    result['status']='Finite state checklist v12; user geometry decisions and catalog playback progress recorded, all11 integrated state workstreams remain incomplete'
    dest=base/'state-completion-checklist-v12';dest.mkdir(exist_ok=False);(dest/'manifest.json').write_text(json.dumps(result,indent=2)+'\n')
if __name__=='__main__':main()
