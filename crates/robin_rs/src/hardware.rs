//! CPU / hardware feature detection.
//!
//! Uses `std::arch::is_x86_feature_detected!` on x86 targets and
//! `libc::uname` for the machine identifier.

use std::ffi::{CStr, CString};

// ---------------------------------------------------------------------------
// Processor type enum
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessorType {
    Non586 = 0,
    Pentium = 1,
    PentiumMmx = 2,
    PentiumPro = 3,
    PentiumII = 4,
    PentiumIII = 5,
    Celeron = 6,
    CeleronA = 7,
    AmdK5 = 8,
    AmdK6 = 9,
    AmdK6II = 10,
    AmdK6III = 11,
    AmdK7 = 12,
    Cyrix6x86 = 13,
    CyrixMediaGx = 14,
    Cyrix6x86Mx = 15,
    CyrixGxm = 16,
    PowerPcGeneric = 17,
    PowerPcG4G5 = 18,
    Unknown = 19,
}

// ---------------------------------------------------------------------------
// Hardware info
// ---------------------------------------------------------------------------

pub struct Hardware {
    processor_identifier: CString,
    processor_type: ProcessorType,
    processor_speed: u16,

    has_mmx: bool,
    has_3dnow: bool,
    has_ext_3dnow: bool,
    has_sse: bool,
    has_fpu: bool,
    has_altivec: bool,
    is_multiprocessor: bool,

    cache_l1_data: i16,
    cache_l1_code: i16,
    cache_l2: i16,
}

impl Hardware {
    /// Detect hardware features.
    pub fn detect() -> Self {
        // Detect SIMD features
        let has_mmx = Self::detect_mmx();
        let has_sse = Self::detect_sse();
        let has_3dnow = Self::detect_3dnow();
        let has_ext_3dnow = has_3dnow;
        let has_altivec = Self::detect_altivec();

        // Get machine name from uname
        let machine_name = Self::get_machine_name();

        // Build identifier and determine processor type
        let mut identifier = machine_name;
        let proc_type;

        if has_mmx || has_sse || has_3dnow {
            proc_type = if has_ext_3dnow {
                ProcessorType::AmdK7
            } else if has_3dnow {
                ProcessorType::AmdK5
            } else if has_sse {
                ProcessorType::PentiumIII
            } else {
                ProcessorType::Pentium
            };

            if identifier.is_empty() {
                identifier = "Generic x86".to_string();
            }

            if has_mmx {
                identifier.push_str(" MMX");
            }

            if has_sse {
                identifier.push_str(" SSE");
            }

            if has_3dnow {
                identifier.push_str(" 3DNow");
            }

            if has_ext_3dnow {
                identifier.push_str(" 3DNow2");
            }
        } else {
            if identifier.is_empty() {
                identifier = "Generic PowerPC".to_string();
            }

            if has_altivec {
                identifier.push_str(" G4/G5 (ALTIVEC)");
                proc_type = ProcessorType::PowerPcG4G5;
            } else {
                proc_type = ProcessorType::PowerPcGeneric;
            }
        }

        Hardware {
            processor_identifier: CString::new(identifier).unwrap_or_default(),
            processor_type: proc_type,
            processor_speed: 0,
            has_mmx,
            has_3dnow,
            has_ext_3dnow,
            has_sse,
            has_fpu: true, // all modern CPUs have FPU
            has_altivec,
            is_multiprocessor: false,
            cache_l1_data: 0,
            cache_l1_code: 0,
            cache_l2: 0,
        }
    }

