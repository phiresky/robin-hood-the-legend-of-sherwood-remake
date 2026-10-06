"""Recheck unchanged wagon supports and render filled ground contacts."""
import sys,shutil
from pathlib import Path
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
import restart4_south_initial_wagon as wagon
from catalog import OUT
wagon.DEST=OUT/'restart4-south-cart-texture/approved-fill-v1/experiment/native-retained-v1'
wagon.VARIANT='v8';wagon.AUDIT='current-support-v1'
for role in ('roof','fore_platform','cabin_walls'):
 shutil.copyfile(OUT/f'restart3-south-cart/initial-physical-v8/{role}-source.png',wagon.DEST/f'{role}-source.png')
wagon.support()
