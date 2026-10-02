use std::{ffi::c_void, mem::size_of, ptr::null_mut};

use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, GetLastError, HANDLE, INVALID_HANDLE_VALUE},
    Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        OPEN_EXISTING,
    },
    System::IO::DeviceIoControl,
};

use crate::{
    error::{Error, Result, last_error},
    util::to_utf16_z,
};

/// Win32 device path for the PawnIO kernel driver.
const PAWNIO_DEVICE_PATH: &str = r"\\.\PawnIO";

// IOCTL codes: device type 41394 (0xA1B2), method BUFFERED (0), access ANY (0)
// Formula: (device_type << 16) | (function << 2)
const IOCTL_PIO_LOAD_BINARY: u32 = (41394 << 16) | (0x821 << 2); // 0xA1B2_2084
const IOCTL_PIO_EXECUTE_FN: u32 = (41394 << 16) | (0x841 << 2); // 0xA1B2_2104

/// Length of the function name field in the PawnIO execute IOCTL buffer (bytes, including null terminator).
const FN_NAME_LEN: usize = 32;

/// Formats a function name string into the fixed 32-byte null-padded buffer required by PawnIO.
///
/// Returns an error if `name` is 32 bytes or longer (no room for null terminator).
fn encode_fn_name(name: &str) -> Result<[u8; FN_NAME_LEN]> {
    let bytes = name.as_bytes();
    if bytes.len() >= FN_NAME_LEN {
        return Err(Error::DriverProtocol {
            context: "PawnIO function name too long (max 31 ASCII characters)",
        });
    }
    let mut buf = [0u8; FN_NAME_LEN];
    buf[..bytes.len()].copy_from_slice(bytes);
    Ok(buf)
}

/// Builds the input buffer for `IOCTL_PIO_EXECUTE_FN`:
/// `[32 bytes name] + [n_cells * 8 bytes of u64 input values]`.
fn build_execute_input(name: &str, in_cells: &[u64]) -> Result<Vec<u8>> {
    let name_buf = encode_fn_name(name)?;
    let mut payload = Vec::with_capacity(FN_NAME_LEN + in_cells.len() * size_of::<u64>());
    payload.extend_from_slice(&name_buf);
    for &cell in in_cells {
        payload.extend_from_slice(&cell.to_le_bytes());
    }
    Ok(payload)
}

/// Handle to the PawnIO kernel driver device (`\\.\PawnIO`).
pub(crate) struct PawnIo {
    handle: HANDLE,
}

// SAFETY: HANDLE is a raw Win32 kernel handle. Once opened, it can be safely
// transferred between threads when access is externally synchronized (e.g. via Mutex).
unsafe impl Send for PawnIo {}
unsafe impl Sync for PawnIo {}

