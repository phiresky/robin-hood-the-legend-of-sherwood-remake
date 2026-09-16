//! Optional Vulkan presentation feedback. Enabled by ROBIN_GAMEPLAY_PROFILE.
//! All extension pointers stay inside synchronous Vulkan calls; no GPU waits.
mod ffi;
mod timeline;
use ash::vk;
use ffi::*;
use std::ffi::CStr;
use timeline::Timeline;
use wgpu::hal::api::Vulkan;

const TIMING: &CStr = c"VK_EXT_present_timing";
const ID2: &CStr = c"VK_KHR_present_id2";
const CALIBRATED: &CStr = c"VK_KHR_calibrated_timestamps";
const QUEUE_END: u32 = 1;
const PIXEL_OUT: u32 = 4;
const PIXEL_VISIBLE: u32 = 8;
const CAPACITY: u32 = 256;

fn check(result: vk::Result) -> Result<(), String> {
    if result == vk::Result::SUCCESS {
        Ok(())
    } else {
        Err(format!("Vulkan presentation timing: {result:?}"))
    }
}

// The queries use the physical device, instance and surface owned by wgpu.
fn stages(
    adapter: &wgpu::hal::vulkan::Adapter,
    surface: &wgpu::hal::vulkan::Surface,
) -> Result<Option<u32>, String> {
    surface_stages(
        adapter.shared_instance(),
        adapter.raw_physical_device(),
        surface,
    )
}
fn surface_stages(
    instance: &wgpu::hal::vulkan::InstanceShared,
    physical: vk::PhysicalDevice,
    surface: &wgpu::hal::vulkan::Surface,
) -> Result<Option<u32>, String> {
    if !instance
        .extensions()
        .contains(&ash::khr::get_surface_capabilities2::NAME)
    {
        return Ok(None);
    }
    // SAFETY: borrowed live wgpu surface, handle is not retained.
    let Some(raw) = (unsafe { surface.raw_native_handle() }) else {
        return Ok(None);
    };
    let loader = ash::khr::get_surface_capabilities2::Instance::new(
        instance.entry(),
        instance.raw_instance(),
    );
    let mut ids = SurfaceCapabilitiesPresentId2KHR::default();
    let mut timing = PresentTimingSurfaceCapabilitiesEXT {
        pNext: (&mut ids as *mut SurfaceCapabilitiesPresentId2KHR).cast(),
        ..Default::default()
    };
    let mut caps = vk::SurfaceCapabilities2KHR {
        p_next: (&mut timing as *mut PresentTimingSurfaceCapabilitiesEXT).cast(),
        ..Default::default()
    };
    // SAFETY: matching physical device/surface; valid output chain lives through call.
    unsafe {
        loader.get_physical_device_surface_capabilities2(
            physical,
            &vk::PhysicalDeviceSurfaceInfo2KHR::default().surface(raw),
            &mut caps,
        )
    }
    .map_err(|e| format!("surface timing capabilities: {e:?}"))?;
    Ok(
        (timing.presentTimingSupported != 0 && ids.presentId2Supported != 0)
            .then_some(timing.presentStageQueries),
    )
}

