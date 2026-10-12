"""Save the source-guarded coupled cap and rounded western stone candidate."""
import hashlib
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import restart2_bank_lofts_saved as builder


def main():
    builder.CPU = builder.B / 'bank-coupled-left-cpu-v3'
    builder.E = builder.B / 'bank-convex-east-cpu-v1/geometry.json'
    builder.W = builder.B / 'bank53-mass-bulk-v2/geometry.json'
    builder.OUT = builder.B / 'bank-coupled-left-saved-v1'
    assert hashlib.sha256(builder.W.read_bytes()).hexdigest() == '98c54d2d9f1033d7d3f80e7a4423dd407be25e5c13faf8fb632e7bd2f748d32a'
    builder.main()


if __name__ == '__main__':
    main()
