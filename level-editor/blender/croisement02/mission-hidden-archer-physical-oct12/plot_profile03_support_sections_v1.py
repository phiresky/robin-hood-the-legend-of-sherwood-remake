"""Plot the eight saved foliage-to-wood junctions without loading receiver textures."""
from pathlib import Path

original = Path(__file__).with_name('plot_support04_sections_v1.py')
source = original.read_text()
for old, new in {
    'support04-saved-junction-readback-v1': 'support03-saved-junction-readback-v1',
    'geometry-v8-support04-sweep-v4': 'profile03-proposed-support-sweep-v2',
    "['rows'][3]": "['rows'][2]",
    'support04-junction-sections-v1': 'support03-junction-sections-v1',
}.items():
    assert old in source
    source = source.replace(old, new)
exec(compile(source, str(original), 'exec'), {'__name__':'__main__', '__file__':str(original)})