pub(crate) fn request_device(
    adapter: &wgpu::Adapter,
    surface: &wgpu::Surface<'_>,
    desc: &wgpu::DeviceDescriptor<'_>,
) -> Result<Option<(wgpu::Device, wgpu::Queue)>, String> {
    if !crate::presentation_timing::enabled() {
        return Ok(None);
    }
    // SAFETY: handles are borrowed for this call and never destroyed externally.
    let (Some(a), Some(s)) = (unsafe { adapter.as_hal::<Vulkan>() }, unsafe {
        surface.as_hal::<Vulkan>()
    }) else {
        return Ok(None);
    };
    let instance = a.shared_instance().raw_instance();
    let physical = a.raw_physical_device();
    let extensions = unsafe { instance.enumerate_device_extension_properties(physical) }
        .map_err(|e| format!("enumerate timing extensions: {e:?}"))?;
    if [TIMING, ID2, CALIBRATED].iter().any(|name| {
        !extensions
            .iter()
            .any(|e| e.extension_name_as_c_str() == Ok(*name))
    }) {
        tracing::info!(target: "presentation_perf", "Vulkan presentation feedback unavailable: required extensions missing");
        return Ok(None);
    }
    if stages(&a, &s)?.is_none() {
        return Ok(None);
    }
    let mut ids = PhysicalDevicePresentId2FeaturesKHR::default();
    let mut timing = PhysicalDevicePresentTimingFeaturesEXT {
        pNext: (&mut ids as *mut PhysicalDevicePresentId2FeaturesKHR).cast(),
        ..Default::default()
    };
    let mut features = vk::PhysicalDeviceFeatures2 {
        p_next: (&mut timing as *mut PhysicalDevicePresentTimingFeaturesEXT).cast(),
        ..Default::default()
    };
    let properties = ash::khr::get_physical_device_properties2::Instance::new(
        a.shared_instance().entry(),
        instance,
    );
    unsafe { properties.get_physical_device_features2(physical, &mut features) };
    if timing.presentTiming == 0 || ids.presentId2 == 0 {
        return Ok(None);
    }
    // Enable feedback only: do not request scheduling features or alter pacing.
    timing.presentAtAbsoluteTime = 0;
    timing.presentAtRelativeTime = 0;
    let ids_ptr = &mut ids as *mut PhysicalDevicePresentId2FeaturesKHR;
    let timing_ptr = (&mut timing as *mut PhysicalDevicePresentTimingFeaturesEXT).cast();
    // SAFETY: extensions/features were queried on this adapter. Stack chains live
    // until open_with_callback returns, and existing wgpu features are preserved.
    let opened = unsafe {
        a.open_with_callback(
            desc.required_features,
            &desc.required_limits,
            &desc.memory_hints,
            Some(Box::new(move |args| {
                for extension in [TIMING, ID2, CALIBRATED] {
                    if !args.extensions.contains(&extension) {
                        args.extensions.push(extension);
                    }
                }
                (*ids_ptr).pNext = args.create_info.p_next.cast_mut();
                // Keep the captured output feature storage borrowed until device creation.
                args.create_info.p_next = timing_ptr;
            })),
        )
    }
    .map_err(|e| format!("open Vulkan timing device: {e:?}"))?;
    // SAFETY: device was opened from this exact adapter with descriptor features.
    unsafe { adapter.create_device_from_hal::<Vulkan>(opened, desc) }
        .map(Some)
        .map_err(|e| format!("wrap Vulkan timing device: {e}"))
}

pub(crate) fn configure(
    surface: &wgpu::Surface<'_>,
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
) {
    // SAFETY: only optional native configuration flags are changed before configure.
    if let (Some(d), Some(s)) = (unsafe { device.as_hal::<Vulkan>() }, unsafe {
        surface.as_hal::<Vulkan>()
    }) {
        if d.enabled_device_extensions().contains(&TIMING) {
            match surface_stages(d.shared_instance(), d.raw_physical_device(), &s) {
                Ok(supported) => unsafe {
                    s.set_swapchain_create_flags(vk::SwapchainCreateFlagsKHR::from_raw(
                        if supported.is_some() { 0x200 | 0x40 } else { 0 },
                    ))
                },
                Err(e) => {
                    tracing::warn!(target: "presentation_perf", error = %e, "surface timing disabled");
                    unsafe { s.set_swapchain_create_flags(vk::SwapchainCreateFlagsKHR::empty()) };
                }
            }
        }
    }
    surface.configure(device, config);
}

// Runtime resources, not persistent state: Vulkan handles/function pointers must
// never be serialized or outlive the wgpu device/surface passed to present().
#[derive(Default)]
pub(crate) struct Feedback {
    state: Option<State>,
    initialized: bool,
    generation: u64,
}
struct State {
    raw: vk::SwapchainKHR,
    get_properties: GetProperties,
    get_domains: GetDomains,
    get_past: GetPast,
    stage: u32,
    domain_id: u64,
    next_id: u64,
    pending: std::collections::BTreeSet<u64>,
    timing_counter: u64,
    domain_counter: u64,
    refresh: u64,
    refresh_interval: u64,
    timeline: Timeline,
}

