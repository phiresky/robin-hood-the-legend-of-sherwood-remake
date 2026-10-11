"""Construct conservative per-piece depth exclusions for an own-source flight candidate."""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1');os.environ.setdefault('OMP_NUM_THREADS','1')
import argparse
from pathlib import Path
import restart26_butterfly07_piece_envelope as envelope

def main():
 p=argparse.ArgumentParser();p.add_argument('--candidate',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();assert not a.output.exists()
 envelope.pinned.FIT=a.candidate/'fit.json';envelope.pinned.FIT_SHA=envelope.reader.sha(envelope.pinned.FIT);envelope.pinned.HELPER=a.candidate/'geometry.py';envelope.pinned.HELPER_SHA=envelope.reader.sha(envelope.pinned.HELPER);envelope.OUT=a.output;envelope.main()
if __name__=='__main__':main()
