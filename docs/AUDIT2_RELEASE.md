# Audit 2: release-tool boundaries

## Implemented

The release APIs and document schemas are unchanged. This pass separates the
authority-owning pieces from pure policy without introducing a generic filesystem
abstraction or changing deployment scripts.

| Boundary | Responsibility |
| --- | --- |
| `publication_v3/topology.rs` | Pure expected file/directory topology, canonical lock authoring, materialized inventory, path grammar. No filesystem I/O. |
| `publication_v3/topology_inventory.rs` | Compare independently derived topology against descriptor-backed inventory; seal modes and sync the exact open files/directories. |
| `publication_v3/persistence.rs` | Pinned private staging, no-replace installation, post-rename reconciliation, durability outcomes, guarded failed-stage cleanup. |
| `vps_release_v2/activation_lock.rs` | Own activation lock descriptors and inode identity; acquire or inherit the exact locked open-file-description. Descriptor fields are private to this module. |
| `vps_release_v2/host_policy.rs` | Reviewed canonical template bytes, exact deployment-kit inventories, pure host-file policy. |

Host-file policy now consumes one bounded byte snapshot. Previously the wrapper
read the same path once for UTF-8/placeholder validation and again for template
validation. One read binds both checks to the same bytes; this does not replace
the separate artifact and source-authority validation performed by assembly.

Public activation functions and error types are re-exported at their previous
paths. Existing Rust structs were relocated, not replaced with new serialized
authority objects. A lock still cannot be constructed from serialized metadata.

## Security and compatibility invariants

- No canonical document, artifact inventory, template bytes, mode policy, or
  version number was changed. Relative `include_str!` paths were adjusted solely
  for the child module location.
- Publication staging and candidates remain descriptor-pinned. The exact tree
  is synced before no-replace rename. Actual source/output inode identities are
  checked even when rename reports an error.
- Known installed output with failed parent sync remains distinct from uncertain
  persistence. Neither authorizes ordinary failed-stage cleanup. Parent/output
  rebinding and exact inventory are checked after sync as before.
- Source projection still consumes a duplicated inherited descriptor and checks
  canonical bytes, out-of-band digest, owner, links, mode, size, and identity.
- Deploy still validates inherited authorities before acquiring activation
  exclusion. Rollback does not acquire source-consumption or plan authority.
- Native Linux guards and unsupported-platform behavior were retained.
- No deployment, publication, promotion, source consumption, or shell-script
  modification was performed for this refactor.

The persistence and activation implementations were mechanically compared with
the starting revision after removing visibility/formatting differences. Their
operational bodies are unchanged. This comparison is not a substitute for tests.

## Validation

Completed in the implementation worktree:

- `cargo fmt --all`.
- `git diff --check`.
- Static normalized comparison of persistence/activation implementation bodies.

Six regression tests added:

- Lock bytes/digest independent of topology registration order, with exact
  directory/file modes.
- Artifact-media normalization and executable mode preservation.
- Noncanonical paths and lock self-reference rejected.
- Exact host templates accepted and altered bytes rejected without filesystem
  inputs.
- Invalid UTF-8, unresolved template content, and wrong release commit rejected.
- An inherited lock retains exclusion after the original owner drops, releasing
  it only when the final owner drops.

Existing descriptor-substitution, symlink, fault-injected rename/sync, uncertain
persistence, source-consumption recovery, runtime-fence, canonical inventory,
and template-policy tests remain in their existing parent test modules.

Integration acceptance passed at `9f0b52c2a`: explicit `robin_manifest_tool`
suite, 150 library tests, binary test target and doctests, including the six new
regressions and retained descriptor/fault-injection/recovery tests. No deployment
or publication command ran. The implementation worktree itself did not duplicate
the combined build. See [final acceptance](AUDIT2_PLAN.md#final-acceptance).

## Integration seam and remaining scope

An active performance branch also touches `publication_v3.rs`. No performance
changes were imported. Preserve its admitted-profile/resource-loading changes
when merging; the extraction must not silently restore older parent-file code.

The parent modules continue to orchestrate admitted document closure, assembly,
source consumption, and runtime-fence recovery. Those workflows were not folded
into new generic repositories or pathname helpers. TODO: if they are extracted
later, retain their pinned-descriptor inputs and current crash-recovery phase
boundaries rather than expanding the pure topology/template modules to own I/O.