impl Feedback {
    pub(crate) fn reset(&mut self) {
        if let Some(s) = &self.state {
            tracing::debug!(target: "presentation_perf", pending = s.pending.len(), "presentation feedback reset on surface reconfiguration");
        }
        self.state = None;
        self.initialized = false;
        self.generation += 1;
    }
    pub(crate) fn present(
        &mut self,
        surface: &wgpu::Surface<'_>,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: wgpu::SurfaceTexture,
    ) {
        if !crate::presentation_timing::enabled() {
            queue.present(frame);
            return;
        }
        let _span = tracing::info_span!(target: "presentation_perf", "vulkan_display", swapchain_generation = self.generation).entered();
        // SAFETY: borrowed HAL objects are not destroyed/mutated outside the
        // documented extension hooks. SharedSurface serializes configure/present.
        let (Some(d), Some(s)) = (unsafe { device.as_hal::<Vulkan>() }, unsafe {
            surface.as_hal::<Vulkan>()
        }) else {
            queue.present(frame);
            return;
        };
        if !self.initialized {
            self.initialized = true;
            match State::new(&d, &s) {
                Ok(state) => self.state = state,
                Err(e) => {
                    tracing::warn!(target: "presentation_perf", error = %e, "Vulkan presentation feedback unavailable")
                }
            }
        }
        let Some(state) = &mut self.state else {
            queue.present(frame);
            return;
        };
        if let Err(e) = state.poll(d.raw_device().handle()) {
            tracing::warn!(target: "presentation_perf", error = %e, "Vulkan presentation feedback disabled");
            self.state = None;
            queue.present(frame);
            return;
        }
        if state.pending.len() >= CAPACITY as usize {
            // Never fill the driver's query queue: that would fail presentation.
            tracing::warn!(target: "presentation_perf", pending = state.pending.len(), "presentation feedback queue full; presenting without a timing query");
            state.timeline.reset();
            state.next_id += 1;
            queue.present(frame);
            return;
        }
        let id = state.next_id;
        state.next_id += 1;
        state.pending.insert(id);
        tracing::debug!(target: "presentation_perf", present_id = id,
            submitted_at_us = crate::window::process_uptime_us(),
            "Vulkan presentation submitted");
        let timing = PresentTimingInfoEXT {
            timeDomainId: state.domain_id,
            presentStageQueries: state.stage | QUEUE_END,
            targetTimeDomainPresentStage: state.stage,
            ..Default::default()
        };
        let mut ids = PresentId2KHR {
            swapchainCount: 1,
            pPresentIds: &id,
            ..Default::default()
        };
        let mut times = PresentTimingsInfoEXT {
            pNext: (&mut ids as *mut PresentId2KHR).cast(),
            swapchainCount: 1,
            pTimingInfos: &timing,
            ..Default::default()
        };
        // SAFETY: all chain storage remains live and unaliased through present.
        // No reconfigure or competing presentation can intervene under the surface lock.
        unsafe { s.set_next_present_chain((&mut times as *mut PresentTimingsInfoEXT).cast()) };
        queue.present(frame);
        // Clear even if core rejected present before entering HAL; no dangling chain.
        unsafe { s.set_next_present_chain(std::ptr::null_mut()) };
    }
}

