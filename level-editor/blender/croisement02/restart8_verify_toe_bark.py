"""Reopen exact bark candidate for source, shader and local contact checks."""
import sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
import restart8_review_toe_bark as shader
import restart8_toe_bark_native_guard as native
import restart8_toe_bark_contact as contact
from render_slots import acquire,release
acquire()
try:
 number=int(sys.argv[sys.argv.index('--')+1]);shader.main(number);native.main(number);contact.main(number)
finally:release()