impl PawnIo {
    /// Opens `\\.\PawnIO`. Returns [`Error::NotInstalled`] if the device is absent
    /// (driver not running). Returns [`Error::WinApi`] for other OS-level failures.
    pub(crate) fn open() -> Result<Self> {
        let path_w = to_utf16_z(PAWNIO_DEVICE_PATH);

        let handle = unsafe {
            CreateFileW(
                path_w.as_ptr(),
                FILE_GENERIC_READ | FILE_GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null_mut(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                null_mut(),
            )
        };

        if handle == INVALID_HANDLE_VALUE {
            let code = unsafe { GetLastError() };
            return Err(if code == ERROR_FILE_NOT_FOUND || code == ERROR_PATH_NOT_FOUND {
                Error::NotInstalled
            } else {
                Error::WinApi {
                    context: "CreateFileW(PawnIO)",
                    code,
                }
            });
        }

        Ok(Self { handle })
    }

    /// Loads a compiled PawnIO module blob (`.bin`) into the driver.
    ///
    /// Each loaded module can be queried and executed by name. Multiple loads
    /// accumulate; calling with the same module again is a no-op in practice.
    pub(crate) fn load(&self, blob: &[u8]) -> Result<()> {
        debug_assert!(self.handle != INVALID_HANDLE_VALUE, "load called on closed PawnIo");

        let mut bytes_returned: u32 = 0;
        let ok = unsafe {
            DeviceIoControl(
                self.handle,
                IOCTL_PIO_LOAD_BINARY,
                blob.as_ptr() as *const c_void as *mut c_void,
                blob.len() as u32,
                null_mut(),
                0,
                &mut bytes_returned,
                null_mut(),
            )
        };

        if ok == 0 {
            let code = unsafe { GetLastError() };
            return Err(Error::ModuleLoad { status: code });
        }

        Ok(())
    }

    /// Executes a named function exported by a loaded PawnIO module.
    ///
    /// `input` and `out` are u64 "cell" arrays. Returns the number of cells
    /// actually written to `out` by the driver.
    ///
    /// Maps Windows error code 5 (`ERROR_ACCESS_DENIED`) to [`Error::MsrNotAllowed`]
    /// so callers can distinguish "MSR not on allow-list" from other failures.
    pub(crate) fn execute(&self, name: &str, input: &[u64], out: &mut [u64]) -> Result<usize> {
        debug_assert!(self.handle != INVALID_HANDLE_VALUE, "execute called on closed PawnIo");

        let in_buf = build_execute_input(name, input)?;
        let out_byte_cap = (out.len() * size_of::<u64>()) as u32;
        let mut bytes_returned: u32 = 0;

        let ok = unsafe {
            DeviceIoControl(
                self.handle,
                IOCTL_PIO_EXECUTE_FN,
                in_buf.as_ptr() as *const c_void as *mut c_void,
                in_buf.len() as u32,
                out.as_mut_ptr() as *mut c_void,
                out_byte_cap,
                &mut bytes_returned,
                null_mut(),
            )
        };

        if ok == 0 {
            let code = unsafe { GetLastError() };
            // ERROR_ACCESS_DENIED (5) is returned when the module's allow-list
            // rejects the requested MSR address.
            let msr_hint = input.first().copied().unwrap_or(0) as u32;
            return Err(if code == 5 {
                Error::MsrNotAllowed { msr: msr_hint }
            } else {
                Error::WinApi {
                    context: "DeviceIoControl(IOCTL_PIO_EXECUTE_FN)",
                    code,
                }
            });
        }

        Ok((bytes_returned as usize) / size_of::<u64>())
    }

    /// Closes the device handle. Safe to call more than once (idempotent).
    pub(crate) fn close(&mut self) -> Result<()> {
        if self.handle == INVALID_HANDLE_VALUE {
            return Ok(());
        }
        let ok = unsafe { CloseHandle(self.handle) };
        self.handle = INVALID_HANDLE_VALUE;
        if ok == 0 {
            return Err(last_error("CloseHandle(PawnIO)"));
        }
        Ok(())
    }
}

impl Drop for PawnIo {
    fn drop(&mut self) {
        // Best-effort close; errors in Drop are swallowed.
        let _ = self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_fn_name_ok() {
        let buf = encode_fn_name("ioctl_read_msr").unwrap();
        assert_eq!(&buf[..14], b"ioctl_read_msr");
        assert_eq!(buf[14], 0, "byte after name must be null");
        assert_eq!(buf[31], 0, "last byte must be null");
        assert_eq!(buf.len(), 32);
    }

    #[test]
    fn test_encode_fn_name_too_long() {
        let long = "a".repeat(32); // exactly 32 bytes → too long (no room for null)
        assert!(encode_fn_name(&long).is_err());
    }

    #[test]
    fn test_build_execute_input() {
        let cells = [0x611u64];
        let payload = build_execute_input("ioctl_read_msr", &cells).unwrap();
        assert_eq!(payload.len(), 32 + 8);
        assert_eq!(&payload[..14], b"ioctl_read_msr");
        assert_eq!(&payload[32..40], &0x611u64.to_le_bytes());
    }
}