impl State {
    fn new(
        d: &wgpu::hal::vulkan::Device,
        s: &wgpu::hal::vulkan::Surface,
    ) -> Result<Option<Self>, String> {
        if !d.enabled_device_extensions().contains(&TIMING) {
            return Ok(None);
        }
        let Some(supported) = surface_stages(d.shared_instance(), d.raw_physical_device(), s)?
        else {
            return Ok(None);
        };
        let stage = if supported & PIXEL_VISIBLE != 0 {
            PIXEL_VISIBLE
        } else if supported & PIXEL_OUT != 0 {
            PIXEL_OUT
        } else {
            tracing::info!(target: "presentation_perf", supported_stages = supported, "Vulkan timing lacks a display stage; keeping CPU diagnostics only");
            return Ok(None);
        };
        let Some(raw) = s.raw_native_swapchain() else {
            return Ok(None);
        };
        let instance = d.shared_instance().raw_instance();
        let device = d.raw_device().handle();
        macro_rules! load {
            ($name:literal, $ty:ty) => {{
                // SAFETY: extension was enabled; function ABI matches Vulkan headers.
                let f = unsafe { instance.get_device_proc_addr(device, $name.as_ptr()) }
                    .ok_or_else(|| format!("missing Vulkan entry point {:?}", $name))?;
                unsafe { std::mem::transmute::<unsafe extern "system" fn(), $ty>(f) }
            }};
        }
        let set_size = load!(c"vkSetSwapchainPresentTimingQueueSizeEXT", SetQueueSize);
        let mut state = Self {
            raw,
            get_properties: load!(c"vkGetSwapchainTimingPropertiesEXT", GetProperties),
            get_domains: load!(c"vkGetSwapchainTimeDomainPropertiesEXT", GetDomains),
            get_past: load!(c"vkGetPastPresentationTimingEXT", GetPast),
            stage,
            domain_id: 0,
            next_id: 1,
            pending: Default::default(),
            timing_counter: 0,
            domain_counter: 0,
            refresh: 0,
            refresh_interval: 0,
            timeline: Timeline::default(),
        };
        unsafe {
            check(set_size(device, raw, CAPACITY))?;
        }
        state.update_domains(device)?;
        state.update_properties(device)?;
        tracing::info!(target: "presentation_perf", stage = state.stage_name(), refresh_ns = state.refresh,
            refresh_interval_ns = state.refresh_interval, time_domain_id = state.domain_id,
            "Vulkan display presentation feedback enabled");
        Ok(Some(state))
    }
    fn stage_name(&self) -> &'static str {
        if self.stage == PIXEL_VISIBLE {
            "first_pixel_visible"
        } else {
            "first_pixel_out"
        }
    }
    fn update_domains(&mut self, device: vk::Device) -> Result<(), String> {
        let mut props = SwapchainTimeDomainPropertiesEXT::default();
        let mut counter = 0;
        // SAFETY: two-call Vulkan enumeration, bounded output storage kept live.
        unsafe {
            check((self.get_domains)(
                device,
                self.raw,
                &mut props,
                &mut counter,
            ))?;
        }
        if props.timeDomainCount == 0 {
            return Err("no presentation time domains".into());
        }
        let mut domains = vec![vk::TimeDomainKHR::DEVICE; props.timeDomainCount as usize];
        let mut ids = vec![0u64; domains.len()];
        props.pTimeDomains = domains.as_mut_ptr();
        props.pTimeDomainIds = ids.as_mut_ptr();
        unsafe {
            check((self.get_domains)(
                device,
                self.raw,
                &mut props,
                &mut counter,
            ))?;
        }
        if props.timeDomainCount == 0 {
            return Err("presentation time domains disappeared".into());
        }
        // Prefer monotonic timestamps; otherwise intervals remain valid within
        // one reported time domain, with no CPU-clock latency claims.
        let index = domains[..props.timeDomainCount as usize]
            .iter()
            .position(|d| *d == vk::TimeDomainKHR::CLOCK_MONOTONIC)
            .unwrap_or(0);
        self.domain_id = ids[index];
        self.domain_counter = counter;
        self.timeline.reset();
        Ok(())
    }
    fn update_properties(&mut self, device: vk::Device) -> Result<(), String> {
        let mut props = SwapchainTimingPropertiesEXT::default();
        let mut counter = 0;
        unsafe {
            check((self.get_properties)(
                device,
                self.raw,
                &mut props,
                &mut counter,
            ))?;
        }
        if counter != self.timing_counter
            || props.refreshDuration != self.refresh
            || props.refreshInterval != self.refresh_interval
        {
            self.timeline.reset();
        }
        self.timing_counter = counter;
        self.refresh = props.refreshDuration;
        self.refresh_interval = props.refreshInterval;
        Ok(())
    }
    fn poll(&mut self, device: vk::Device) -> Result<(), String> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let info = PastPresentationTimingInfoEXT {
            swapchain: self.raw,
            ..Default::default()
        };
        let mut stages = [[PresentStageTimeEXT::default(); 2]; CAPACITY as usize];
        let mut results = [PastPresentationTimingEXT::default(); CAPACITY as usize];
        for (result, storage) in results.iter_mut().zip(stages.iter_mut()) {
            result.presentStageCount = 2;
            result.pPresentStages = storage.as_mut_ptr();
        }
        let mut props = PastPresentationTimingPropertiesEXT {
            presentationTimingCount: CAPACITY,
            pPresentationTimings: results.as_mut_ptr(),
            ..Default::default()
        };
        // SAFETY: output buffers have requested capacity, each entry's stage
        // buffer holds both queried stages. No partial/out-of-order results requested.
        let status = unsafe { (self.get_past)(device, &info, &mut props) };
        if status != vk::Result::SUCCESS && status != vk::Result::INCOMPLETE {
            check(status)?;
        }
        let domains_changed = props.timeDomainsCounter != self.domain_counter;
        let timing_changed = props.timingPropertiesCounter != self.timing_counter;
        if domains_changed || timing_changed {
            self.timeline.reset();
        }
        self.update_properties(device)?;
        for (result, values) in results[..props.presentationTimingCount as usize]
            .iter()
            .zip(stages.iter())
        {
            if result.reportComplete == 0 {
                return Err("incomplete presentation feedback without partial-results flag".into());
            }
            if !self.pending.remove(&result.presentId) {
                return Err(format!(
                    "unknown presentation feedback id {}",
                    result.presentId
                ));
            }
            let time = values[..result.presentStageCount as usize]
                .iter()
                .find(|v| v.stage == self.stage)
                .map(|v| v.time)
                .unwrap_or(0);
            let ready = values[..result.presentStageCount as usize]
                .iter()
                .find(|v| v.stage == QUEUE_END)
                .map(|v| v.time)
                .unwrap_or(0);
            self.timeline.record(
                result.presentId,
                time,
                ready,
                result.timeDomain.as_raw(),
                result.timeDomainId,
                self.stage_name(),
                self.refresh,
                if timing_changed || domains_changed {
                    0
                } else {
                    self.refresh_interval
                },
            );
        }
        if domains_changed {
            self.update_domains(device)?;
        }
        if domains_changed || timing_changed {
            self.timeline.reset();
        }
        Ok(())
    }
}
