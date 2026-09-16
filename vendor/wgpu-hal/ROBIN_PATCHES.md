# Local Vulkan presentation hooks

Base: crates.io wgpu-hal 30.0.1 (upstream licenses retained).

- Backport the Vulkan source changes from upstream PR #9847,
  commit 5430e2d2f3711516cd942628623feb364e0c02f6:
  https://github.com/gfx-rs/wgpu/pull/9847
  A null chain additionally clears the pending hook, so a caller can safely
  clean up if core validation rejects a present before HAL consumes the chain.
- Add `Surface::set_swapchain_create_flags`, preserving extra flags across
  reconfiguration. This is the narrow flag hook discussed in
  https://github.com/gfx-rs/wgpu/issues/2869#issuecomment-5544571905
  and addressed by the broader https://github.com/gfx-rs/wgpu/pull/10304.
  It is unsafe: callers validate the surface and enable the required features.

No rendering or pacing changes when these hooks are unused. On upgrade, replace
these hooks with upstream equivalents, remove this patch override, and rerun
native presentation profiling with Vulkan validation enabled, including resize.

- Enable optional `VK_KHR_get_surface_capabilities2` when available for
  application-side presentation capability queries.
