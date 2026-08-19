#![allow(non_snake_case)]
use crate::runner::CommandError;
use std::io;
use std::mem::zeroed;
use std::os::raw::c_void;
use std::os::windows::prelude::AsRawHandle;
use std::process::Child;
use std::ptr::null_mut;

use std::fs::File;
use std::os::windows::io::{FromRawHandle, RawHandle};
use std::time::Duration;

use windows_sys::Win32::Foundation::SetHandleInformation;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_FAILED, WAIT_OBJECT_0};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject, TerminateJobObject,
    JOBOBJECT_BASIC_LIMIT_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::LPPROC_THREAD_ATTRIBUTE_LIST;
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
    InitializeProcThreadAttributeList, ResumeThread, TerminateProcess, UpdateProcThreadAttribute,
    WaitForSingleObject, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT,
    EXTENDED_STARTUPINFO_PRESENT, PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    STARTF_USESTDHANDLES, STARTUPINFOEXW, STARTUPINFOW,
};
const HANDLE_FLAG_INHERIT: u32 = 1;

fn to_wide(s: &str) -> Vec<u16> {
    let mut v: Vec<u16> = s.encode_utf16().collect();
    v.push(0);
    v
}

fn quote_windows_cmd_arg(arg: &str) -> String {
    // Follow Windows command-line quoting rules for CreateProcess
    // If empty, return "". If contains no spaces or quotes, return as-is.
    if arg.is_empty() {
        return "\"\"".to_string();
    }
    let need_quote = arg.chars().any(|c| c == ' ' || c == '\t' || c == '"');
    if !need_quote {
        return arg.to_string();
    }

    let mut res = String::new();
    res.push('"');
    let mut backslashes = 0;
    for ch in arg.chars() {
        if ch == '\\' {
            backslashes += 1;
            res.push('\\');
        } else if ch == '"' {
            // escape all backslashes before a quote
            for _ in 0..backslashes {
                res.push('\\');
            }
            backslashes = 0;
            res.push('\\');
            res.push('"');
        } else {
            backslashes = 0;
            res.push(ch);
        }
    }
    // escape trailing backslashes
    for _ in 0..backslashes {
        res.push('\\');
    }
    res.push('"');
    res
}

fn build_command_line(program: &str, args: &[String]) -> Vec<u16> {
    // Use lpCommandLine with quoted arguments; lpApplicationName will be program path
    let mut parts: Vec<String> = Vec::new();
    // First argument is program name as per CreateProcess rules
    parts.push(quote_windows_cmd_arg(program));
    for a in args {
        parts.push(quote_windows_cmd_arg(a));
    }
    to_wide(&parts.join(" "))
}

pub trait ProcessHandle {
    fn take_stdout(&mut self) -> Option<File>;
    fn take_stderr(&mut self) -> Option<File>;
    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>>;
    fn wait(&mut self) -> std::io::Result<std::process::ExitStatus>;
    fn kill(&mut self) -> std::io::Result<()>;
    fn wait_timeout(&mut self, dur: Duration) -> std::io::Result<Option<std::process::ExitStatus>>;
}

