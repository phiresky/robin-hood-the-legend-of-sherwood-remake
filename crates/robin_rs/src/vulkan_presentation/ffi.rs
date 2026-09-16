//! Minimal ABI bindings for Vulkan-Headers 1.4 VK_EXT_present_timing revision 3
//! and VK_KHR_present_id2 revision 1, absent from ash 0.38. Structs are transient
//! FFI storage, never serialized. Replace with ash bindings when released.
#![allow(non_snake_case)]
use ash::vk;
use std::ffi::c_void;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PhysicalDevicePresentTimingFeaturesEXT {
    pub sType: vk::StructureType,
    pub pNext: *mut c_void,
    pub presentTiming: u32,
    pub presentAtAbsoluteTime: u32,
    pub presentAtRelativeTime: u32,
}
impl Default for PhysicalDevicePresentTimingFeaturesEXT {
    fn default() -> Self {
        // SAFETY: Vulkan ABI scalar/handle/pointer fields all admit zero.
        let value: Self = unsafe { std::mem::zeroed() };
        Self {
            sType: vk::StructureType::from_raw(1000208000),
            ..value
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PhysicalDevicePresentId2FeaturesKHR {
    pub sType: vk::StructureType,
    pub pNext: *mut c_void,
    pub presentId2: u32,
}
impl Default for PhysicalDevicePresentId2FeaturesKHR {
    fn default() -> Self {
        // SAFETY: Vulkan ABI scalar/handle/pointer fields all admit zero.
        let value: Self = unsafe { std::mem::zeroed() };
        Self {
            sType: vk::StructureType::from_raw(1000479002),
            ..value
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PresentTimingSurfaceCapabilitiesEXT {
    pub sType: vk::StructureType,
    pub pNext: *mut c_void,
    pub presentTimingSupported: u32,
    pub presentAtAbsoluteTimeSupported: u32,
    pub presentAtRelativeTimeSupported: u32,
    pub presentStageQueries: u32,
}
impl Default for PresentTimingSurfaceCapabilitiesEXT {
    fn default() -> Self {
        // SAFETY: Vulkan ABI scalar/handle/pointer fields all admit zero.
        let value: Self = unsafe { std::mem::zeroed() };
        Self {
            sType: vk::StructureType::from_raw(1000208008),
            ..value
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SurfaceCapabilitiesPresentId2KHR {
    pub sType: vk::StructureType,
    pub pNext: *mut c_void,
    pub presentId2Supported: u32,
}
impl Default for SurfaceCapabilitiesPresentId2KHR {
    fn default() -> Self {
        // SAFETY: Vulkan ABI scalar/handle/pointer fields all admit zero.
        let value: Self = unsafe { std::mem::zeroed() };
        Self {
            sType: vk::StructureType::from_raw(1000479000),
            ..value
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SwapchainTimingPropertiesEXT {
    pub sType: vk::StructureType,
    pub pNext: *mut c_void,
    pub refreshDuration: u64,
    pub refreshInterval: u64,
}
impl Default for SwapchainTimingPropertiesEXT {
    fn default() -> Self {
        // SAFETY: Vulkan ABI scalar/handle/pointer fields all admit zero.
        let value: Self = unsafe { std::mem::zeroed() };
        Self {
            sType: vk::StructureType::from_raw(1000208001),
            ..value
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SwapchainTimeDomainPropertiesEXT {
    pub sType: vk::StructureType,
    pub pNext: *mut c_void,
    pub timeDomainCount: u32,
    pub pTimeDomains: *mut vk::TimeDomainKHR,
    pub pTimeDomainIds: *mut u64,
}
impl Default for SwapchainTimeDomainPropertiesEXT {
    fn default() -> Self {
        // SAFETY: Vulkan ABI scalar/handle/pointer fields all admit zero.
        let value: Self = unsafe { std::mem::zeroed() };
        Self {
            sType: vk::StructureType::from_raw(1000208002),
            ..value
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PastPresentationTimingInfoEXT {
    pub sType: vk::StructureType,
    pub pNext: *const c_void,
    pub flags: u32,
    pub swapchain: vk::SwapchainKHR,
}
impl Default for PastPresentationTimingInfoEXT {
    fn default() -> Self {
        // SAFETY: Vulkan ABI scalar/handle/pointer fields all admit zero.
        let value: Self = unsafe { std::mem::zeroed() };
        Self {
            sType: vk::StructureType::from_raw(1000208005),
            ..value
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PresentStageTimeEXT {
    pub stage: u32,
    pub time: u64,
}
impl Default for PresentStageTimeEXT {
    fn default() -> Self {
        // SAFETY: Vulkan ABI scalar/handle/pointer fields all admit zero.
        let value: Self = unsafe { std::mem::zeroed() };
        value
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PastPresentationTimingEXT {
    pub sType: vk::StructureType,
    pub pNext: *mut c_void,
    pub presentId: u64,
    pub targetTime: u64,
    pub presentStageCount: u32,
    pub pPresentStages: *mut PresentStageTimeEXT,
    pub timeDomain: vk::TimeDomainKHR,
    pub timeDomainId: u64,
    pub reportComplete: u32,
}
impl Default for PastPresentationTimingEXT {
    fn default() -> Self {
        // SAFETY: Vulkan ABI scalar/handle/pointer fields all admit zero.
        let value: Self = unsafe { std::mem::zeroed() };
        Self {
            sType: vk::StructureType::from_raw(1000208007),
            ..value
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PastPresentationTimingPropertiesEXT {
    pub sType: vk::StructureType,
    pub pNext: *mut c_void,
    pub timingPropertiesCounter: u64,
    pub timeDomainsCounter: u64,
    pub presentationTimingCount: u32,
    pub pPresentationTimings: *mut PastPresentationTimingEXT,
}
impl Default for PastPresentationTimingPropertiesEXT {
    fn default() -> Self {
        // SAFETY: Vulkan ABI scalar/handle/pointer fields all admit zero.
        let value: Self = unsafe { std::mem::zeroed() };
        Self {
            sType: vk::StructureType::from_raw(1000208006),
            ..value
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PresentTimingInfoEXT {
    pub sType: vk::StructureType,
    pub pNext: *const c_void,
    pub flags: u32,
    pub targetTime: u64,
    pub timeDomainId: u64,
    pub presentStageQueries: u32,
    pub targetTimeDomainPresentStage: u32,
}
impl Default for PresentTimingInfoEXT {
    fn default() -> Self {
        // SAFETY: Vulkan ABI scalar/handle/pointer fields all admit zero.
        let value: Self = unsafe { std::mem::zeroed() };
        Self {
            sType: vk::StructureType::from_raw(1000208004),
            ..value
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PresentTimingsInfoEXT {
    pub sType: vk::StructureType,
    pub pNext: *const c_void,
    pub swapchainCount: u32,
    pub pTimingInfos: *const PresentTimingInfoEXT,
}
impl Default for PresentTimingsInfoEXT {
    fn default() -> Self {
        // SAFETY: Vulkan ABI scalar/handle/pointer fields all admit zero.
        let value: Self = unsafe { std::mem::zeroed() };
        Self {
            sType: vk::StructureType::from_raw(1000208003),
            ..value
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PresentId2KHR {
    pub sType: vk::StructureType,
    pub pNext: *const c_void,
    pub swapchainCount: u32,
    pub pPresentIds: *const u64,
}
impl Default for PresentId2KHR {
    fn default() -> Self {
        // SAFETY: Vulkan ABI scalar/handle/pointer fields all admit zero.
        let value: Self = unsafe { std::mem::zeroed() };
        Self {
            sType: vk::StructureType::from_raw(1000479001),
            ..value
        }
    }
}

pub type SetQueueSize = unsafe extern "system" fn(vk::Device, vk::SwapchainKHR, u32) -> vk::Result;
pub type GetProperties = unsafe extern "system" fn(
    vk::Device,
    vk::SwapchainKHR,
    *mut SwapchainTimingPropertiesEXT,
    *mut u64,
) -> vk::Result;
pub type GetDomains = unsafe extern "system" fn(
    vk::Device,
    vk::SwapchainKHR,
    *mut SwapchainTimeDomainPropertiesEXT,
    *mut u64,
) -> vk::Result;
pub type GetPast = unsafe extern "system" fn(
    vk::Device,
    *const PastPresentationTimingInfoEXT,
    *mut PastPresentationTimingPropertiesEXT,
) -> vk::Result;
