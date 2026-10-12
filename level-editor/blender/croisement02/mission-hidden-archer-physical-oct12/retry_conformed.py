"""Retry identical clearance geometry without overwriting interrupted evidence."""
from pathlib import Path
import hashlib
here=Path(__file__).resolve().parent
for name,expected in {'build_conformed.py':'deaa31f261d207087bb460e57ce058de31fdee5619ba443402ea7ecd5af1e70f','build_models.py':'a86bc37af5ea3de13acc0d90398537ebbf2d41e8bedfd982994f4c840ba3be48'}.items():
    assert hashlib.sha256((here/name).read_bytes()).hexdigest()==expected
script=here/'build_conformed.py'
source=script.read_text();assert source.count("'geometry-v3-conformed'")==1
source=source.replace("'geometry-v3-conformed'","'geometry-v3-conformed-recovered'")
exec(compile(source,str(script),'exec'),{'__name__':'__main__','__file__':str(script)})
