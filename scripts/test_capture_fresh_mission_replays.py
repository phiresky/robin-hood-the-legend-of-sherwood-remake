import json
from pathlib import Path
import tempfile
import unittest
from capture_fresh_mission_replays import mission_inventory, validate_header, validate_extent, FOOTER

class CaptureGates(unittest.TestCase):
    def test_inventory_covers_files_and_rejects_unknown_team(self):
        with tempfile.TemporaryDirectory() as d:
            data=Path(d); levels=data/'Data/Levels'; levels.mkdir(parents=True)
            for suffix in ['rhm','scb']: (levels/f'Rescue.{suffix}').touch()
            mission=dict(mission_filename='Rescue',proto_level_filename='Nottingham',required_character_indices=[6])
            profile=dict(missions={'Rescue':mission,'pseudo':dict(mission,mission_filename='Impossible')})
            missions,excluded=mission_inventory(profile,data)
            self.assertEqual(missions,[dict(mission='Rescue',proto='Nottingham',team='M')])
            self.assertEqual(excluded,['pseudo'])
            mission['required_character_indices']=[9]
            with self.assertRaises(ValueError): mission_inventory(profile,data)

    def test_rejects_save_start_wrong_seed_and_wrong_difficulty(self):
        run=dict(mission='M',proto='P',seed=1,input_seed=2,difficulty='EASY')
        header=dict(type='header',schema=16,start_state='mission_start',initial_frame=0,mission='M',proto_level='p',rng_seed=1,random_input_seed=2,simulation_hz=25,campaign={'version':1},sim_config={'difficulty':'easy'})
        validate_header(header,run)
        for change in [dict(start_state='loaded_save'),dict(initial_save={}),dict(rng_seed=2),dict(sim_config={'difficulty':'hard'}),dict(campaign=None),dict(initial_frame=1)]:
            with self.subTest(change=change),self.assertRaises(ValueError): validate_header(dict(header,**change),run)

    def test_early_exit_is_valid_but_invalid_extent_cannot_publish(self):
        with tempfile.TemporaryDirectory() as d:
            path=Path(d)/'trace'
            for count,final in [(1499,1500),(1500,1501),(1501,1501)]:
                path.write_bytes(FOOTER.pack(b'RHPRTRACEFOOTER!',68,count,final))
                with self.assertRaises(ValueError): validate_extent(path,1500)
            path.write_bytes(FOOTER.pack(b'RHPRTRACEFOOTER!',68,1500,1500))
            validate_extent(path,1500)
            for count in [0,1499]:
                path.write_bytes(FOOTER.pack(b'RHPRTRACEFOOTER!',68,count,count))
                self.assertEqual(validate_extent(path,1500),count)

if __name__=='__main__': unittest.main()
