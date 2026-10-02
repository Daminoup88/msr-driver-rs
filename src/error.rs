use std::fmt;

use windows_sys::Win32::Foundation::GetLastError;

/// All errors that can be returned by `msr-driver-rs`.
#[derive(Debug)]
pub enum Error {
    /// An I/O error occurred (file system operations for the driver binary).
    Io(std::io::Error),
    /// A Windows API call returned a non-zero error code.
    WinApi { context: &'static str, code: u32 },
    /// The driver returned an unexpected protocol response.
    DriverProtocol { context: &'static str },
    /// The driver is not installed or the device is not present.
    NotInstalled,
    /// The driver service is installed but the device is not running/accessible.
    NotRunning,
    /// The device handle has already been closed.
    DeviceClosed,
    /// The CPU vendor or model is not supported.
    UnsupportedCpu,
    /// A driver module failed to load.
    ModuleLoad { status: u32 },
    /// The requested MSR address is not allowed by driver policy.
    MsrNotAllowed { msr: u32 },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::WinApi { context, code } => {
                write!(f, "Windows API error in {context} (code {code})")
            }
            Self::DriverProtocol { context } => write!(f, "Driver protocol error: {context}"),
            Self::NotInstalled => write!(f, "Driver is not installed"),
            Self::NotRunning => write!(f, "Driver is installed but not running"),
            Self::DeviceClosed => write!(f, "Driver handle is closed"),
            Self::UnsupportedCpu => write!(f, "Unsupported CPU vendor or model"),
            Self::ModuleLoad { status } => {
                write!(f, "Driver module load failed with status {status:#x}")
            }
            Self::MsrNotAllowed { msr } => {
                write!(f, "MSR {msr:#010x} is not allowed by driver policy")
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn last_error(context: &'static str) -> Error {
    Error::WinApi {
        context,
        code: unsafe { GetLastError() },
    }
}