pub fn spawn_process(
    cmd: &mut std::process::Command,
) -> Result<Box<dyn ProcessHandle>, CommandError> {
    unsafe {
        // build application name and command line (proper quoting)
        let exe = cmd.get_program().to_string_lossy().to_string();
        let args_vec: Vec<String> = cmd
            .get_args()
            .map(|s| s.to_string_lossy().to_string())
            .collect();
        let mut clw = build_command_line(&exe, &args_vec);
        let _appw = to_wide(&exe);

        // create pipes for stdout/stderr
        let mut sa: SECURITY_ATTRIBUTES = std::mem::zeroed();
        sa.nLength = std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32;
        sa.bInheritHandle = 1; // TRUE: create inheritable handles; we'll use attribute list to control inheritance
        sa.lpSecurityDescriptor = null_mut();

        // RAII guard for native handles created during spawn; ensures cleanup on early error
        struct HandleGuard {
            out_read: HANDLE,
            out_write: HANDLE,
            err_read: HANDLE,
            err_write: HANDLE,
            in_read: HANDLE,
            in_write: HANDLE,
        }
        impl HandleGuard {
            fn new() -> Self {
                HandleGuard {
                    out_read: 0,
                    out_write: 0,
                    err_read: 0,
                    err_write: 0,
                    in_read: 0,
                    in_write: 0,
                }
            }
            fn disarm(&mut self) {
                self.out_read = 0;
                self.out_write = 0;
                self.err_read = 0;
                self.err_write = 0;
                self.in_read = 0;
                self.in_write = 0;
            }
        }
        impl Drop for HandleGuard {
            fn drop(&mut self) {
                unsafe {
                    if self.out_read != 0 {
                        let _ = CloseHandle(self.out_read);
                    }
                    if self.out_write != 0 {
                        let _ = CloseHandle(self.out_write);
                    }
                    if self.err_read != 0 {
                        let _ = CloseHandle(self.err_read);
                    }
                    if self.err_write != 0 {
                        let _ = CloseHandle(self.err_write);
                    }
                    if self.in_read != 0 {
                        let _ = CloseHandle(self.in_read);
                    }
                    if self.in_write != 0 {
                        let _ = CloseHandle(self.in_write);
                    }
                }
            }
        }

        let mut guard = HandleGuard::new();

        let mut out_read: HANDLE = 0;
        let mut out_write: HANDLE = 0;
        if CreatePipe(
            &mut out_read as *mut HANDLE,
            &mut out_write as *mut HANDLE,
            &sa as *const SECURITY_ATTRIBUTES,
            0,
        ) == 0
        {
            return Err(CommandError::SpawnFailed(format!(
                "CreatePipe stdout failed: {}",
                io::Error::last_os_error()
            )));
        }
        guard.out_read = out_read;
        guard.out_write = out_write;

        // ensure read handle not inheritable (parent's read end)
        if SetHandleInformation(out_read, HANDLE_FLAG_INHERIT, 0) == 0 {
            return Err(CommandError::SpawnFailed(format!(
                "SetHandleInformation failed for stdout read handle: {}",
                io::Error::last_os_error()
            )));
        }

        let mut err_read: HANDLE = 0;
        let mut err_write: HANDLE = 0;
        if CreatePipe(
            &mut err_read as *mut HANDLE,
            &mut err_write as *mut HANDLE,
            &sa as *const SECURITY_ATTRIBUTES,
            0,
        ) == 0
        {
            return Err(CommandError::SpawnFailed(format!(
                "CreatePipe stderr failed: {}",
                io::Error::last_os_error()
            )));
        }
        guard.err_read = err_read;
        guard.err_write = err_write;
        if SetHandleInformation(err_read, HANDLE_FLAG_INHERIT, 0) == 0 {
            return Err(CommandError::SpawnFailed(format!(
                "SetHandleInformation failed for stderr read handle: {}",
                io::Error::last_os_error()
            )));
        }

        // stdin pipe (parent write -> child read)
        let mut in_read: HANDLE = 0;
        let mut in_write: HANDLE = 0;
        if CreatePipe(
            &mut in_read as *mut HANDLE,
            &mut in_write as *mut HANDLE,
            &sa as *const SECURITY_ATTRIBUTES,
            0,
        ) == 0
        {
            return Err(CommandError::SpawnFailed(format!(
                "CreatePipe stdin failed: {}",
                io::Error::last_os_error()
            )));
        }
        guard.in_read = in_read;
        guard.in_write = in_write;
        // ensure parent write handle not inheritable (child should inherit read end)
        if SetHandleInformation(in_write, HANDLE_FLAG_INHERIT, 0) == 0 {
            return Err(CommandError::SpawnFailed(format!(
                "SetHandleInformation failed for stdin write handle: {}",
                io::Error::last_os_error()
            )));
        }

        // prepare STARTUPINFOW
        // prepare environment block and cwd, ensuring buffers live through CreateProcessW call
        // Build env block starting from the parent process environment, then apply Command overrides.
        // If a variable is set to None in Command.get_envs(), it should be removed.
        let mut env_block: Vec<u16> = Vec::new();
        let mut env_ptr: *mut c_void = null_mut();

        // Collect parent envs
        let mut env_map: std::collections::HashMap<String, String> = std::env::vars_os()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().to_string(),
                    v.to_string_lossy().to_string(),
                )
            })
            .collect();

        // Apply overrides / removals from Command
        for (k, v) in cmd.get_envs() {
            let key = k.to_string_lossy().to_string();
            match v {
                Some(val) => {
                    env_map.insert(key, val.to_string_lossy().to_string());
                }
                None => {
                    env_map.remove(&key);
                }
            }
        }

        if !env_map.is_empty() {
            // Windows expects a sequence of UTF-16 null-terminated "key=val" strings ending with an extra NUL
            for (k, v) in env_map {
                let mut w = to_wide(&format!("{}={}", k, v));
                env_block.append(&mut w);
            }
            env_block.push(0);
            env_ptr = env_block.as_mut_ptr() as *mut c_void;
        }

        let mut cwd_w: Vec<u16> = match cmd.get_current_dir() {
            Some(p) => to_wide(&p.to_string_lossy()),
            None => Vec::new(),
        };
        let cwd_ptr = if cwd_w.is_empty() {
            null_mut()
        } else {
            cwd_w.as_mut_ptr()
        };

        // Create an attribute list that explicitly lists the handles we want the child to inherit.
        let mut attr_size: usize = 0;
        // First call to get the required size
        InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut attr_size as *mut usize);
        if attr_size == 0 {
            return Err(CommandError::SpawnFailed(format!(
                "InitializeProcThreadAttributeList failed to get size: {}",
                io::Error::last_os_error()
            )));
        }

        // allocate initialized buffer for the attribute list with pointer alignment
        let elem = std::mem::size_of::<usize>();
        let count = attr_size.div_ceil(elem);
        let mut attr_buf: Vec<usize> = vec![0usize; count];
        let attr_ptr = attr_buf.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;

        if InitializeProcThreadAttributeList(attr_ptr, 1, 0, &mut attr_size as *mut usize) == 0 {
            return Err(CommandError::SpawnFailed(format!(
                "InitializeProcThreadAttributeList init failed: {}",
                io::Error::last_os_error()
            )));
        }

        // The list of handles the child should inherit: child ends of the pipes
        let inherit_handles: Vec<HANDLE> = vec![out_write, err_write, in_read];

        if UpdateProcThreadAttribute(
            attr_ptr,
            0,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
            inherit_handles.as_ptr() as *const c_void,
            (inherit_handles.len() * std::mem::size_of::<HANDLE>()) as usize,
            null_mut(),
            null_mut(),
        ) == 0
        {
            // cleanup attribute list
            DeleteProcThreadAttributeList(attr_ptr);
            return Err(CommandError::SpawnFailed(format!(
                "UpdateProcThreadAttribute failed: {}",
                io::Error::last_os_error()
            )));
        }

        // prepare STARTUPINFOEXW (extended startup info with attribute list)
        let mut si_ex: STARTUPINFOEXW = std::mem::zeroed();
        si_ex.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        si_ex.lpAttributeList = attr_ptr;
        si_ex.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        si_ex.StartupInfo.hStdOutput = out_write as HANDLE;
        si_ex.StartupInfo.hStdError = err_write as HANDLE;
        si_ex.StartupInfo.hStdInput = in_read as HANDLE;

        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();

        // Creation flags: include EXTENDED_STARTUPINFO_PRESENT and optionally CREATE_UNICODE_ENVIRONMENT if we pass a UTF-16 env
        let mut creation_flags = CREATE_SUSPENDED | EXTENDED_STARTUPINFO_PRESENT;
        if !env_block.is_empty() {
            creation_flags |= CREATE_UNICODE_ENVIRONMENT;
        }

        let success = CreateProcessW(
            null_mut(),
            clw.as_mut_ptr(),
            null_mut(),
            null_mut(),
            1, // bInheritHandles = TRUE; attribute list supplies explicit handle inheritance
            creation_flags,
            env_ptr,
            cwd_ptr,
            &mut si_ex as *mut STARTUPINFOEXW as *mut STARTUPINFOW,
            &mut pi as *mut PROCESS_INFORMATION,
        );

        // (we will close parent-side pipe ends later after transferring ownership to Rust types)

        if success == 0 {
            // cleanup attribute list
            DeleteProcThreadAttributeList(attr_ptr);
            return Err(CommandError::SpawnFailed(format!(
                "CreateProcessW failed: {}",
                io::Error::last_os_error()
            )));
        }

        // create job and assign process
        let job = CreateJobObjectW(null_mut(), null_mut());
        if job == 0 {
            // cleanup
            let _ = TerminateProcess(pi.hProcess, 1);
            CloseHandle(pi.hProcess);
            CloseHandle(pi.hThread);
            return Err(CommandError::SpawnFailed(format!(
                "CreateJobObjectW failed: {}",
                io::Error::last_os_error()
            )));
        }

        // set kill on job close
        let mut basic: JOBOBJECT_BASIC_LIMIT_INFORMATION = zeroed();
        basic.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let mut ext: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
        ext.BasicLimitInformation = basic;
        let res = SetInformationJobObject(
            job,
            windows_sys::Win32::System::JobObjects::JobObjectExtendedLimitInformation,
            &ext as *const _ as *mut c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if res == 0 {
            let _ = TerminateProcess(pi.hProcess, 1);
            CloseHandle(pi.hProcess);
            CloseHandle(pi.hThread);
            CloseHandle(job);
            return Err(CommandError::SpawnFailed(format!(
                "SetInformationJobObject failed: {}",
                io::Error::last_os_error()
            )));
        }

        if AssignProcessToJobObject(job, pi.hProcess) == 0 {
            let _ = TerminateProcess(pi.hProcess, 1);
            CloseHandle(pi.hProcess);
            CloseHandle(pi.hThread);
            CloseHandle(job);
            return Err(CommandError::SpawnFailed(format!(
                "AssignProcessToJobObject failed: {}",
                io::Error::last_os_error()
            )));
        }

        // Resume thread so process runs inside job
        let resume_res = ResumeThread(pi.hThread);
        if resume_res == u32::MAX {
            // resume failed
            let _ = TerminateProcess(pi.hProcess, 1);
            CloseHandle(pi.hProcess);
            CloseHandle(pi.hThread);
            CloseHandle(job);
            return Err(CommandError::SpawnFailed(format!(
                "ResumeThread failed: {}",
                io::Error::last_os_error()
            )));
        }

        // close thread handle; keep process handle
        CloseHandle(pi.hThread);

        // attribute list and its buffers may be freed now that process is created
        DeleteProcThreadAttributeList(attr_ptr);

        // create File from out_read / err_read (transfer ownership to Rust)
        let out_file = File::from_raw_handle(out_read as RawHandle);
        let err_file = File::from_raw_handle(err_read as RawHandle);
        // prevent guard from closing handles now owned by File/WinHandle
        guard.disarm();
        // close parent-side handles that parent does not need
        let _ = CloseHandle(out_write);
        let _ = CloseHandle(err_write);
        let _ = CloseHandle(in_read);
        // close parent write handle to child's stdin unless caller expects to write to it
        let _ = CloseHandle(in_write);

        #[allow(dead_code)]
        struct WinHandle {
            proc: HANDLE,
            job: HANDLE,
            out: Option<File>,
            err: Option<File>,
        }
        impl ProcessHandle for WinHandle {
            fn take_stdout(&mut self) -> Option<File> {
                self.out.take()
            }
            fn take_stderr(&mut self) -> Option<File> {
                self.err.take()
            }
            fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
                let s = unsafe { WaitForSingleObject(self.proc, 0) };
                if s == WAIT_OBJECT_0 {
                    let mut code: u32 = 0;
                    let ok = unsafe { GetExitCodeProcess(self.proc, &mut code) };
                    if ok == 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    return Ok(Some(std::os::windows::process::ExitStatusExt::from_raw(
                        code,
                    )));
                }
                if s == WAIT_FAILED {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(None)
            }
            fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
                unsafe {
                    let r = WaitForSingleObject(self.proc, u32::MAX);
                    if r == WAIT_FAILED {
                        return Err(std::io::Error::last_os_error());
                    }
                    if r == WAIT_OBJECT_0 {
                        let mut code: u32 = 0;
                        let ok = GetExitCodeProcess(self.proc, &mut code);
                        if ok == 0 {
                            return Err(std::io::Error::last_os_error());
                        }
                        return Ok(std::os::windows::process::ExitStatusExt::from_raw(code));
                    }
                    // Unexpected return: treat as error
                    Err(std::io::Error::other(
                        "WaitForSingleObject returned unexpected value",
                    ))
                }
            }
            fn kill(&mut self) -> std::io::Result<()> {
                unsafe {
                    // Prefer terminating the entire Job Object so descendants are also killed.
                    if self.job != 0 {
                        let _ = TerminateJobObject(self.job, 1);
                    } else {
                        let _ = TerminateProcess(self.proc, 1);
                    }
                    Ok(())
                }
            }
            fn wait_timeout(
                &mut self,
                dur: Duration,
            ) -> std::io::Result<Option<std::process::ExitStatus>> {
                let ms = dur.as_millis().min(u32::MAX as u128) as u32;
                let r = unsafe { WaitForSingleObject(self.proc, ms) };
                if r == WAIT_OBJECT_0 {
                    let mut code: u32 = 0;
                    let ok = unsafe { GetExitCodeProcess(self.proc, &mut code) };
                    if ok == 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    return Ok(Some(std::os::windows::process::ExitStatusExt::from_raw(
                        code,
                    )));
                }
                if r == WAIT_FAILED {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(None)
            }
        }

        // Ensure native handles are closed when this handle wrapper is dropped.
        impl Drop for WinHandle {
            fn drop(&mut self) {
                unsafe {
                    if self.proc != 0 {
                        let _ = CloseHandle(self.proc);
                        self.proc = 0;
                    }
                    if self.job != 0 {
                        // Closing the job handle will trigger KILL_ON_JOB_CLOSE semantics
                        let _ = CloseHandle(self.job);
                        self.job = 0;
                    }
                }
            }
        }

        Ok(Box::new(WinHandle {
            proc: pi.hProcess,
            job,
            out: Some(out_file),
            err: Some(err_file),
        }))
    }
}

