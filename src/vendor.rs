use crate::error::{Error, Result};

/// Detected CPU hardware vendor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuVendor {
    Intel,
    Amd,
}

impl CpuVendor {
    /// Detects the vendor of the current CPU using CPUID leaf 0.
    ///
    /// Returns [`Error::UnsupportedCpu`] if:
    /// - The target architecture is not `x86_64`, or
    /// - The CPU is neither `GenuineIntel` nor `AuthenticAMD`.
    pub fn detect() -> Result<Self> {
        #[cfg(target_arch = "x86_64")]
        {
            // SAFETY: CPUID is always available on x86_64 targets.
            let cpuid = core::arch::x86_64::__cpuid(0);
            Self::from_cpuid_regs(cpuid.ebx, cpuid.edx, cpuid.ecx)
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Err(Error::UnsupportedCpu)
        }
    }

    /// Maps CPUID leaf 0 register values (ebx, edx, ecx) to a [`CpuVendor`].
    ///
    /// The vendor string bytes are spread across the three registers in
    /// little-endian order: `[ebx_bytes][edx_bytes][ecx_bytes]`.
    /// `"GenuineIntel"` → [`CpuVendor::Intel`], `"AuthenticAMD"` → [`CpuVendor::Amd`].
    pub fn from_cpuid_regs(ebx: u32, edx: u32, ecx: u32) -> Result<Self> {
        let mut vendor = [0u8; 12];
        vendor[0..4].copy_from_slice(&ebx.to_le_bytes());
        vendor[4..8].copy_from_slice(&edx.to_le_bytes());
        vendor[8..12].copy_from_slice(&ecx.to_le_bytes());

        match &vendor {
            b"GenuineIntel" => Ok(CpuVendor::Intel),
            b"AuthenticAMD" => Ok(CpuVendor::Amd),
            _ => Err(Error::UnsupportedCpu),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intel_vendor() {
        // "GenuineIntel": ebx="Genu", edx="ineI", ecx="ntel"
        let ebx = u32::from_le_bytes(*b"Genu");
        let edx = u32::from_le_bytes(*b"ineI");
        let ecx = u32::from_le_bytes(*b"ntel");
        assert_eq!(CpuVendor::from_cpuid_regs(ebx, edx, ecx).unwrap(), CpuVendor::Intel);
    }

    #[test]
    fn test_amd_vendor() {
        // "AuthenticAMD": ebx="Auth", edx="enti", ecx="cAMD"
        let ebx = u32::from_le_bytes(*b"Auth");
        let edx = u32::from_le_bytes(*b"enti");
        let ecx = u32::from_le_bytes(*b"cAMD");
        assert_eq!(CpuVendor::from_cpuid_regs(ebx, edx, ecx).unwrap(), CpuVendor::Amd);
    }

    #[test]
    fn test_unknown_vendor() {
        // "CentaurHauls" (VIA)
        let ebx = u32::from_le_bytes(*b"Cent");
        let edx = u32::from_le_bytes(*b"aurH");
        let ecx = u32::from_le_bytes(*b"auls");
        assert!(matches!(
            CpuVendor::from_cpuid_regs(ebx, edx, ecx),
            Err(Error::UnsupportedCpu)
        ));
    }
}
