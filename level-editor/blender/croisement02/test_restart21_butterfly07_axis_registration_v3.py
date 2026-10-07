"""Verify closed-cycle selection and measured-source landmark penalties."""
import copy,json,unittest
import numpy as np
from scipy.spatial.transform import Rotation
import restart21_butterfly07_axis_registration_v3 as recipe

class RegistrationTests(unittest.TestCase):
    def test_selection_includes_last_to_first_transition(self):
        def candidate(angle,loss):
            return dict(parameters=[0,0,angle,0,0,0,0],quaternion=Rotation.from_euler('z',angle,degrees=True).as_quat().tolist(),loss=loss)
        banks={p:[candidate(0,0),candidate(90,-1.5 if p==0 else 20)] for p in range(99)}
        # A single open-chain transition costs 1.11, but closing the cycle costs
        # twice that: phase0's apparent 1.5 advantage must not win.
        chosen=recipe.select(banks)
        self.assertTrue(all(c['parameters'][2]==0 for c in chosen))
    def test_real_source_axis_prior_does_not_change_silhouette_metrics(self):
        parent=json.loads(recipe.PARENT.read_text());phase=20
        ctx=recipe.fit.context(parent['rows'][phase]['source'])
        shift=np.array(recipe.LANDMARKS[phase][0])-ctx[0]
        p=np.array([0.,0.,0.,0.,0.,*shift]);reverse=p.copy();reverse[2]=180
        for x in (p,reverse):
            actual=recipe.evaluate(x,ctx,phase);baseline=recipe.fit.evaluate(x,ctx)
            for key in ('missing','covered','extra','source_pixels','bright_missing'):
                self.assertEqual(actual[key],baseline[key])
        ideal=recipe.evaluate(p,ctx,phase);flipped=recipe.evaluate(reverse,ctx,phase)
        self.assertAlmostEqual(ideal['body_center_error_pixels'],0)
        self.assertAlmostEqual(ideal['body_axis_error_degrees'],0)
        self.assertAlmostEqual(flipped['body_axis_error_degrees'],180)
        penalty=flipped['loss']-recipe.fit.evaluate(reverse,ctx)['loss']
        self.assertAlmostEqual(penalty,24/flipped['source_pixels'])

if __name__=='__main__':unittest.main()
