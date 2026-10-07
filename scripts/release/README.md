# Native release publisher

`publish_native_release.ts` runs directly in `actions/github-script@v9` using
Node 24's native type stripping. The action supplies the authenticated Octokit
client, workflow context and logging. All `@actions` imports are type-only, so
the release workflow needs neither `pnpm install` nor a compilation step.

For local checks with Node 24 or newer:

```sh
pnpm --dir scripts/release install --frozen-lockfile
pnpm --dir scripts/release verify
```

The tooling CI suite runs the same type check and Node tests. Keep TypeScript
syntax erasable; `tsconfig.json` checks this with `erasableSyntaxOnly` and
`noEmit`. Node itself does not read that configuration.

The publisher verifies local packages and both Velopack indexes, creates or
resumes a draft, uploads missing assets, downloads and verifies their hashes,
then publishes by release ID. It never replaces assets or modifies a published
release. The creation response supplies the draft ID and upload URL, avoiding
an immediate lookup through a releases list that may omit the new draft.

The game crate keeps an explicit native release base version (`0.1.0`), separate
from the unpublished workspace crates (`0.0.0`). Nightlies append the original
workflow timestamp. Package versions must satisfy Velopack's `>= 0.0.1` floor
and be strictly newer than every published Windows/Linux update-feed version;
changing build metadata alone does not count as an update. The same candidate
tag may be retried, with immutable assets verified by the publisher.

The workflow checks versions before compiling. Promotion repeats the check on
the actual packaged feeds and requires matching platform versions. A shared
publication concurrency group serializes promotion across branches and tags.
API errors and malformed feeds fail the check rather than removing the floor.

## Windows application manifest

The game build embeds `crates/robin_rs/windows/robin.manifest` as process
manifest resource 1. It declares `asInvoker` privileges and Windows 10/11
compatibility so Windows need not infer legacy compatibility behavior.
Resource compilation failures fail the build. Other target platforms skip
resource compilation.

The release workflow uses `pefile==2024.8.26` to check the staged executable
and both the root launcher and real executable in Velopack's portable ZIP:

```sh
python3 scripts/release/verify_windows_manifest.py path/to/robin.exe
python3 scripts/release/verify_windows_manifest.py path/to/Portable.zip \
  --main-exe "Robin Hood - The Legend of Sherwood.exe"
```

These checks inspect embedded PE resources, not loose manifest files. They
do not replace installing/updating under a standard Windows user and checking
that the install hook, first launch, and shortcuts work without elevation.
An existing user-selected "Run as administrator" compatibility setting may
still require removal on the affected machine.
