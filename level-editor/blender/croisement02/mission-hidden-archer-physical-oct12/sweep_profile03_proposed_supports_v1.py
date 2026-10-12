"""Check eight proposed short attachments before spending memory on construction."""
import json
import math
import sys
import time
from pathlib import Path
from types import SimpleNamespace

HERE = Path(__file__).resolve().parent
sys.path[:0] = [str(HERE), str(HERE.parents[2] / 'refinement')]
import qualify as q
import render_slots

DEST = q.OUT / 'profile03-proposed-support-sweep-v1'
CANCEL = q.OUT / 'cancel-profile03-proposed-support-sweep-v1'


def check_cancel():
    if CANCEL.exists():
        raise SystemExit('Cancelled at owned boundary before further allocation')


def checked_sleep(seconds):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        check_cancel()
        time.sleep(min(.2, max(0, deadline - time.monotonic())))


def main():
    check_cancel()
    plan_path = q.OUT / 'profile03-wood-route-plan-v1/report.json'
    plan = json.loads(plan_path.read_text())
    proposal_path = q.OUT / 'profile03-proposed-supports-v1.json'
    assert not proposal_path.exists() and not DEST.exists()
    sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))
    ray = [0, -cosine, sine]
    rows = []
    for state in plan['states']:
        for component in state['components']:
            for side in ['left', 'right']:
                candidates = [c for c in component['candidates'] if c['side'] == side]
                assert candidates, (state['phase'], component['component'], side)
                c = min(candidates, key=lambda c: c['gap'])
                assert c['gap'] <= 6
                t = c['target']['world_yup']
                target = [t[0], -t[2], t[1]]
                back = [target[i] + ray[i] * c['gap'] for i in range(3)]
                rows.append(dict(phase=state['phase'], component=f"{component['component']}-{side}",
                    status='BUILT_PROPOSAL_ONLY_NO_SAVED_MODEL', candidate=c, radius=.2,
                    support_start=[back[i] + ray[i] * .2 for i in range(3)],
                    support_end=[target[i] - ray[i] * .2 for i in range(3)]))
    proposal_path.write_text(json.dumps(dict(status='PROPOSED_NOT_CONSTRUCTED', rows=rows,
        input=q.pin(plan_path), recipe=q.pin(Path(__file__))), indent=2) + '\n')
    original = HERE / 'support04_sweep_v1.py'
    source = original.read_text().replace("DEST=q.OUT/'geometry-v8-support04-sweep'", 'DEST=proposal_dest')
    source = source.replace("input_path=q.OUT/'geometry-v8-short-support04/report.json'", 'input_path=proposal_input')
    source = source.replace("    assert q.pin(q.ROOT/construction['model']['path'])==construction['model']\n", '')
    namespace = dict(__name__='proposed_sweep', __file__=str(original),
                     proposal_dest=DEST, proposal_input=proposal_path)
    exec(compile(source, str(original), 'exec'), namespace)
    check_cancel()
    namespace['main']()


if __name__ == '__main__':
    def available():
        return int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines()
                        if x.startswith('MemAvailable:'))) * 1024
    # The existing FIFO and lock implementation remains unchanged. Only its
    # process-local sleep observes this lane's explicit cancellation file.
    clock = render_slots.time
    render_slots.time = SimpleNamespace(time_ns=time.time_ns, monotonic=time.monotonic, sleep=checked_sleep)
    try:
        while True:
            check_cancel()
            while available() < 11 * 1024**3:
                print('Waiting 11GiB before proposed-support sweep queue', flush=True)
                checked_sleep(30)
            render_slots.acquire(slots=2)
            try:
                check_cancel()
                if available() < 10 * 1024**3:
                    continue
                main()
                break
            finally:
                render_slots.release()
    finally:
        render_slots.time = clock
