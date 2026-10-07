"""Bind completed live HTTP assertions, preserving incomplete runs and network diagnostics."""
from pathlib import Path
import hashlib
import json
import sys

ROOT = Path(__file__).resolve().parents[3]
STATE = ROOT / 'level-editor/work/croisement02-refinement/restart2-state'
base = STATE / sys.argv[1]
exit_code = int(sys.argv[2])
ids = sys.argv[3:]
assert ids
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
inputs = json.loads((base / 'inputs.json').read_text())
for path, expected in inputs['files'].items():
    assert sha(ROOT / path) == expected, path
package = json.loads((STATE / 'remaining-seven-package-v2/manifest.json').read_text())
overlay = {row['path'] for row in package['files']} | {'mission-states/index.json'}
served = {}
for line in (base / 'browser/served-snapshot.jsonl').read_text().splitlines():
    row = json.loads(line)
    path = (STATE / 'remaining-seven-package-v2/library' if row['path'] in overlay else ROOT / 'level-editor/library') / row['path']
    assert sha(path) == row['sha256'], str(path)
    served[str(path.relative_to(ROOT))] = row['sha256']
events = [json.loads(line) for line in (base / 'browser/runtime-events.jsonl').read_text().splitlines()]
requests = {e['params']['requestId']: e['params']['request']['url'] for e in events if e['method'] == 'Network.requestWillBeSent'}
noise = []
for event in events:
    params = event['params']
    assert event['method'] != 'Runtime.exceptionThrown', event
    if event['method'] == 'Network.loadingFailed':
        url = requests[params['requestId']]
        assert params.get('canceled') is True and params['errorText'] == 'net::ERR_ABORTED' and '/scenes/' in url and url.endswith('.avif'), event
        noise.append({'kind': 'canceled_map_picker_thumbnail', 'url': url})
    if event['method'] == 'Network.responseReceived' and params['response']['status'] >= 400:
        response = params['response']
        url = response['url']
        fallback = url[:-5] + '.webp' if url.endswith('.avif') else None
        fallback_ok = fallback is not None and any(e['method'] == 'Network.responseReceived' and e['params']['response']['url'] == fallback and e['params']['response']['status'] == 200 for e in events)
        assert response['status'] == 404 and (url.endswith('/favicon.ico') or ('/scenes/' in url and fallback_ok)), event
        noise.append({'kind': 'expected_thumbnail_format_probe_with_webp_success' if fallback_ok else 'private_fixture_missing_favicon', 'url': url, 'status': 404, 'successful_fallback': fallback if fallback_ok else None})
families = []
checks = []
for entry_id in ids:
    path = base / 'browser/remaining-seven-states' / (entry_id + '-terminal.json')
    family = json.loads(path.read_text())
    assert family['entry_id'] == entry_id and family['status'] in ('PASS', 'PASS_CURRENT_LIVE_FAMILY')
    authority = next(e for e in package['entries'] if e['id'] == entry_id)
    assert family['contract_sha256'] == authority['contract']['sha256']
    assert family['checks'] and all(row['pass'] is True for row in family['checks'])
    checks.extend(family['checks'])
    families.append({'path': str(path.relative_to(ROOT)), 'sha256': sha(path), 'checks': len(family['checks'])})
result = {'status': 'PASS_COMPLETED_SCOPED_FAMILIES_POSTFLIGHT', 'tool_exit_code': exit_code,
          'termination_reason': 'Unknown external termination; no stop issued' if exit_code == 143 else 'See original process-final and failure/verification; no failure evidence overwritten',
          'inputs_sha256': sha(base / 'inputs.json'), 'unchanged_input_count': len(inputs['files']),
          'static_map_sha256': inputs['static_map_sha256'], 'served_files': served, 'completed_families': families,
          'checks': checks, 'disclosed_fixture_network_noise': noise,
          'scope': 'Only named completed terminal families accepted. All source/static pins and served resources rehashed. Does not treat partial assertion counters as completed families. Favicon404 and explicit thumbnail cancellation are disclosed; no model/contract resource failure or runtime exception accepted.'}
output = base / 'browser/completed-families-postflight.json'
assert not output.exists()
output.write_text(json.dumps(result, indent=2) + '\n')
print(output, len(checks), 'checks', len(served), 'served resources')
