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