pub struct ProcessSupervisor {
    job: HANDLE,
}

impl ProcessSupervisor {
    pub fn new_for_child(child: &Child) -> Result<Self, CommandError> {
        unsafe {
            // create job object
            let job = CreateJobObjectW(null_mut(), null_mut());
            if job == 0 {
                return Err(CommandError::SpawnFailed(format!(
                    "CreateJobObject failed: {}",
                    io::Error::last_os_error()
                )));
            }

            // set KILL_ON_JOB_CLOSE
            let mut basic: JOBOBJECT_BASIC_LIMIT_INFORMATION = zeroed();
            basic.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let mut ext: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
            ext.BasicLimitInformation = basic;

            let res = SetInformationJobObject(
                job,
                windows_sys::Win32::System::JobObjects::JobObjectExtendedLimitInformation,
                &ext as *const _ as *mut c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if res == 0 {
                let e = io::Error::last_os_error();
                CloseHandle(job);
                return Err(CommandError::SpawnFailed(format!(
                    "SetInformationJobObject failed: {}",
                    e
                )));
            }

            // assign process to job
            let ph = child.as_raw_handle() as HANDLE;
            let ok = AssignProcessToJobObject(job, ph);
            if ok == 0 {
                let e = io::Error::last_os_error();
                // cleanup
                CloseHandle(job);
                return Err(CommandError::SpawnFailed(format!(
                    "AssignProcessToJobObject failed: {}",
                    e
                )));
            }

            Ok(ProcessSupervisor { job })
        }
    }

    pub fn terminate(&self) -> Result<(), CommandError> {
        unsafe {
            let r = TerminateJobObject(self.job, 1);
            if r == 0 {
                return Err(CommandError::SpawnFailed(format!(
                    "TerminateJobObject failed: {}",
                    io::Error::last_os_error()
                )));
            }
            Ok(())
        }
    }
}

impl Drop for ProcessSupervisor {
    fn drop(&mut self) {
        unsafe {
            if self.job != 0 {
                // closing handle will kill processes because KILL_ON_JOB_CLOSE is set
                let _ = CloseHandle(self.job);
                self.job = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command as StdCommand;

    // helper to compile a small Rust probe binary on-the-fly for tests
    fn compile_probe_to(path: &std::path::Path) -> std::io::Result<()> {
        use std::io::Write;
        let src = r#"use std::env;use std::io::{self,Write};fn main()->io::Result<()>{let args:Vec<String>=env::args().collect();for (i,a) in args.iter().enumerate(){println!("ARG:{}:{}",i,a);}if let Ok(cwd)=env::current_dir(){println!("CWD={}",cwd.to_string_lossy());}if let Ok(keys)=env::var("PROBE_KEYS"){for k in keys.split(','){let ktrim=k.trim();if ktrim.is_empty(){continue;}match env::var(ktrim){Ok(v)=>println!("ENV:{}={}",ktrim,v),Err(_)=>println!("ENV:{}=<MISSING>",ktrim),}}}io::stdout().flush()?;Ok(()) }"#;
        let src_path = path.with_extension("rs");
        let mut f = std::fs::File::create(&src_path)?;
        f.write_all(src.as_bytes())?;
        // invoke rustc to compile
        let status = StdCommand::new("rustc")
            .arg("--edition=2021")
            .arg(&src_path)
            .arg("-o")
            .arg(path)
            .status()
            .map_err(|e| std::io::Error::other(format!("rustc failed: {}", e)))?;
        if status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other("rustc failed"))
        }
    }

    #[test]
    fn windows_env_override_and_inherit_and_remove() {
        // ensure parent env is visible if not overridden
        // Use the built-in arg_probe helper binary to deterministically inspect argv/cwd/env
        // compile a probe binary into a path with spaces under target/debug to avoid execution policy blocking
        let cur = std::env::current_exe().expect("current_exe");
        let debug = cur
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf())
            .expect("unable to locate debug dir");
        // Try to use prebuilt bin (target/debug/arg_probe.exe) if present; otherwise compile a small probe
        let prebuilt = debug.join("arg_probe.exe");
        let probe_copy = if prebuilt.exists() {
            // create a hard link with spaces in the filename pointing to prebuilt so AppLocker policies
            // that affect newly created binaries are less likely to trigger.
            let spaced = debug.join(format!("arg probe {}.exe", uuid::Uuid::new_v4()));
            std::fs::hard_link(&prebuilt, &spaced)
                .or_else(|_| std::fs::copy(&prebuilt, &spaced).map(|_| ()))
                .expect("link or copy prebuilt probe");
            spaced
        } else {
            let compiled = debug.join(format!("arg_probe_compiled_{}.exe", uuid::Uuid::new_v4()));
            compile_probe_to(&compiled).expect("compile probe");
            let spaced = debug.join(format!("arg probe {}.exe", uuid::Uuid::new_v4()));
            std::fs::hard_link(&compiled, &spaced)
                .or_else(|_| std::fs::copy(&compiled, &spaced).map(|_| ()))
                .expect("link or copy compiled probe");
            spaced
        };

        // ensure parent env variable present
        std::env::set_var("PARENT_VAR_TEST", "parentvalue");

        // executable path test + inherited env
        let mut cmd = StdCommand::new(probe_copy.to_string_lossy().to_string());
        cmd.env("PROBE_KEYS", "PARENT_VAR_TEST");
        cmd.stdout(std::process::Stdio::piped());
        let mut handle = match crate::platform::spawn_process(&mut cmd) {
            Ok(h) => h,
            Err(e) => {
                // If execution is blocked by application-control policy (OS error 4551),
                // fall back to the prebuilt binary (no space) if available and continue tests.
                let s = format!("{:?}", e);
                if s.contains("4551") || s.contains("Application") {
                    if prebuilt.exists() {
                        let mut cmd2 = StdCommand::new(prebuilt.to_string_lossy().to_string());
                        cmd2.env("PROBE_KEYS", "PARENT_VAR_TEST");
                        cmd2.stdout(std::process::Stdio::piped());
                        crate::platform::spawn_process(&mut cmd2).expect("spawn fallback")
                    } else {
                        panic!("spawn failed and no prebuilt probe available: {:?}", e)
                    }
                } else {
                    panic!("spawn failed: {:?}", e)
                }
            }
        };
        let mut out = String::new();
        let f = handle.take_stdout().expect("stdout");
        {
            use std::io::BufRead;
            let mut reader = std::io::BufReader::new(f);
            // read a few lines
            for _ in 0..3 {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                out.push_str(&line);
            }
        }
        assert!(out.contains("ARG:0:"), "probe printed argv");
        assert!(out.contains("ENV:PARENT_VAR_TEST=parentvalue"));

        // override works
        // override works
        let mut cmd2 = StdCommand::new(probe_copy.to_string_lossy().to_string());
        cmd2.env("PROBE_KEYS", "MY_TEST_ENV");
        cmd2.env("MY_TEST_ENV", "hello-world");
        cmd2.stdout(std::process::Stdio::piped());
        let mut handle2 = match crate::platform::spawn_process(&mut cmd2) {
            Ok(h) => h,
            Err(e) => {
                let s = format!("{:?}", e);
                if s.contains("4551") || s.contains("Application") {
                    if prebuilt.exists() {
                        let mut cmd2b = StdCommand::new(prebuilt.to_string_lossy().to_string());
                        cmd2b.env("PROBE_KEYS", "MY_TEST_ENV");
                        cmd2b.env("MY_TEST_ENV", "hello-world");
                        cmd2b.stdout(std::process::Stdio::piped());
                        crate::platform::spawn_process(&mut cmd2b).expect("spawn fallback")
                    } else {
                        panic!("spawn failed and no prebuilt probe available: {:?}", e)
                    }
                } else {
                    panic!("spawn failed: {:?}", e)
                }
            }
        };
        let mut out2 = String::new();
        let f2 = handle2.take_stdout().expect("stdout");
        {
            use std::io::BufRead;
            let mut reader = std::io::BufReader::new(f2);
            for _ in 0..5 {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                out2.push_str(&line);
            }
        }
        assert!(out2.contains("ENV:MY_TEST_ENV=hello-world"));

        // removal works: set an env in parent, then remove it via env_remove on command
        // removal works: set an env in parent, then remove it via env_remove on command
        std::env::set_var("TO_REMOVE_TEST", "to-be-removed");
        let mut cmd3 = StdCommand::new(probe_copy.to_string_lossy().to_string());
        cmd3.env("PROBE_KEYS", "TO_REMOVE_TEST");
        cmd3.env_remove("TO_REMOVE_TEST");
        cmd3.stdout(std::process::Stdio::piped());
        let mut handle3 = match crate::platform::spawn_process(&mut cmd3) {
            Ok(h) => h,
            Err(e) => {
                let s = format!("{:?}", e);
                if s.contains("4551") || s.contains("Application") {
                    if prebuilt.exists() {
                        let mut cmd3b = StdCommand::new(prebuilt.to_string_lossy().to_string());
                        cmd3b.env("PROBE_KEYS", "TO_REMOVE_TEST");
                        cmd3b.env_remove("TO_REMOVE_TEST");
                        cmd3b.stdout(std::process::Stdio::piped());
                        crate::platform::spawn_process(&mut cmd3b).expect("spawn fallback")
                    } else {
                        panic!("spawn failed and no prebuilt probe available: {:?}", e)
                    }
                } else {
                    panic!("spawn failed: {:?}", e)
                }
            }
        };
        let mut out3 = String::new();
        let f3 = handle3.take_stdout().expect("stdout");
        {
            use std::io::BufRead;
            let mut reader = std::io::BufReader::new(f3);
            for _ in 0..5 {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                out3.push_str(&line);
            }
        }
        assert!(out3.contains("ENV:TO_REMOVE_TEST=<MISSING>"));

        // unicode value
        // unicode value
        let mut cmd4 = StdCommand::new(probe_copy.to_string_lossy().to_string());
        cmd4.env("PROBE_KEYS", "UNICODE_TEST");
        cmd4.env("UNICODE_TEST", "こんにちは");
        cmd4.stdout(std::process::Stdio::piped());
        let mut handle4 = match crate::platform::spawn_process(&mut cmd4) {
            Ok(h) => h,
            Err(e) => {
                let s = format!("{:?}", e);
                if s.contains("4551") || s.contains("Application") {
                    if prebuilt.exists() {
                        let mut cmd4b = StdCommand::new(prebuilt.to_string_lossy().to_string());
                        cmd4b.env("PROBE_KEYS", "UNICODE_TEST");
                        cmd4b.env("UNICODE_TEST", "こんにちは");
                        cmd4b.stdout(std::process::Stdio::piped());
                        crate::platform::spawn_process(&mut cmd4b).expect("spawn fallback")
                    } else {
                        panic!("spawn failed and no prebuilt probe available: {:?}", e)
                    }
                } else {
                    panic!("spawn failed: {:?}", e)
                }
            }
        };
        let mut out4 = String::new();
        let f4 = handle4.take_stdout().expect("stdout");
        {
            use std::io::BufRead;
            let mut reader = std::io::BufReader::new(f4);
            for _ in 0..5 {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                out4.push_str(&line);
            }
        }
        assert!(out4.contains("ENV:UNICODE_TEST=こんにちは"));
    }

