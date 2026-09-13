# Original save fixtures

Only `Savegame_SuN1Sh1nE/` is tracked. The ordinary Windows v48 parser tests in
`robin_engine` (`legacy_save/engine.rs`, `legacy_save/campaign.rs`) require
`Savegame_SuN1Sh1nE/Profile_004/Savegame_005`; see `docs/TESTING.md`.

The other nine save collections (~210 MB) were removed from the tracked tree
because no test or document names them. They were inputs to Original-recorder
corpus capture: the schema16 onward controller,
`scripts/parity-campaigns/schema16-20260824/` supervisors, and
`scripts/capture_parity_subset.sh` pass this whole directory (or use it as the
default `SAVE_DIR`). Before re-running such a capture, restore them from the
last commit that contained them (the parent of the commit that removed them):

```sh
git log --diff-filter=D --format=%H -1 -- reference-saves/Savegame_linux3
git checkout <that-commit>^ -- reference-saves/Savegame_Cyrdach \
    reference-saves/Savegame_Cyrdach2 reference-saves/Savegame_Cyrdach_moje \
    reference-saves/Savegame_linux reference-saves/Savegame_linux2 \
    reference-saves/Savegame_linux3 reference-saves/Savegame_Nescafe \
    reference-saves/Savegame_nicouzouf reference-saves/Savegame_randomguy
```

Alternatively pass an explicit external save directory to those drivers.
