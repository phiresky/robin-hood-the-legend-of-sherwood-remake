"""Build the mask68 stump and its complete saved-material inspection packet."""
import sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).parent))
import prepare_props,render_candidate,audit_native_coverage
sys.argv=['prepare_props','--','--asset','southeast-small-stump'];prepare_props.main()
workspace=prepare_props.OUT/'props-round-1/assets/croisement01-southeast-small-stump'
sys.argv=['render_candidate','--',str(workspace)];render_candidate.main()
sys.argv=['audit_native_coverage','--',str(workspace),'--mask','68'];audit_native_coverage.main()
