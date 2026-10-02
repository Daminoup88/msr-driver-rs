use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    path::PathBuf,
    ptr::{null, null_mut},
};

use windows_sys::Win32::{
    Foundation::{
        ERROR_ALREADY_EXISTS, ERROR_INSUFFICIENT_BUFFER, ERROR_SERVICE_ALREADY_RUNNING, ERROR_SERVICE_DOES_NOT_EXIST,
        ERROR_SERVICE_EXISTS, GetLastError,
    },
    System::Services::{
        CloseServiceHandle, ControlService, CreateServiceW, DeleteService, OpenSCManagerW, OpenServiceW,
        QUERY_SERVICE_CONFIGW, QueryServiceConfigW, SC_HANDLE, SC_MANAGER_ALL_ACCESS, SC_MANAGER_CONNECT,
        SERVICE_ALL_ACCESS, SERVICE_AUTO_START, SERVICE_CONTROL_STOP, SERVICE_ERROR_NORMAL, SERVICE_KERNEL_DRIVER,
        SERVICE_QUERY_CONFIG, SERVICE_QUERY_STATUS, SERVICE_START, SERVICE_STATUS, StartServiceW,
    },
};

#[cfg(not(feature = "pawnio"))]
use windows_sys::Win32::System::Services::{ChangeServiceConfigW, SERVICE_NO_CHANGE};

use crate::{
    error::{Error, Result, last_error},
    util::to_utf16_z,
};

#[cfg(feature = "scaphandre")]
const DRIVER_BYTES: &[u8] = include_bytes!("../drivers/scaphandre/ScaphandreDrv.sys");
#[cfg(feature = "scaphandre")]
const SERVICE_NAME: &str = "ScaphandreDrv";
#[cfg(feature = "scaphandre")]
const SERVICE_DISPLAY_NAME: &str = "Scaphandre Driver Service";

#[cfg(feature = "winring0")]
const DRIVER_BYTES: &[u8] = include_bytes!("../drivers/winring0/WinRing0x64.sys");
#[cfg(feature = "winring0")]
const SERVICE_NAME: &str = "WinRing0_1_2_0";
#[cfg(feature = "winring0")]
const SERVICE_DISPLAY_NAME: &str = "Rust WinRing0 Driver Service";

#[cfg(feature = "pawnio")]
const DRIVER_BYTES: &[u8] = include_bytes!("../drivers/pawnio/PawnIO.sys");
#[cfg(feature = "pawnio")]
const SERVICE_NAME: &str = "PawnIO";
#[cfg(feature = "pawnio")]
const SERVICE_DISPLAY_NAME: &str = "PawnIO Kernel Driver";

pub(crate) fn is_installed() -> Result<bool> {
    let manager = open_service_manager(SC_MANAGER_CONNECT)?;

    let service_name_w = to_utf16_z(SERVICE_NAME);
    let service = unsafe { OpenServiceW(manager, service_name_w.as_ptr(), SERVICE_QUERY_STATUS) };

    if service.is_null() {
        let code = unsafe { GetLastError() };
        unsafe { CloseServiceHandle(manager) };
        if code == ERROR_SERVICE_DOES_NOT_EXIST {
            return Ok(false);
        }
        return Err(Error::WinApi {
            context: "OpenServiceW",
            code,
        });
    }

    unsafe {
        CloseServiceHandle(service);
        CloseServiceHandle(manager);
    }

    Ok(true)
}

