"""Build and publish the editor at https://robinhood.phiresky.xyz/editor/."""
import argparse
import json
from pathlib import Path
import subprocess

from cloudflare_publish import EDITOR, worker_config, new_output, deploy


def stage_editor(output):
    output = Path(output).resolve()
    output.mkdir(parents=True, exist_ok=False)
    site = output/'site'
    subprocess.run(['pnpm', '--dir', str(EDITOR), '--filter', 'app', 'exec', 'vite', 'build',
                    '--base', '/editor/', '--outDir', str(site/'editor')], check=True)
    (site/'_headers').write_text('''/editor/*
  X-Content-Type-Options: nosniff

/editor/
  Cache-Control: public, max-age=0, must-revalidate

/editor/assets/*
  Cache-Control: public, max-age=31536000, immutable
''')
    (output/'wrangler.json').write_text(json.dumps(
        worker_config('robinhood-editor', '/editor', 'auto-trailing-slash'), indent=2)+'\n')
    print(f'Staged editor deployment: {output}', flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, help='Fresh output directory; defaults to a new work/disposable/editor-publish snapshot')
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument('--stage-only', action='store_true', help='Build without running Wrangler')
    mode.add_argument('--dry-run', action='store_true', help='Build and validate without uploading')
    args = parser.parse_args()
    output = args.output or new_output('editor')
    stage_editor(output)
    if not args.stage_only:
        deploy(output, args.dry_run)


if __name__ == '__main__':
    main()
