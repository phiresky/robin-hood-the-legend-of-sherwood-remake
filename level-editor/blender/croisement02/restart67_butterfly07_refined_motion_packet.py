"""Stage the improved source-pose fit with fresh own-source materials and full flight tracks."""
from pathlib import Path
import restart65_butterfly07_exact_observation_atlas as wings
import restart66_butterfly07_body_observations as body
import restart66_butterfly07_fullpath_export as export
import restart66_verify_butterfly07_fullpath as verify
BASE=export.BASE;FIT=BASE/'restart66-butterfly07-global-bright-fit-v1/fit.json'
def main():
 wings.FIT=FIT;wings.OUT=BASE/'restart67-butterfly07-wing-observations-v1';wings.main()
 body.FIT=FIT;body.OUT=BASE/'restart67-butterfly07-body-observations-v1';body.main()
 export.FIT=FIT;export.ATLAS=wings.OUT;export.BODY_ATLAS=body.OUT/'body.png';export.OUT=BASE/'restart67-butterfly07-fullpath-export-v1';export.main()
 verify.FIT=FIT;verify.ATLAS=wings.OUT;verify.BODY_ATLAS=export.BODY_ATLAS;verify.OUT=export.OUT;verify.MODEL=export.OUT/'butterfly07-fullpath.glb';verify.main()
if __name__=='__main__':main()
