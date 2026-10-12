"""Complete the remaining seven approved pose derivatives under one bounded lease."""
import json,sys,time,shutil
from pathlib import Path
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
import bake_approved03_04_textures_v1 as bake
import review_approved03_04_textures_v1 as review
from render_slots import acquire,release
STATES=['03-transition-0','03-transition-1','03-applied','04-initial','04-transition-0','04-transition-1','04-applied']
def main(mode):
    receipt=bake.BASE/'baked-v1/03-initial/saved-review-v2/self-review.json'
    initial=json.loads(receipt.read_text());assert initial['ready_for_remaining_bakes']
    assert bake.sha(bake.BASE/'baked-v1/03-initial/model.blend')==initial['model_sha256']
    pending=[]
    if mode=='bake':
        for state in STATES:
            bake.preflight(state)
            packet=bake.BASE/'baked-v1'/state
            if packet.exists():
                existing=json.loads((packet/'bake-validation.json').read_text())
                assert bake.sha(packet/'model.blend')==existing['model_sha256']
                assert existing['generation_review_sha256']==bake.sha(bake.BASE/'experiments-v2'/state/'experiment'/bake.GEN/'generation-review.json')
            else:pending.append(state)
    else:
        for state in STATES:
            packet=bake.BASE/'baked-v1'/state
            assert bake.sha(packet/'model.blend')==json.loads((packet/'bake-validation.json').read_text())['model_sha256']
            if (packet/'saved-review-v2').exists():
                existing=json.loads((packet/'saved-review-v2/report.json').read_text())
                assert bake.sha(packet/'model.blend')==existing['model_sha256']
                for path,digest in existing['images'].items():assert bake.sha(path)==digest
            else:pending.append(state)
    while True:
        while bake.available()<11*1024**3:print('Waiting 11 GiB for remaining archer batch',flush=True);time.sleep(30)
        assert shutil.disk_usage(bake.ROOT).free>=10*1024**3
        acquire()
        try:
            if bake.available()<10*1024**3:continue
            for state in pending:
                assert bake.available()>=10*1024**3
                (bake.main if mode=='bake' else review.main)(state)
            break
        finally:release()
    print('ARCHER_REMAINING_BATCH_COMPLETE',mode,flush=True)
if __name__=='__main__':
    mode=sys.argv[sys.argv.index('--')+1];assert mode in ['bake','review'];main(mode)
