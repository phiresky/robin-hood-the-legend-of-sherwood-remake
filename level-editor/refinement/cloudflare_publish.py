"""Shared static Worker deployment settings and Wrangler invocation."""
from pathlib import Path
import subprocess
import tempfile

EDITOR = Path(__file__).resolve().parents[1]
WRANGLER_PROJECT = EDITOR.parent / 'wasm-www'
HOST = 'robinhood.phiresky.xyz'


def worker_config(name, prefix, html_handling='none'):
    return {'name': name, 'compatibility_date': '2026-08-30', 'workers_dev': False,
            'preview_urls': False,
            'routes': [{'pattern': HOST + path, 'zone_name': 'phiresky.xyz'}
                       for path in (prefix, prefix + '/*')],
            'assets': {'directory': './site', 'html_handling': html_handling,
                       'not_found_handling': 'none'}}


def new_output(kind):
    # Deployment payloads are reproducible from the source app/library. Keep
    # them separate from final editable assets and raw synthesis outputs.
    runs = EDITOR / 'work' / 'disposable' / (kind + '-publish')
    runs.mkdir(parents=True, exist_ok=True)
    return Path(tempfile.mkdtemp(prefix='release-', dir=runs)) / 'deploy'


def deploy(output, dry_run=False):
    command = ['pnpm', '--dir', str(WRANGLER_PROJECT), 'exec', 'wrangler', 'deploy',
               '--config', str((output/'wrangler.json').resolve())]
    if dry_run:
        command.append('--dry-run')
    subprocess.run(command, check=True)
