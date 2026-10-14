#[cfg(target_os = "windows")]
use dll_syringe::{process::OwnedProcess, Syringe};
#[cfg(target_os = "windows")]
use log::info;
#[cfg(target_os = "windows")]
use std::path::PathBuf;
#[cfg(target_os = "windows")]
use std::process::{Command, Stdio};

#[cfg(target_os = "windows")]
use crate::{constants::*, errors::*};

#[cfg(not(target_os = "windows"))]
pub async fn run_samp(
    _name: &str,
    _ip: &str,
    _port: i32,
    _executable_dir: &str,
    _dll_path: &str,
    _omp_file: &str,
    _password: &str,
    _custom_game_exe: &str,
) -> Result<()> {
    Ok(())
}

#[cfg(target_os = "windows")]
pub async fn run_samp(
    name: &str,
    ip: &str,
    port: i32,
    executable_dir: &str,
    dll_path: &str,
    omp_file: &str,
    password: &str,
    custom_game_exe: &str,
) -> Result<()> {
    // Optional (WINE-BUILD.md): start the game through sampcmd. OMP_SAMPCMD names
    // sampcmd.exe (absolute, or relative to the game folder), OMP_SAMPCMD_ARGS
    // holds its own options. sampcmd then starts the game and injects the same
    // samp.dll and omp-client.dll this launcher would have, adding its own
    // monitoring, crash capture and host checks. Unset, nothing changes.
    if let Ok(sampcmd) = std::env::var("OMP_SAMPCMD") {
        if !sampcmd.trim().is_empty() {
            return run_via_sampcmd(sampcmd.trim(), name, ip, port, executable_dir, dll_path,
                                   omp_file, password, custom_game_exe);
        }
    }

    // Prepare the command to spawn the executable
    let target_game_exe = if custom_game_exe.len() > 0 {
        custom_game_exe.to_string()
    } else {
        GTA_SA_EXECUTABLE.to_string()
    };

    let exe_path = PathBuf::from(executable_dir).join(&target_game_exe);

    let exe_path = exe_path.canonicalize().map_err(|e| {
        LauncherError::Process(format!("Invalid executable path {:?}: {}", exe_path, e))
    })?;

    let mut cmd = Command::new(&exe_path);

    let mut ready_for_exec = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .current_dir(executable_dir)
        .arg("-c")
        .arg("-n")
        .arg(name)
        .arg("-h")
        .arg(ip)
        .arg("-p")
        .arg(format!("{}", port));

    if !password.is_empty() {
        ready_for_exec = ready_for_exec.arg("-z").arg(password);
    }

    let process = ready_for_exec.current_dir(executable_dir).spawn();

    match process {
        Ok(p) => {
            inject_dll(p.id(), dll_path, 0, false)?;
            info!("[run_samp] omp_file.is_empty(): {}", omp_file.is_empty());
            if !omp_file.is_empty() {
                inject_dll(p.id(), omp_file, 0, false)
            } else {
                Ok(())
            }
        }
        Err(e) => {
            info!("[injector.rs] Process creation failed: {}", e);

            match e.raw_os_error() {
                Some(ERROR_ELEVATION_REQUIRED) => Err(LauncherError::AccessDenied(
                    "Unable to open game process".to_string(),
                )),
                Some(ERROR_ACCESS_DENIED) => Err(LauncherError::AccessDenied(
                    "Unable to open game process".to_string(),
                )),
                _ => Err(LauncherError::Process(format!(
                    "Failed to spawn process: {}",
                    e
                ))),
            }
        }
    }
}

#[cfg(target_os = "windows")]
#[allow(clippy::too_many_arguments)]
fn run_via_sampcmd(
    sampcmd: &str,
    name: &str,
    ip: &str,
    port: i32,
    executable_dir: &str,
    dll_path: &str,
    omp_file: &str,
    password: &str,
    custom_game_exe: &str,
) -> Result<()> {
    let sampcmd_path = if std::path::Path::new(sampcmd).is_absolute() {
        sampcmd.to_string()
    } else {
        format!("{}/{}", executable_dir, sampcmd)
    };
    let mut cmd = Command::new(&sampcmd_path);
    if let Ok(extra) = std::env::var("OMP_SAMPCMD_ARGS") {
        cmd.args(extra.split_whitespace());
    }
    cmd.arg("--game-dir").arg(executable_dir);
    if !custom_game_exe.is_empty() {
        cmd.arg("--exe").arg(custom_game_exe);
    }
    cmd.arg("--dll").arg(dll_path);
    // empty with --no-omp
    if !omp_file.is_empty() {
        cmd.arg("--omp").arg("--omp-dll").arg(omp_file);
    }
    cmd.arg("--")
        .arg("-c")
        .arg("-n")
        .arg(name)
        .arg("-h")
        .arg(ip)
        .arg("-p")
        .arg(format!("{}", port));
    if !password.is_empty() {
        cmd.arg("-z").arg(password);
    }

    info!("[injector.rs] Starting the game through {}", sampcmd_path);
    match cmd.current_dir(executable_dir).spawn() {
        Ok(_) => Ok(()),
        Err(e) => {
            info!("[injector.rs] sampcmd failed to start: {}", e);
            Err(LauncherError::Process(format!("Failed to start {}: {}", sampcmd_path, e)))
        }
    }
}

