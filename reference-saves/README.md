# Original save fixtures

Keep this collection intact: it is input to recorder/corpus capture, not only
the saves named individually by Rust tests. The schema16 corpus ladder,
distributed capture, and onward controller pass the entire `reference-saves/`
directory to the Original recorder. `scripts/capture_parity_subset.sh` also
uses it as its default source.

The ordinary Windows v48 parser tests additionally require
`Savegame_SuN1Sh1nE/Profile_004/Savegame_005`; see `docs/TESTING.md`.
Deleting other trees solely because no test spells out their names would
silently reduce future capture coverage. Corpus migrations must preserve the
inputs and update the capture workflows together.
