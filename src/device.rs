#[cfg(not(feature = "pawnio"))]
use std::{ffi::c_void, mem::size_of, ptr::null_mut};

#[cfg(feature = "pawnio")]
use std::sync::Mutex;

#[cfg(not(feature = "pawnio"))]
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, GetLastError, HANDLE, INVALID_HANDLE_VALUE},
    Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        OPEN_EXISTING,
    },
    System::IO::DeviceIoControl,
};

#[cfg(feature = "pawnio")]
use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadAffinityMask};

#[cfg(not(feature = "pawnio"))]
use crate::error::last_error;
use crate::error::{Error, Result};
#[cfg(not(feature = "pawnio"))]
use crate::util::to_utf16_z;

#[cfg(feature = "pawnio")]
use crate::{pawnio::PawnIo, vendor::CpuVendor};

#[cfg(feature = "scaphandre")]
const DEVICE_PATH: &str = r"\\.\\ScaphandreDriver";

#[cfg(feature = "winring0")]
const DEVICE_PATH: &str = r"\\.\\WinRing0_1_2_0";

#[cfg(feature = "pawnio")]
const INTEL_MODULE: &[u8] = include_bytes!("../drivers/pawnio/IntelMSR.bin");

#[cfg(feature = "pawnio")]
const AMD_MODULE: &[u8] = include_bytes!("../drivers/pawnio/AMDFamily17.bin");

#[cfg(feature = "scaphandre")]
#[repr(C)]
#[derive(Clone, Copy)]
struct DriverRequest {
    msr_register: u32,
    cpu_index: u32,
}

pub(crate) struct DeviceHandle {
    #[cfg(not(feature = "pawnio"))]
    handle: HANDLE,
    #[cfg(feature = "pawnio")]
    pawnio: Mutex<Option<PawnIo>>,
}

impl DeviceHandle {
    pub(crate) fn open() -> Result<Self> {
        #[cfg(feature = "pawnio")]
        {
            let vendor = CpuVendor::detect()?;
            let pawnio = PawnIo::open()?;

            let module = match vendor {
                CpuVendor::Intel => INTEL_MODULE,
                CpuVendor::Amd => AMD_MODULE,
            };

            pawnio.load(module)?;

            Ok(Self {
                pawnio: Mutex::new(Some(pawnio)),
            })
        }

        #[cfg(not(feature = "pawnio"))]
        {
            let path_w = to_utf16_z(DEVICE_PATH);

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
                if code == ERROR_FILE_NOT_FOUND || code == ERROR_PATH_NOT_FOUND {
                    return Err(Error::NotInstalled);
                }
                return Err(Error::WinApi {
                    context: "CreateFileW",
                    code,
                });
            }

            Ok(Self { handle })
        }
    }

    /// Reads an MSR value for a given CPU index.
    pub(crate) fn read_msr(&self, msr_register: u32, _cpu_index: u32) -> Result<u64> {
        #[cfg(feature = "pawnio")]
        {
            let cpu_index = _cpu_index;
            let guard = self.pawnio.lock().unwrap();
            let pawnio = guard.as_ref().ok_or(Error::DeviceClosed)?;

            let thread = unsafe { GetCurrentThread() };
            let prev_affinity = if cpu_index < 64 {
                let mask: usize = 1usize << cpu_index;
                let old = unsafe { SetThreadAffinityMask(thread, mask) };
                if old != 0 { Some(old) } else { None }
            } else {
                None
            };

            let in_cells = [msr_register as u64];
            let mut out_cells = [0u64; 1];
            let exec_result = pawnio.execute("ioctl_read_msr", &in_cells, &mut out_cells);

            if let Some(old_mask) = prev_affinity {
                unsafe { SetThreadAffinityMask(thread, old_mask) };
            }

            let n_returned = exec_result?;
            if n_returned != 1 {
                return Err(Error::DriverProtocol {
                    context: "ioctl_read_msr: expected 1 output cell",
                });
            }

            Ok(out_cells[0])
        }

        #[cfg(not(feature = "pawnio"))]
        {
            if self.handle == INVALID_HANDLE_VALUE {
                return Err(Error::DeviceClosed);
            }

            let mut value: u64 = 0;
            let mut bytes_returned: u32 = 0;

            #[cfg(feature = "scaphandre")]
            {
                let mut request = DriverRequest {
                    msr_register,
                    cpu_index: _cpu_index,
                };

                let ok = unsafe {
                    DeviceIoControl(
                        self.handle,
                        0,
                        &mut request as *mut DriverRequest as *mut c_void,
                        size_of::<DriverRequest>() as u32,
                        &mut value as *mut u64 as *mut c_void,
                        size_of::<u64>() as u32,
                        &mut bytes_returned,
                        null_mut(),
                    )
                };

                if ok == 0 {
                    return Err(last_error("DeviceIoControl"));
                }

                if bytes_returned != size_of::<u64>() as u32 {
                    return Err(Error::DriverProtocol {
                        context: "unexpected output size",
                    });
                }
            }

            #[cfg(feature = "winring0")]
            {
                // WinRing0 OLS_READ_MSR IOCTL: device_type=40000, function=0x821, method=BUFFERED, access=ANY
                const IOCTL_READ_MSR: u32 = (40000 << 16) | (0 << 14) | (0x821 << 2) | 0;

                let mut msr = msr_register;

                let ok = unsafe {
                    DeviceIoControl(
                        self.handle,
                        IOCTL_READ_MSR,
                        &mut msr as *mut u32 as *mut c_void,
                        size_of::<u32>() as u32,
                        &mut value as *mut u64 as *mut c_void,
                        size_of::<u64>() as u32,
                        &mut bytes_returned,
                        null_mut(),
                    )
                };

                if ok == 0 {
                    return Err(last_error("DeviceIoControl"));
                }

                if bytes_returned != size_of::<u64>() as u32 {
                    return Err(Error::DriverProtocol {
                        context: "unexpected output size",
                    });
                }
            }

            Ok(value)
        }
    }

    pub(crate) fn close(&mut self) -> Result<()> {
        #[cfg(feature = "pawnio")]
        {
            let mut guard = self.pawnio.lock().unwrap();
            if let Some(mut pawnio) = guard.take() {
                pawnio.close()?;
            }
            Ok(())
        }

        #[cfg(not(feature = "pawnio"))]
        {
            if self.handle == INVALID_HANDLE_VALUE {
                return Ok(());
            }

            let ok = unsafe { CloseHandle(self.handle) };
            self.handle = INVALID_HANDLE_VALUE;

            if ok == 0 {
                return Err(last_error("CloseHandle"));
            }

            Ok(())
        }
    }
}

impl Drop for DeviceHandle {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