#[cfg(target_os = "windows")]
pub fn inject_dll(child: u32, dll_path: &str, times: u32, waiting_for_vorbis: bool) -> Result<()> {
    use winapi::{
        shared::minwindef::{FALSE, HMODULE},
        um::{
            processthreadsapi::OpenProcess,
            psapi::{EnumProcessModulesEx, GetModuleFileNameExA},
            winnt::PROCESS_ALL_ACCESS,
        },
    };

    match OwnedProcess::from_pid(child) {
        Ok(p) => {
            if waiting_for_vorbis {
                unsafe {
                    let handle = OpenProcess(PROCESS_ALL_ACCESS, FALSE, child);
                    let mut module_handles: [HMODULE; PROCESS_MODULE_BUFFER_SIZE] =
                        [0 as *mut _; PROCESS_MODULE_BUFFER_SIZE];
                    let mut found = 0;

                    EnumProcessModulesEx(
                        handle,
                        module_handles.as_mut_ptr(),
                        module_handles.len() as _,
                        &mut found,
                        0x03,
                    );

                    let mut bytes = [0i8; PROCESS_MODULE_BUFFER_SIZE];

                    if found == 0 {
                        let delay = std::time::Duration::from_millis(INJECTION_RETRY_DELAY_MS);
                        std::thread::sleep(delay);
                        return inject_dll(child, dll_path, times, true);
                    }

                    let mut found_vorbis = false;
                    for i in 0..(found / 4) {
                        if GetModuleFileNameExA(
                            handle,
                            module_handles[i as usize],
                            bytes.as_mut_ptr(),
                            PROCESS_MODULE_BUFFER_SIZE as u32,
                        ) != 0
                        {
                            let string = std::ffi::CStr::from_ptr(bytes.as_ptr());
                            if string.to_string_lossy().to_string().contains("vorbis") {
                                found_vorbis = true;
                            }
                        }
                    }

                    if !found_vorbis {
                        let delay = std::time::Duration::from_millis(INJECTION_RETRY_DELAY_MS);
                        std::thread::sleep(delay);
                        return inject_dll(child, dll_path, times, true);
                    }
                }
            }

            // create a new syringe for the target process
            let syringe = Syringe::for_process(p);

            // inject the payload into the target process
            match syringe.inject(dll_path) {
                Ok(_) => Ok(()),
                Err(e) => {
                    let delay = std::time::Duration::from_millis(INJECTION_RETRY_DELAY_MS);
                    std::thread::sleep(delay);

                    if times >= INJECTION_MAX_RETRIES {
                        info!(
                            "[injector.rs] DLL {} injection failed after {} attempts: {}",
                            dll_path, INJECTION_MAX_RETRIES, e
                        );

                        if !waiting_for_vorbis {
                            return inject_dll(child, dll_path, 0, true);
                        }
                        return Err(LauncherError::Injection(format!(
                            "DLL injection failed: {}",
                            e
                        )));
                    }

                    inject_dll(child, dll_path, times + 1, waiting_for_vorbis)
                }
            }
        }
        Err(e) => {
            info!("[injector.rs] Failed to access process: {}", e);

            match e.raw_os_error() {
                Some(ERROR_ELEVATION_REQUIRED) => Err(LauncherError::AccessDenied(
                    "Unable to open game process".to_string(),
                )),
                Some(ERROR_ACCESS_DENIED) => Err(LauncherError::AccessDenied(
                    "Unable to open game process".to_string(),
                )),
                _ => Err(LauncherError::Process(format!(
                    "Failed to access process: {}",
                    e
                ))),
            }
        }
    }
}
