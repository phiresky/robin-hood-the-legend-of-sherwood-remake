"""Read RGB textures even when the receiver material is opaque."""
from pathlib import Path
original=Path(__file__).with_name('joined02_source_roles_v1.py')
source=original.read_text().replace("joined02-source-roles-v1'","joined02-source-roles-v2'")
# The reader lazily loads textures through its alpha path. A temporary argument
# enables loading opaque RGB without changing the actual material or ray tests.
source=source.replace(']=texture_record(mat)', "]=texture_record(dict(mat,alphaMode='BLEND'))")
exec(compile(source,str(original),'exec'),{'__name__':'__main__','__file__':str(original)})