/// Returns whether the deployed driver binary differs from the one embedded
/// in this crate build. Returns `false` if nothing is deployed yet.
pub(crate) fn needs_update() -> Result<bool> {
    #[cfg(feature = "pawnio")]
    {
        if !is_owned_by_us() {
            return Ok(false);
        }
    }

    match fs::read(driver_binary_path()) {
        Ok(existing) => Ok(hash_bytes(&existing) != hash_bytes(DRIVER_BYTES)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}

fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

/// Starts an already-installed, stopped service.
pub(crate) fn start() -> Result<()> {
    let manager = open_service_manager(SC_MANAGER_CONNECT)?;
    let service_name_w = to_utf16_z(SERVICE_NAME);
    let service = unsafe { OpenServiceW(manager, service_name_w.as_ptr(), SERVICE_START) };

    if service.is_null() {
        let code = unsafe { GetLastError() };
        unsafe { CloseServiceHandle(manager) };
        return Err(if code == ERROR_SERVICE_DOES_NOT_EXIST {
            Error::NotInstalled
        } else {
            Error::WinApi {
                context: "OpenServiceW",
                code,
            }
        });
    }

    let started = unsafe { StartServiceW(service, 0, null()) };
    let result = if started == 0 {
        let code = unsafe { GetLastError() };
        if code == ERROR_SERVICE_ALREADY_RUNNING {
            Ok(())
        } else {
            Err(Error::WinApi {
                context: "StartServiceW",
                code,
            })
        }
    } else {
        Ok(())
    };

    unsafe {
        CloseServiceHandle(service);
        CloseServiceHandle(manager);
    }

    result
}

pub(crate) fn install() -> Result<()> {
    #[cfg(feature = "pawnio")]
    {
        if is_installed()? {
            return start();
        }
    }

    let outdated = needs_update()?;
    if outdated {
        stop_service();
    }

    let driver_path = deploy_driver_binary(outdated)?;
    let manager = open_service_manager(SC_MANAGER_ALL_ACCESS)?;

    let service_name_w = to_utf16_z(SERVICE_NAME);
    let display_name_w = to_utf16_z(SERVICE_DISPLAY_NAME);
    let driver_path_w = to_utf16_z(driver_path.to_string_lossy().as_ref());

    let service = unsafe {
        CreateServiceW(
            manager,
            service_name_w.as_ptr(),
            display_name_w.as_ptr(),
            SERVICE_ALL_ACCESS,
            SERVICE_KERNEL_DRIVER,
            SERVICE_AUTO_START,
            SERVICE_ERROR_NORMAL,
            driver_path_w.as_ptr(),
            null(),
            null_mut(),
            null(),
            null(),
            null(),
        )
    };

    let service_handle = if service.is_null() {
        let code = unsafe { GetLastError() };
        if code == ERROR_SERVICE_EXISTS || code == ERROR_ALREADY_EXISTS {
            let existing = unsafe { OpenServiceW(manager, service_name_w.as_ptr(), SERVICE_ALL_ACCESS) };
            if existing.is_null() {
                unsafe { CloseServiceHandle(manager) };
                return Err(last_error("OpenServiceW"));
            }

            #[cfg(feature = "pawnio")]
            {
                mark_owned_by_us();
            }

            #[cfg(not(feature = "pawnio"))]
            {
                let changed = unsafe {
                    ChangeServiceConfigW(
                        existing,
                        SERVICE_NO_CHANGE,
                        SERVICE_AUTO_START,
                        SERVICE_NO_CHANGE,
                        driver_path_w.as_ptr(),
                        null(),
                        null_mut(),
                        null(),
                        null(),
                        null(),
                        null(),
                    )
                };
                if changed == 0 {
                    let code = unsafe { GetLastError() };
                    return Err(Error::WinApi {
                        context: "ChangeServiceConfigW",
                        code,
                    });
                }
            }

            existing
        } else {
            unsafe { CloseServiceHandle(manager) };
            return Err(Error::WinApi {
                context: "CreateServiceW",
                code,
            });
        }
    } else {
        #[cfg(feature = "pawnio")]
        {
            mark_owned_by_us();
        }
        service
    };

    unsafe { CloseServiceHandle(service_handle) };
    unsafe { CloseServiceHandle(manager) };

    start()
}

/// Stop the service so the backing `.sys` file can be replaced.
fn stop_service() {
    let Ok(manager) = open_service_manager(SC_MANAGER_ALL_ACCESS) else {
        return;
    };
    let service_name_w = to_utf16_z(SERVICE_NAME);
    let service = unsafe { OpenServiceW(manager, service_name_w.as_ptr(), SERVICE_ALL_ACCESS) };
    if !service.is_null() {
        let mut status = SERVICE_STATUS {
            dwServiceType: 0,
            dwCurrentState: 0,
            dwControlsAccepted: 0,
            dwWin32ExitCode: 0,
            dwServiceSpecificExitCode: 0,
            dwCheckPoint: 0,
            dwWaitHint: 0,
        };
        let _ = unsafe { ControlService(service, SERVICE_CONTROL_STOP, &mut status) };
        unsafe { CloseServiceHandle(service) };
    }
    unsafe { CloseServiceHandle(manager) };
}

pub(crate) fn uninstall() -> Result<()> {
    #[cfg(feature = "pawnio")]
    {
        if !is_owned_by_us() {
            return Ok(());
        }
    }

    let manager = open_service_manager(SC_MANAGER_ALL_ACCESS)?;
    let service_name_w = to_utf16_z(SERVICE_NAME);
    let service = unsafe { OpenServiceW(manager, service_name_w.as_ptr(), SERVICE_ALL_ACCESS) };

    if service.is_null() {
        let code = unsafe { GetLastError() };
        unsafe { CloseServiceHandle(manager) };
        if code == ERROR_SERVICE_DOES_NOT_EXIST {
            remove_driver_binary();
            #[cfg(feature = "pawnio")]
            clear_ownership();
            return Ok(());
        }
        return Err(Error::WinApi {
            context: "OpenServiceW",
            code,
        });
    }

    let mut status = SERVICE_STATUS {
        dwServiceType: 0,
        dwCurrentState: 0,
        dwControlsAccepted: 0,
        dwWin32ExitCode: 0,
        dwServiceSpecificExitCode: 0,
        dwCheckPoint: 0,
        dwWaitHint: 0,
    };

    let _ = unsafe { ControlService(service, SERVICE_CONTROL_STOP, &mut status) };

    let deleted = unsafe { DeleteService(service) };
    if deleted == 0 {
        let code = unsafe { GetLastError() };
        unsafe {
            CloseServiceHandle(service);
            CloseServiceHandle(manager);
        }
        return Err(Error::WinApi {
            context: "DeleteService",
            code,
        });
    }

    unsafe {
        CloseServiceHandle(service);
        CloseServiceHandle(manager);
    }

    remove_driver_binary();
    #[cfg(feature = "pawnio")]
    clear_ownership();

    Ok(())
}

fn open_service_manager(access: u32) -> Result<SC_HANDLE> {
    let manager = unsafe { OpenSCManagerW(null(), null(), access) };
    if manager.is_null() {
        return Err(last_error("OpenSCManagerW"));
    }
    Ok(manager)
}

fn deploy_driver_binary(force_rewrite: bool) -> Result<PathBuf> {
    let path = driver_binary_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    if force_rewrite || !path.exists() {
        fs::write(&path, DRIVER_BYTES)?;
    }

    Ok(path)
}

fn remove_driver_binary() {
    let path = driver_binary_path();
    let _ = fs::remove_file(path);
}

fn driver_binary_path() -> PathBuf {
    #[cfg(feature = "pawnio")]
    {
        let base = std::env::var_os("ProgramData")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
        let mut path = base;
        path.push("msr-driver-rs");
        path.push("PawnIO.sys");
        path
    }

    #[cfg(not(feature = "pawnio"))]
    {
        let mut path = std::env::temp_dir();
        #[cfg(feature = "scaphandre")]
        {
            path.push("msr-driver-rs");
            path.push("ScaphandreDrv.sys");
        }
        #[cfg(feature = "winring0")]
        {
            path.push("msr-driver-rs");
            path.push("WinRing0x64.sys");
        }
        path
    }
}

#[cfg(feature = "pawnio")]
fn ownership_marker_path() -> PathBuf {
    let mut path = driver_binary_path();
    path.set_extension("installed_by_msr_driver_rs");
    path
}

#[cfg(feature = "pawnio")]
fn mark_owned_by_us() {
    let path = ownership_marker_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, b"msr-driver-rs");
}

#[cfg(feature = "pawnio")]
fn clear_ownership() {
    let path = ownership_marker_path();
    let _ = fs::remove_file(path);
}

#[cfg(feature = "pawnio")]
fn is_owned_by_us() -> bool {
    ownership_marker_path().exists()
}

fn query_service_binary_path(service_name: &str) -> Result<Option<String>> {
    let manager = open_service_manager(SC_MANAGER_CONNECT)?;
    let service_name_w = to_utf16_z(service_name);
    let service = unsafe { OpenServiceW(manager, service_name_w.as_ptr(), SERVICE_QUERY_CONFIG) };

    if service.is_null() {
        let code = unsafe { GetLastError() };
        unsafe { CloseServiceHandle(manager) };
        if code == ERROR_SERVICE_DOES_NOT_EXIST {
            return Ok(None);
        }
        return Err(Error::WinApi {
            context: "OpenServiceW",
            code,
        });
    }

    let mut bytes_needed = 0u32;
    unsafe {
        QueryServiceConfigW(service, null_mut(), 0, &mut bytes_needed);
    }
    let code = unsafe { GetLastError() };
    if code != ERROR_INSUFFICIENT_BUFFER {
        unsafe {
            CloseServiceHandle(service);
            CloseServiceHandle(manager);
        }
        return Err(Error::WinApi {
            context: "QueryServiceConfigW",
            code,
        });
    }

    let mut buffer = vec![0u8; bytes_needed as usize];
    let ok = unsafe {
        QueryServiceConfigW(
            service,
            buffer.as_mut_ptr() as *mut QUERY_SERVICE_CONFIGW,
            bytes_needed,
            &mut bytes_needed,
        )
    };

    if ok == 0 {
        let code = unsafe { GetLastError() };
        unsafe {
            CloseServiceHandle(service);
            CloseServiceHandle(manager);
        }
        return Err(Error::WinApi {
            context: "QueryServiceConfigW",
            code,
        });
    }

    let config = unsafe { &*(buffer.as_ptr() as *const QUERY_SERVICE_CONFIGW) };
    let path_ptr = config.lpBinaryPathName;
    let mut len = 0;
    while unsafe { *path_ptr.add(len) } != 0 {
        len += 1;
    }
    let slice = unsafe { std::slice::from_raw_parts(path_ptr, len) };
    let binary_path = String::from_utf16_lossy(slice);

    unsafe {
        CloseServiceHandle(service);
        CloseServiceHandle(manager);
    }

    Ok(Some(binary_path))
}

pub fn legacy_winring0_cleanup() -> Result<()> {
    let mut our_expected_path = std::env::temp_dir();
    our_expected_path.push("msr-driver-rs");
    our_expected_path.push("WinRing0x64.sys");

    let service_name = "WinRing0_1_2_0";

    if let Some(binary_path) = query_service_binary_path(service_name)? {
        let normalized_binary = binary_path.trim_matches('"').trim_start_matches(r"\??\").to_lowercase();
        let our_str = our_expected_path.to_string_lossy().to_lowercase();

        if normalized_binary == our_str {
            let manager = open_service_manager(SC_MANAGER_ALL_ACCESS)?;
            let service_name_w = to_utf16_z(service_name);
            let service = unsafe { OpenServiceW(manager, service_name_w.as_ptr(), SERVICE_ALL_ACCESS) };

            if !service.is_null() {
                let mut status = SERVICE_STATUS {
                    dwServiceType: 0,
                    dwCurrentState: 0,
                    dwControlsAccepted: 0,
                    dwWin32ExitCode: 0,
                    dwServiceSpecificExitCode: 0,
                    dwCheckPoint: 0,
                    dwWaitHint: 0,
                };
                let _ = unsafe { ControlService(service, SERVICE_CONTROL_STOP, &mut status) };
                let _ = unsafe { DeleteService(service) };
                unsafe { CloseServiceHandle(service) };
            }
            unsafe { CloseServiceHandle(manager) };

            let _ = fs::remove_file(&our_expected_path);
        }
    }

    Ok(())
}