    #[test]
    fn windows_argument_and_cwd_semantics() {
        use std::io::BufRead;
        use tempfile::tempdir;

        let dir = tempdir().expect("tempdir");
        let script_dir = dir.path().join("cwd-test");
        std::fs::create_dir_all(&script_dir).unwrap();

        // Try to use prebuilt bin (target/debug/arg_probe.exe) if present; otherwise compile a small probe
        let cur = std::env::current_exe().expect("current_exe");
        let debug = cur
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf())
            .expect("unable to locate debug dir");
        let prebuilt = debug.join("arg_probe.exe");
        let probe_copy = if prebuilt.exists() {
            prebuilt
        } else {
            let compiled = debug.join(format!("arg_probe_compiled_{}.exe", uuid::Uuid::new_v4()));
            compile_probe_to(&compiled).expect("compile probe");
            let spaced = debug.join(format!("arg probe {}.exe", uuid::Uuid::new_v4()));
            let _ = std::fs::copy(&compiled, &spaced).expect("copy compiled to spaced name");
            spaced
        };

        let args = vec![
            "simple".to_string(),
            "arg with spaces".to_string(),
            "".to_string(),
            "ユニコード".to_string(),
            // quotes/backslashes edgecases
            "contains\"quote".to_string(),
            "backslashes\\before\\quote\"end".to_string(),
            "trailing\\".to_string(),
        ];

