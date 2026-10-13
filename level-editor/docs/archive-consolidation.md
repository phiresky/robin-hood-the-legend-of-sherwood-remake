# Archived duplicate consolidation

`refinement/consolidate_archived_duplicates.py` consolidates exact byte-identical
archive files while preserving all consumer filenames, relative paths and hashes.
It retains one inode per eligible group and replaces the other physical copies
with read-only hardlinks. These are regular files, not compatibility symlinks.

This resolves the two consumer constraints found in the storage review: Blender
relative textures still resolve from the original directory, and library-index
entries still resolve from their snapshot root. Hash-bound approval and rollback
records retain their original bytes. No resolver or fallback is needed for reads.

Read-only permissions prevent an in-place archive write from modifying other
snapshots. For an edit or replay that writes an existing archive file, first use:

```sh
python3 refinement/consolidate_archived_duplicates.py edit work/needs-review/<job>/<file>
```

The edit command copies and verifies the bytes, atomically installs an independent
inode, then permits owner writes. It refuses symlink paths and detects concurrent
changes. Never use `chmod +w` on a consolidated file. A normal copy into a new
active workspace is also independent; if that copy preserves read-only
permissions, make only the independent copy writable. Do not use `cp -l` for
editable copies.

The plan excludes files changed since inventory, symlinked paths, and groups
with hardlinks outside the audited set. Those external links may belong to
active or published assets; their permissions and contents must remain unchanged.
Before consolidation, the tool rehashes every distinct input inode. Its append-only
journal records each replacement and the physical allocation released when the
last old link disappears. Each group is rehashed after consolidation and every
consumer path is checked to be a read-only link to those verified bytes.

The 2026-10-13 plan and execution journal are retained locally under
`work/storage-maintenance/dedup-20261013/`. They identify every path and hash,
original permissions, retained source, excluded group, and verification result.
This operation addresses duplicate storage only. Unique obsolete revisions and
raw-output provenance still require the classification described in
[the storage audit](needs-review-storage-audit.md).

The completed run consolidated 1447 redundant copies across 888 groups,
reclaiming 88.21 GiB of allocated file storage. All 2,343 original paths remain
readable with their original bytes. Thirty-four groups with external hardlinks
were excluded. The focused consolidation, texture-decision and relocation test
suites passed (19 tests). Final path/inode/permission checks are recorded in
`consumer-verification.json` alongside the execution receipt.
