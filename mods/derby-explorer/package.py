"""Build the small startup script and package the mission for the Rust port."""
from pathlib import Path
import struct
import zipfile

ROOT = Path(__file__).resolve().parent
LEVELS = ROOT / 'Data' / 'Levels'


def string(value):
    value = value.encode('ascii')
    return struct.pack('<I', len(value)) + value


def quad(opcode, operands=b''):
    return bytes([opcode]) + operands.ljust(8, b'\0')


# Initialize opens the two drawbridges; the remaining callbacks do nothing.
# VM local 0x8000 holds each patch index and then its resolved handle.
quads = [quad(3, struct.pack('<HH', 1, 0))]
for patch_index in (7, 8):
    quads.extend([
        quad(19, struct.pack('<HHi', 0x8000, 0, patch_index)),
        quad(11, struct.pack('<H', 0x8000)),
        quad(12, struct.pack('<I', 5)),  # GetPatchScript
        quad(13, struct.pack('<H', 0x8000)),
        quad(11, struct.pack('<H', 0x8000)),
        quad(12, struct.pack('<I', 145)),  # ApplyPatch
    ])
quads.append(quad(6))
empty_address = len(quads)
quads.extend([quad(3), quad(6)])
script = b'SBSCRIPT' + struct.pack('<fI', 1.5, 1)
script += string('DerbyExplorer.scs') + string('StartUp')
script += struct.pack('<iii', 0, 0, 3)
for name, address, parameters, local_size in [
    ('Initialize', 0, 1, 4),
    ('PostInitialize', empty_address, 0, 0),
    ('Finalize', empty_address, 1, 0),
]:
    script += string(name) + struct.pack(
        '<iiiiii', address, parameters, 0, parameters * 4, local_size, 0
    )
script += struct.pack('<i', len(quads)) + b''.join(quads)
(LEVELS / 'DerbyExplorer.scb').write_bytes(script)
with zipfile.ZipFile(ROOT / 'derby-explorer.zip', 'w', zipfile.ZIP_DEFLATED) as archive:
    for path in sorted(LEVELS.iterdir()):
        archive.write(path, path.relative_to(ROOT))
