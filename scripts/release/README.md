# Native release publisher

`publish_native_release.ts` runs directly in `actions/github-script@v9` using
Node 24's native type stripping. The action supplies the authenticated Octokit
client, workflow context and logging. All `@actions` imports are type-only, so
the release workflow needs neither `npm install` nor a compilation step.

For local checks with Node 24 or newer:

```sh
npm ci --prefix scripts/release
npm --prefix scripts/release run verify
```

The tooling CI suite runs the same type check and Node tests. Keep TypeScript
syntax erasable; `tsconfig.json` checks this with `erasableSyntaxOnly` and
`noEmit`. Node itself does not read that configuration.

The publisher verifies local packages and both Velopack indexes, creates or
resumes a draft, uploads missing assets, downloads and verifies their hashes,
then publishes by release ID. It never replaces assets or modifies a published
release. The creation response supplies the draft ID and upload URL, avoiding
an immediate lookup through a releases list that may omit the new draft.
