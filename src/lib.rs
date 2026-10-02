#![cfg_attr(not(windows), allow(dead_code, unused_imports))]

#[cfg(not(windows))]
compile_error!("msr-driver-rs supports Windows only");

#[cfg(all(feature = "scaphandre", feature = "winring0"))]
compile_error!("Cannot enable both 'scaphandre' and 'winring0' features at the same time");

#[cfg(all(feature = "scaphandre", feature = "pawnio"))]
compile_error!("Cannot enable both 'scaphandre' and 'pawnio' features at the same time");

#[cfg(all(feature = "winring0", feature = "pawnio"))]
compile_error!("Cannot enable both 'winring0' and 'pawnio' features at the same time");

#[cfg(not(any(feature = "scaphandre", feature = "winring0", feature = "pawnio")))]
compile_error!("Exactly one of 'scaphandre', 'winring0', or 'pawnio' features must be enabled");

mod device;
mod error;
#[cfg(feature = "pawnio")]
mod pawnio;
mod service;
mod util;
#[cfg(feature = "pawnio")]
pub mod vendor;

pub use crate::error::{Error, Result};
#[cfg(feature = "pawnio")]
pub use crate::vendor::CpuVendor;

/// Handle to the Windows MSR driver device.
///
/// Mutually exclusive features: `pawnio` (default), `scaphandre`, or `winring0`.
pub struct MsrDriver {
    device: device::DeviceHandle,
}

impl MsrDriver {
    /// Opens the device handle. The driver must already be installed and running.
    pub fn new() -> Result<Self> {
        match device::DeviceHandle::open() {
            Ok(device) => Ok(Self { device }),
            Err(Error::NotInstalled) => {
                if service::is_installed()? {
                    return Err(Error::NotRunning);
                }
                Err(Error::NotInstalled)
            }
            Err(err) => Err(err),
        }
    }

    /// Installs the driver service and starts it (requires Administrator rights).
    pub fn install() -> Result<()> {
        service::install()
    }

    /// Returns whether the driver service exists without requiring admin rights.
    pub fn is_installed() -> Result<bool> {
        service::is_installed()
    }

    /// Returns whether the deployed driver binary is older than the one
    /// bundled in this crate build (compared by content hash).
    pub fn needs_update() -> Result<bool> {
        service::needs_update()
    }

    /// Starts an already-installed, stopped driver service (requires Administrator rights).
    pub fn start() -> Result<()> {
        service::start()
    }

    /// Closes the driver handle.
    pub fn close(&mut self) -> Result<()> {
        self.device.close()
    }

    /// Uninstalls the driver service (requires Administrator rights).
    pub fn uninstall_service() -> Result<()> {
        service::uninstall()
    }

    /// Uninstalls the driver service from a running driver instance (requires Administrator rights).
    pub fn uninstall(&mut self) -> Result<()> {
        let _ = self.close();
        service::uninstall()
    }

    /// Reads an MSR value for a given CPU index.
    pub fn read_msr(&self, msr_register: u32, cpu_index: u32) -> Result<u64> {
        self.device.read_msr(msr_register, cpu_index)
    }
}

/// Cleans up legacy WinRing0 service and driver files created by msr-driver-rs.
///
/// Only removes the service and files if the service binary path matches `%TEMP%\msr-driver-rs\WinRing0x64.sys`. If WinRing0 was installed by another tool, it is left untouched.
pub fn legacy_winring0_cleanup() -> Result<()> {
    service::legacy_winring0_cleanup()
}
