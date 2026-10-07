"""Scoped limits for small, resumable mound contact evidence."""
import hashlib,re,shutil
from pathlib import Path
MIB=1024**2
GIB=1024**3

def digest(path):
 h=hashlib.sha256()
 with Path(path).open('rb')as stream:
  for chunk in iter(lambda:stream.read(MIB),b''):h.update(chunk)
 return h.hexdigest()

def allocated(path):
 return sum(p.stat().st_blocks*512 for p in Path(path).rglob('*')if p.is_file())

def check(site,total,reserve=0):
 if shutil.disk_usage(site).free-reserve<10*GIB:raise RuntimeError('Scoped contact hard free floor10GiB reached')
 if allocated(site)+reserve>8*MIB:raise RuntimeError('Scoped contact site8MiB cap reached')
 if allocated(total)+reserve>160*MIB:raise RuntimeError('Scoped contact total160MiB cap reached')
 for file in Path(site).rglob('*'):
  if file.is_file()and file.stat().st_size>4*MIB:raise RuntimeError('Scoped contact persistent file4MiB cap reached')

def source_name(expected,names):
 # Saved import suffixes are not stable source object identities.
 normalize=lambda s:re.sub(r'\.\d{3}$','',s)
 matches=[n for n in names if normalize(n)==normalize(expected)]
 if len(matches)!=1:raise ValueError(f'Nonunique pinned source object: {expected}: {matches}')
 return matches[0]

def png_bound(width,height,channels):
 """Conservative8-bit PNG envelope: filtered rows, deflate bound, chunks,64KiB metadata."""
 if width<1 or height<1 or channels not in (3,4):raise ValueError('Unsupported contact raster')
 raw=height*(width*channels+1)
 compressed=raw+(raw>>12)+(raw>>14)+(raw>>25)+13
 return compressed+12*((compressed+65535)//65536)+128+65536
