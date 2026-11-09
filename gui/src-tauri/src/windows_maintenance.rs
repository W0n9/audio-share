//! Windows-only runtime for ticket 11 maintenance actions.
//!
//! The active firewall profiles are read through
//! `INetFwPolicy2::CurrentProfileTypes` rather than localized `netsh` output.
//! The rule changes run in a temporary PowerShell script started through
//! `ShellExecuteExW` with the `runas` verb, so Windows shows the UAC prompt for
//! the application itself; the caller waits for that process on a blocking
//! thread, never the UI thread.
//!
//! This file is compiled only for Windows. The script text, profile-mask
//! conversion, and error mapping are pure and tested on every host in
//! [`crate::maintenance`].

use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, RPC_E_CHANGED_MODE, WAIT_OBJECT_0};
use windows::Win32::NetworkManagement::WindowsFirewall::{INetFwPolicy2, NetFwPolicy2};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, CLSCTX_LOCAL_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
use windows::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
};

use crate::maintenance::{
    classify_exit_code, classify_shell_error, netsh_profiles, powershell_args, powershell_script,
    MaintenanceError,
};

/// Deletes the temporary script when it goes out of scope, including on every
/// error path after creation.
struct TempScript(PathBuf);

impl Drop for TempScript {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Initialize COM as a single-threaded apartment on the calling (blocking)
/// thread.
///
/// `Ok(false)` means the thread already runs in another apartment
/// (`RPC_E_CHANGED_MODE`): COM is still usable, but this call must not be
/// paired with `CoUninitialize`.
fn com_initialize() -> Result<bool, String> {
    unsafe {
        let init = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if init.is_err() {
            if init == RPC_E_CHANGED_MODE {
                return Ok(false);
            }
            return Err(format!("COM initialization failed (0x{:08X})", init.0));
        }
        Ok(true)
    }
}

fn com_uninitialize() {
    unsafe { CoUninitialize() }
}

/// Read the currently active firewall profiles.
///
/// `CurrentProfileTypes` is a normal unelevated read; only the rule changes
/// need elevation.
fn current_profile_mask() -> Result<i32, MaintenanceError> {
    let initialized = com_initialize().map_err(MaintenanceError::ProfileQuery)?;

    let result = unsafe {
        CoCreateInstance::<_, INetFwPolicy2>(
            &NetFwPolicy2,
            None::<&windows::core::IUnknown>,
            CLSCTX_INPROC_SERVER | CLSCTX_LOCAL_SERVER,
        )
        .map_err(|error| MaintenanceError::ProfileQuery(error.message()))
        .and_then(|policy| {
            policy
                .CurrentProfileTypes()
                .map_err(|error| MaintenanceError::ProfileQuery(error.message()))
        })
    };

    if initialized {
        com_uninitialize();
    }
    result
}

/// Create a uniquely named `.ps1` in the user's temp directory.
///
/// `create_new` avoids overwriting a pre-existing file in the shared temp
/// directory; the file is removed by [`TempScript`] after the elevated process
/// exits.
fn write_temp_script(script: &str) -> Result<TempScript, MaintenanceError> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!(
        "audio-share-firewall-{}-{nanos}.ps1",
        std::process::id()
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| MaintenanceError::TempScript(error.to_string()))?;
    let guard = TempScript(path);
    file.write_all(script.as_bytes())
        .map_err(|error| MaintenanceError::TempScript(error.to_string()))?;
    Ok(guard)
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Start `program` elevated and wait for it to exit.
///
/// Shell extensions involved in `ShellExecuteEx` may require an STA, so COM is
/// initialized for the duration of the call as well.
fn run_elevated(program: &str, parameters: &str) -> Result<(), MaintenanceError> {
    let initialized = com_initialize().map_err(MaintenanceError::ElevationFailed)?;
    let result = shell_execute_elevated(program, parameters);
    if initialized {
        com_uninitialize();
    }
    result
}

fn shell_execute_elevated(program: &str, parameters: &str) -> Result<(), MaintenanceError> {
    let verb = wide("runas");
    let file = wide(program);
    let params = wide(parameters);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        // SW_HIDE; the UAC consent prompt is owned by the system.
        nShow: 0,
        ..Default::default()
    };

    unsafe {
        if let Err(error) = ShellExecuteExW(&mut info) {
            return Err(classify_shell_error(error.code().0, error.message()));
        }
        if info.hProcess.is_invalid() {
            return Err(MaintenanceError::ElevationFailed(
                "the elevated process was not started".to_owned(),
            ));
        }

        let wait = WaitForSingleObject(info.hProcess, INFINITE);
        let mut exit_code = 0u32;
        let read = GetExitCodeProcess(info.hProcess, &mut exit_code);
        let _ = CloseHandle(info.hProcess);

        if wait != WAIT_OBJECT_0 {
            return Err(MaintenanceError::WaitFailed(format!(
                "unexpected wait result {}",
                wait.0
            )));
        }
        read.map_err(|error| MaintenanceError::WaitFailed(error.message()))?;
        classify_exit_code(exit_code)
    }
}

/// `%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe`, falling back
/// to PATH resolution when the environment is unusual.
fn powershell_path() -> String {
    let root = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let candidate = root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    if candidate.is_file() {
        candidate.to_string_lossy().into_owned()
    } else {
        "powershell.exe".to_owned()
    }
}

/// Rebuild the inbound TCP and UDP rules for the running executable.
///
/// Blocking: call this from `spawn_blocking`/a worker thread. UAC cancellation
/// and netsh failures are returned as typed errors for the command layer to
/// localize.
pub fn repair_firewall_blocking() -> Result<(), MaintenanceError> {
    let active = current_profile_mask()?;
    let profiles = netsh_profiles(active);

    let exe = std::env::current_exe()
        .map_err(|error| MaintenanceError::AppPathUnavailable(error.to_string()))?;
    let exe = exe.to_str().ok_or_else(|| {
        MaintenanceError::AppPathUnavailable("the executable path is not valid Unicode".to_owned())
    })?;

    let script = powershell_script(exe, &profiles)?;
    let temp = write_temp_script(&script)?;
    let script_path = temp.0.to_string_lossy();
    let args = powershell_args(&script_path)?;
    run_elevated(&powershell_path(), &args)
}

/// Open `control.exe /name Microsoft.Sound`.
///
/// The child is not waited for: the v0.3.4 legacy Windows GUI blocked its UI
/// until the
/// control panel closed, which this intentionally fixes.
pub fn open_sound_panel() -> Result<(), MaintenanceError> {
    std::process::Command::new("control.exe")
        .args(["/name", "Microsoft.Sound"])
        .spawn()
        .map(|_child| ())
        .map_err(|error| MaintenanceError::SoundPanelFailed(error.to_string()))
}