        let mut cmd = StdCommand::new(probe_copy.to_string_lossy().to_string());
        cmd.args(&args);
        cmd.current_dir(&script_dir);
        cmd.stdout(std::process::Stdio::piped());

        let mut handle = crate::platform::spawn_process(&mut cmd).expect("spawn");
        // read several lines of output
        let mut reader = std::io::BufReader::new(handle.take_stdout().expect("stdout"));
        let mut lines = Vec::new();
        for _ in 0..(args.len() + 2) {
            let mut line = String::new();
            let _ = reader.read_line(&mut line);
            if line.is_empty() {
                break;
            }
            // trim newline
            let trimmed = line.trim_end_matches(&['\r', '\n'][..]).to_string();
            lines.push(trimmed);
        }

        // parse ARG lines into argv values map
        let mut got_args = Vec::new();
        for l in &lines {
            if let Some(rest) = l.strip_prefix("ARG:") {
                if let Some(colpos) = rest.find(':') {
                    let val = &rest[colpos + 1..];
                    got_args.push(val.to_string());
                }
            }
        }

        // argv[1..] should match our args (argv[0] is program path)
        if got_args.len() > args.len() {
            let recv = &got_args[1..(args.len() + 1)];
            assert_eq!(recv, &args[..]);
        } else {
            panic!("not enough argv values from probe: {:?}", got_args);
        }

        // verify cwd
        let cwd_line = lines
            .iter()
            .find(|l| l.starts_with("CWD="))
            .expect("CWD present");
        let cwd = cwd_line.trim_start_matches("CWD=");
        assert_eq!(cwd, script_dir.to_string_lossy());
    }
}
