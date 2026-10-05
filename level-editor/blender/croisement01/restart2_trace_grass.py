"""Prepare the isolated native grass75 leaf-path guide before its Blender recipe."""
import sys
import argparse
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import trace_grass_leaves


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--mask',type=int,choices=[74,75,76,82],default=75);args=parser.parse_args()
    original = trace_grass_leaves.OUT
    destination = original/f'restart2/grass{args.mask}-source-v1'
    destination.mkdir(parents=True, exist_ok=False)
    (destination/'baseline').symlink_to(original/'baseline')
    trace_grass_leaves.OUT = destination
    sys.argv = ['trace_grass_leaves', '--mask', str(args.mask), '--revision', '3']
    trace_grass_leaves.main()


if __name__ == '__main__':
    main()
