"""Construct the two short trunk attachments only after finite clearance passes."""
import json
import sys
import time
from pathlib import Path
from types import SimpleNamespace

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import build_models as build
import render_slots

CANCEL = build.q.OUT / 'cancel-profile03-support-build-v1'


def check_cancel():
    if CANCEL.exists():
        raise SystemExit('Cancelled at owned boundary before model loading')


def checked_sleep(seconds):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        check_cancel()
        time.sleep(min(.2, max(0, deadline - time.monotonic())))


def main():
    check_cancel()
    sweep_path = build.q.OUT / 'profile03-proposed-support-sweep-v2/report.json'
    sweep = json.loads(sweep_path.read_text())
    proposals = build.q.OUT / 'profile03-proposed-supports-v2.json'
    assert sweep['construction'] == build.q.pin(proposals)
    assert len(sweep['rows']) == 8
    for row in sweep['rows']:
        assert row['counts'].get('OTHER_RECEIVER_CONTACT', 0) == 0, row
        assert row['counts'].get('TARGET_WOOD_REQUIRES_JUNCTION_REVIEW', 0) > 0, row
    plan_path = build.q.OUT / 'profile03-wood-route-plan-v1/report.json'
    plan = json.loads(plan_path.read_text())
    selected = dict(states=[])
    for state in plan['states']:
        components = []
        for component in state['components']:
            for side in ['left', 'right']:
                candidate = min((c for c in component['candidates'] if c['side'] == side), key=lambda c: c['gap'])
                components.append(dict(component=f"{component['component']}-{side}", pixels=component['pixels'], candidates=[candidate]))
        selected['states'].append(dict(phase=state['phase'], components=components))
    candidates_path = build.q.OUT / 'profile03-selected-short-supports-v1.json'
    assert not candidates_path.exists()
    candidates_path.write_text(json.dumps(selected, indent=2) + '\n')
    original = HERE / 'build_support04_v1.py'
    source = original.read_text()
    replacements = {
        "geometry-v8-short-support04'": "geometry-v9-short-support03-v1'",
        'support-route-probe-v1/all-pose-attachment-candidates-v2.json': 'profile03-selected-short-supports-v1.json',
        "'profile-04/'": "'profile-03/'",
        'Inferred short support04-': 'Inferred short support03-',
        ";stem.hide_render=phase!='initial';stem.hide_viewport=phase!='initial'": '',
        "            rows.append(dict(phase=phase,component=component['component'],pixels=component['pixels'],\n                status='BUILT":
            "            stem.hide_render=phase!='initial';stem.hide_viewport=phase!='initial'\n            rows.append(dict(phase=phase,component=component['component'],pixels=component['pixels'],\n                status='BUILT",
    }
    for old, new in replacements.items():
        assert source.count(old) == 1, old
        source = source.replace(old, new)
    namespace = dict(__name__='profile03_construction', __file__=str(original))
    exec(compile(source, str(original), 'exec'), namespace)
    check_cancel()
    namespace['main']()
    report_path = build.q.OUT / 'geometry-v9-short-support03-v1/report.json'
    report = json.loads(report_path.read_text())
    report['preconstruction_clearance'] = build.q.pin(sweep_path)
    report['recipe'] = build.q.pin(Path(__file__))
    report['limits'].append('Eight proposed finite sweeps passed before construction; saved tube readback and both endpoint sections remain required.')
    report_path.write_text(json.dumps(report, indent=2) + '\n')
    # Read the just-saved file under this same bounded lease; do not retain a
    # loaded scene while waiting for a second phase to acquire another slot.
    check_cancel()
    readback_recipe = HERE / 'inspect_support04_saved_v1.py'
    readback_source = readback_recipe.read_text()
    for old, new in {
        'support04-saved-junction-readback-v1': 'support03-saved-junction-readback-v1',
        'geometry-v8-short-support04-v4': 'geometry-v9-short-support03-v1',
        'Inferred short support04-': 'Inferred short support03-',
    }.items():
        assert old in readback_source
        readback_source = readback_source.replace(old, new)
    readback_namespace = dict(__name__='profile03_saved_readback', __file__=str(readback_recipe))
    exec(compile(readback_source, str(readback_recipe), 'exec'), readback_namespace)
    readback_namespace['main']()


if __name__ == '__main__':
    def available():
        return int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines()
                        if x.startswith('MemAvailable:'))) * 1024
    clock = render_slots.time
    render_slots.time = SimpleNamespace(time_ns=time.time_ns, monotonic=time.monotonic, sleep=checked_sleep)
    try:
        while True:
            check_cancel()
            while available() < 11 * 1024**3:
                print('Waiting 11GiB before support construction queue', flush=True)
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