    // -- Feature detection helpers --

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    fn detect_mmx() -> bool {
        is_x86_feature_detected!("mmx")
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    fn detect_mmx() -> bool {
        false
    }

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    fn detect_sse() -> bool {
        is_x86_feature_detected!("sse")
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    fn detect_sse() -> bool {
        false
    }

    // 3DNow! detection is not available via std::arch on stable Rust,
    // so we default to false.
    fn detect_3dnow() -> bool {
        false
    }

    #[cfg(target_arch = "powerpc")]
    fn detect_altivec() -> bool {
        // On PowerPC we'd check for Altivec; not easily available in
        // stable Rust, so default false.
        false
    }
    #[cfg(not(target_arch = "powerpc"))]
    fn detect_altivec() -> bool {
        false
    }

    /// Get machine architecture name.
    ///
    /// Uses `std::env::consts::ARCH` which returns the same value as
    /// `uname().machine` (e.g. "x86_64") without any unsafe FFI.
    fn get_machine_name() -> String {
        std::env::consts::ARCH.to_string()
    }

    // -- Accessors --

    pub fn processor_identifier(&self) -> &CStr {
        &self.processor_identifier
    }

    pub fn processor_type(&self) -> ProcessorType {
        self.processor_type
    }

    pub fn processor_speed(&self) -> u16 {
        self.processor_speed
    }

    pub fn has_mmx(&self) -> bool {
        self.has_mmx
    }

    pub fn has_sse(&self) -> bool {
        self.has_sse
    }

    // -- Memory queries (stub values) --

    pub fn physical_memory_mb(&self) -> u16 {
        512
    }

    pub fn free_physical_memory_mb(&self) -> u16 {
        256
    }

    pub fn page_memory_mb(&self) -> u16 {
        256
    }

    pub fn free_page_memory_mb(&self) -> u16 {
        128
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_creates_valid_instance() {
        let hw = Hardware::detect();
        // On any modern x86 machine, MMX and SSE should be present
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        {
            assert!(hw.has_mmx());
            assert!(hw.has_sse());
            // With MMX+SSE detected, processor type should be PentiumIII
            // (SSE sets it last in the chain, unless 3DNow overrides)
            assert_ne!(hw.processor_type(), ProcessorType::Unknown);
        }
        // Identifier should be non-empty
        assert!(
            !hw.processor_identifier().to_bytes().is_empty(),
            "processor identifier should not be empty"
        );
    }

    #[test]
    fn processor_type_enum_values_match_c() {
        // Verify the explicit enum discriminants stay stable
        assert_eq!(ProcessorType::Non586 as i32, 0);
        assert_eq!(ProcessorType::Pentium as i32, 1);
        assert_eq!(ProcessorType::PentiumMmx as i32, 2);
        assert_eq!(ProcessorType::PentiumPro as i32, 3);
        assert_eq!(ProcessorType::PentiumII as i32, 4);
        assert_eq!(ProcessorType::PentiumIII as i32, 5);
        assert_eq!(ProcessorType::Celeron as i32, 6);
        assert_eq!(ProcessorType::CeleronA as i32, 7);
        assert_eq!(ProcessorType::AmdK5 as i32, 8);
        assert_eq!(ProcessorType::AmdK6 as i32, 9);
        assert_eq!(ProcessorType::AmdK6II as i32, 10);
        assert_eq!(ProcessorType::AmdK6III as i32, 11);
        assert_eq!(ProcessorType::AmdK7 as i32, 12);
        assert_eq!(ProcessorType::Cyrix6x86 as i32, 13);
        assert_eq!(ProcessorType::CyrixMediaGx as i32, 14);
        assert_eq!(ProcessorType::Cyrix6x86Mx as i32, 15);
        assert_eq!(ProcessorType::CyrixGxm as i32, 16);
        assert_eq!(ProcessorType::PowerPcGeneric as i32, 17);
        assert_eq!(ProcessorType::PowerPcG4G5 as i32, 18);
        assert_eq!(ProcessorType::Unknown as i32, 19);
    }

    #[test]
    fn memory_stubs_return_expected_values() {
        let hw = Hardware::detect();
        assert_eq!(hw.physical_memory_mb(), 512);
        assert_eq!(hw.free_physical_memory_mb(), 256);
        assert_eq!(hw.page_memory_mb(), 256);
        assert_eq!(hw.free_page_memory_mb(), 128);
    }

    #[test]
    fn detect_roundtrip() {
        let hw = Hardware::detect();
        assert!(!hw.processor_identifier.to_bytes().is_empty());
        assert!((hw.processor_type as i32) >= 0 && (hw.processor_type as i32) <= 19);
        assert_eq!(hw.physical_memory_mb(), 512);
        assert_eq!(hw.free_physical_memory_mb(), 256);
    }
}
