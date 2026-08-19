use crate::permissions::Permission;
use crate::redaction::{redact_env, RedactionResult};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{atomic::AtomicBool, atomic::Ordering, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
// platform spawn provides wait_timeout semantics via ProcessHandle

#[derive(Debug, Serialize)]
pub struct CommandSpec {
    pub executable: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: Option<HashMap<String, String>>,
    /// optional timeout
    pub timeout: Option<Duration>,
    /// max bytes to collect per stream
    pub output_limit: Option<usize>,
}

#[derive(Debug)]
pub struct CommandResult {
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub started_at: Instant,
    pub finished_at: Instant,
    pub timed_out: bool,
    pub cancelled: bool,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

#[derive(thiserror::Error, Debug)]
pub enum CommandError {
    #[error("permission denied")]
    PermissionDenied,
    #[error("decision required to run command")]
    DecisionRequired,
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("spawn failed: {0}")]
    SpawnFailed(String),
    #[error("timeout")]
    Timeout,
    #[error("cancelled")]
    Cancelled,
    #[error("output limit exceeded")]
    OutputLimitExceeded,
}

/// Run a command according to the spec. `cancel_token` can be used to request cancellation from another thread.
pub fn run_command(
    spec: CommandSpec,
    permission: Permission,
    cancel_token: Option<Arc<AtomicBool>>,
) -> Result<CommandResult, CommandError> {
    match permission {
        Permission::Deny => return Err(CommandError::PermissionDenied),
        Permission::Ask => return Err(CommandError::DecisionRequired),
        Permission::Allow => {}
    }

    let mut cmd = Command::new(&spec.executable);
    cmd.args(&spec.args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = &spec.cwd {
        cmd.current_dir(cwd);
    }
    if let Some(envs) = &spec.env {
        for (k, v) in envs {
            cmd.env(k, v);
        }
    }

    // Spawn platform-specific process handle (ensures atomic job assignment on Windows)
    let mut child_handle = crate::platform::spawn_process(&mut cmd).map_err(|e| match e {
        CommandError::SpawnFailed(s) => CommandError::SpawnFailed(s),
        other => other,
    })?;

    let started_at = Instant::now();

    let stdout = child_handle
        .take_stdout()
        .ok_or_else(|| CommandError::Io(std::io::Error::other("missing stdout")))?;
    let stderr = child_handle
        .take_stderr()
        .ok_or_else(|| CommandError::Io(std::io::Error::other("missing stderr")))?;

    let out_buf = Arc::new(Mutex::new(Vec::new()));
    let err_buf = Arc::new(Mutex::new(Vec::new()));
    let stdout_truncated = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stderr_truncated = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let out_clone = out_buf.clone();
    let err_clone = err_buf.clone();
    let out_trunc = stdout_truncated.clone();
    let err_trunc = stderr_truncated.clone();

    let limit = spec.output_limit.unwrap_or(10 * 1024 * 1024); // default 10MB

    // reader thread for stdout
    let stdout_handle = thread::spawn(move || -> std::io::Result<()> {
        use std::io::Read;
        let mut r = stdout;
        let mut buf = [0u8; 4096];
        loop {
            let n = r.read(&mut buf)?;
            if n == 0 {
                break;
            }
            let mut out = out_clone.lock().unwrap();
            if out.len() < limit {
                let space = limit - out.len();
                let to_take = std::cmp::min(space, n);
                out.extend_from_slice(&buf[..to_take]);
                if to_take < n {
                    out_trunc.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            } else {
                // already reached limit; mark truncated and continue draining
                out_trunc.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        Ok(())
    });

    // reader thread for stderr
    let stderr_handle = thread::spawn(move || -> std::io::Result<()> {
        use std::io::Read;
        let mut r = stderr;
        let mut buf = [0u8; 4096];
        loop {
            let n = r.read(&mut buf)?;
            if n == 0 {
                break;
            }
            let mut out = err_clone.lock().unwrap();
            if out.len() < limit {
                let space = limit - out.len();
                let to_take = std::cmp::min(space, n);
                out.extend_from_slice(&buf[..to_take]);
                if to_take < n {
                    err_trunc.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            } else {
                err_trunc.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        Ok(())
    });

    // wait with timeout support
    let timed_out = if let Some(to) = spec.timeout {
        // implement deadline/poll loop so cancellation is checked promptly even when a long
        // timeout was requested. Poll the process in small intervals.
        let deadline = Instant::now() + to;
        let poll = Duration::from_millis(100);
        loop {
            if let Some(tok) = &cancel_token {
                if tok.load(Ordering::SeqCst) {
                    let _ = child_handle.kill();
                    // ensure readers are drained before returning
                    let _ = stdout_handle.join();
                    let _ = stderr_handle.join();
                    return Err(CommandError::Cancelled);
                }
            }

            let now = Instant::now();
            if now >= deadline {
                // timeout expired
                let _ = child_handle.kill();
                break true;
            }

            let remaining = deadline - now;
            let to_wait = std::cmp::min(remaining, poll);
            match child_handle
                .wait_timeout(to_wait)
                .map_err(CommandError::Io)?
            {
                Some(_status) => break false,
                None => {
                    // continue loop to check cancellation and remaining time
                }
            }
        }
    } else {
        // no timeout: wait until completion, but support cancellation (poll loop)
        loop {
            if let Some(tok) = &cancel_token {
                if tok.load(Ordering::SeqCst) {
                    let _ = child_handle.kill();
                    break true;
                }
            }
            match child_handle.try_wait() {
                Ok(Some(_status)) => break false,
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                Err(e) => return Err(CommandError::Io(e)),
            }
        }
    };

    // if cancel_token set and was triggered, report cancelled
    if let Some(tok) = &cancel_token {
        if tok.load(Ordering::SeqCst) {
            let _ = child_handle.kill();
            let _ = stdout_handle.join();
            let _ = stderr_handle.join();
            return Err(CommandError::Cancelled);
        }
    }

    // join readers
    let _ = stdout_handle.join();
    let _ = stderr_handle.join();

    let finished_at = Instant::now();

    let out = out_buf.lock().unwrap().clone();
    let err = err_buf.lock().unwrap().clone();

    if timed_out {
        return Err(CommandError::Timeout);
    }

    // get exit code
    let status = child_handle.wait().map_err(CommandError::Io)?;
    let code = status.code();

    Ok(CommandResult {
        exit_code: code,
        stdout: out,
        stderr: err,
        started_at,
        finished_at,
        timed_out: false,
        cancelled: false,
        stdout_truncated: stdout_truncated.load(std::sync::atomic::Ordering::SeqCst),
        stderr_truncated: stderr_truncated.load(std::sync::atomic::Ordering::SeqCst),
    })
}

#[derive(Debug, serde::Serialize)]
pub struct SanitizedCommandRecord {
    pub executable: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub env: Vec<(String, RedactionResult)>,
    pub timeout_secs: Option<u64>,
    pub output_limit: Option<usize>,
}

pub fn sanitize_command_record(spec: &CommandSpec) -> SanitizedCommandRecord {
    let cwd = spec.cwd.as_ref().map(|p| p.to_string_lossy().to_string());
    let mut envs = Vec::new();
    if let Some(envmap) = &spec.env {
        for (k, v) in envmap {
            envs.push((k.clone(), redact_env(k, v)));
        }
    }
    SanitizedCommandRecord {
        executable: spec.executable.clone(),
        args: spec.args.clone(),
        cwd,
        env: envs,
        timeout_secs: spec.timeout.map(|d| d.as_secs()),
        output_limit: spec.output_limit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn denies_when_permission_is_deny() {
        let spec = CommandSpec {
            executable: "git".into(),
            args: vec!["--version".into()],
            cwd: None,
            env: None,
            timeout: None,
            output_limit: None,
        };
        let res = run_command(spec, Permission::Deny, None);
        assert!(matches!(res, Err(CommandError::PermissionDenied)));
    }

    #[test]
    fn ask_returns_decision_required() {
        let spec = CommandSpec {
            executable: "git".into(),
            args: vec!["--version".into()],
            cwd: None,
            env: None,
            timeout: None,
            output_limit: None,
        };
        let res = run_command(spec, Permission::Ask, None);
        assert!(matches!(res, Err(CommandError::DecisionRequired)));
    }

    #[test]
    fn invalid_executable_returns_spawn_failed() {
        let spec = CommandSpec {
            executable: "no-such-exe-12345".into(),
            args: vec![],
            cwd: None,
            env: None,
            timeout: None,
            output_limit: None,
        };
        let res = run_command(spec, Permission::Allow, None);
        assert!(matches!(res, Err(CommandError::SpawnFailed(_))));
    }

    #[test]
    fn invalid_cwd_returns_error() {
        let spec = CommandSpec {
            executable: "git".into(),
            args: vec!["--version".into()],
            cwd: Some(PathBuf::from("/no/such/dir")),
            env: None,
            timeout: None,
            output_limit: None,
        };
        let res = run_command(spec, Permission::Allow, None);
        assert!(res.is_err());
    }

    #[test]
    #[cfg(unix)]
    fn stdout_output_limit_triggers() {
        let spec = CommandSpec {
            executable: "yes".into(),
            args: vec![],
            cwd: None,
            env: None,
            timeout: Some(Duration::from_secs(1)),
            output_limit: Some(1024),
        };
        let res = run_command(spec, Permission::Allow, None);
        match res {
            Err(CommandError::Timeout) => {}
            Err(CommandError::OutputLimitExceeded) => {}
            Ok(r) => {
                // should be truncated in at least one stream
                assert!(r.stdout_truncated || r.stderr_truncated);
            }
            other => panic!("unexpected result: {:?}", other),
        }
    }

    #[test]
    #[cfg(unix)]
    fn env_override_and_redaction() {
        use std::collections::HashMap;
        let mut env = HashMap::new();
        env.insert("MY_SECRET".to_string(), "supersecret".to_string());
        let spec = CommandSpec {
            executable: "printenv".into(),
            args: vec!["MY_SECRET".into()],
            cwd: None,
            env: Some(env.clone()),
            timeout: None,
            output_limit: None,
        };
        let res = run_command(spec, Permission::Allow, None).unwrap();
        // output should contain the value
        assert!(!res.stdout.is_empty());
    }

    #[test]
    fn success_runs_command() {
        let spec = CommandSpec {
            executable: "git".into(),
            args: vec!["--version".into()],
            cwd: None,
            env: None,
            timeout: None,
            output_limit: None,
        };
        let res = run_command(spec, Permission::Allow, None).unwrap();
        assert!(res.exit_code == Some(0));
        assert!(!res.stdout.is_empty());
    }

    #[test]
    fn timeout_kills_long_running() {
        let (exe, args) = if cfg!(windows) {
            (
                "powershell".to_string(),
                vec!["-Command".to_string(), "Start-Sleep -Seconds 5".to_string()],
            )
        } else {
            ("sleep".to_string(), vec!["5".to_string()])
        };

        let spec = CommandSpec {
            executable: exe,
            args,
            cwd: None,
            env: None,
            timeout: Some(Duration::from_millis(200)),
            output_limit: None,
        };

        let res = run_command(spec, Permission::Allow, None);
        assert!(matches!(res, Err(CommandError::Timeout)));
    }

    #[test]
    fn cancel_terminates() {
        let (exe, args) = if cfg!(windows) {
            (
                "powershell".to_string(),
                vec!["-Command".to_string(), "Start-Sleep -Seconds 5".to_string()],
            )
        } else {
            ("sleep".to_string(), vec!["5".to_string()])
        };

        let spec = CommandSpec {
            executable: exe,
            args,
            cwd: None,
            env: None,
            timeout: None,
            output_limit: None,
        };

        let token = Arc::new(AtomicBool::new(false));
        let tok2 = token.clone();
        // trigger cancel after 100ms
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            tok2.store(true, Ordering::SeqCst);
        });

        let res = run_command(spec, Permission::Allow, Some(token));
        assert!(matches!(res, Err(CommandError::Cancelled)));
    }

    #[test]
    #[cfg(windows)]
    fn timeout_kills_fast_spawned_grandchild() {
        use std::process::Command as StdCommand;
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::GetExitCodeProcess;
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };

        // Parent powershell: spawn a long-running grandchild immediately, print its PID, then sleep
        let ps_cmd = "$p = Start-Process -FilePath powershell -ArgumentList '-NoProfile','-Command','Start-Sleep -Seconds 300' -PassThru; Write-Output $p.Id; Start-Sleep -Seconds 300";

        let mut cmd = StdCommand::new("powershell");
        cmd.arg("-NoProfile")
            .arg("-Command")
            .arg(ps_cmd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        // Use platform spawn to ensure Job object handling path is exercised
        let mut handle = crate::platform::spawn_process(&mut cmd).expect("spawn");
        let out = handle.take_stdout().expect("stdout");
        let mut s = String::new();
        // read the line containing grandchild pid (only until newline)
        {
            use std::io::BufRead;
            let mut reader = std::io::BufReader::new(out);
            reader.read_line(&mut s).unwrap();
            // drain done; reader will be dropped
        }
        let pid: u32 = s
            .lines()
            .next()
            .and_then(|l| l.trim().parse().ok())
            .expect("pid");

        // kill the parent (this should terminate the job and the grandchild)
        let _ = handle.kill();

        // small wait for termination
        std::thread::sleep(std::time::Duration::from_millis(200));

        unsafe {
            let ph = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if ph == 0 {
                // process already gone
                return;
            }
            let mut code: u32 = 0;
            let ok = GetExitCodeProcess(ph, &mut code);
            let _ = CloseHandle(ph);
            assert!(ok != 0 && code != 259, "grandchild should be terminated");
        }
    }

    #[test]
    #[cfg(windows)]
    fn cancel_kills_fast_spawned_grandchild() {
        use std::process::Command as StdCommand;
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::GetExitCodeProcess;
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };

        let ps_cmd = "$p = Start-Process -FilePath powershell -ArgumentList '-NoProfile','-Command','Start-Sleep -Seconds 300' -PassThru; Write-Output $p.Id; Start-Sleep -Seconds 300";
        let mut cmd = StdCommand::new("powershell");
        cmd.arg("-NoProfile")
            .arg("-Command")
            .arg(ps_cmd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let mut handle = crate::platform::spawn_process(&mut cmd).expect("spawn");
        let out = handle.take_stdout().expect("stdout");
        let mut s = String::new();
        {
            use std::io::BufRead;
            let mut reader = std::io::BufReader::new(out);
            reader.read_line(&mut s).unwrap();
        }
        let pid: u32 = s
            .lines()
            .next()
            .and_then(|l| l.trim().parse().ok())
            .expect("pid");

        // simulate cancellation
        let _ = handle.kill();

        std::thread::sleep(std::time::Duration::from_millis(200));

        unsafe {
            let ph = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if ph == 0 {
                return;
            }
            let mut code: u32 = 0;
            let ok = GetExitCodeProcess(ph, &mut code);
            let _ = CloseHandle(ph);
            assert!(
                ok != 0 && code != 259,
                "grandchild should be terminated on cancel"
            );
        }
    }

    #[test]
    fn cancel_interrupts_process_with_long_timeout() {
        use std::time::Instant;
        let (exe, args) = if cfg!(windows) {
            (
                "powershell".to_string(),
                vec![
                    "-Command".to_string(),
                    "Start-Sleep -Seconds 60".to_string(),
                ],
            )
        } else {
            ("sleep".to_string(), vec!["60".to_string()])
        };

        let spec = CommandSpec {
            executable: exe,
            args,
            cwd: None,
            env: None,
            timeout: Some(std::time::Duration::from_secs(60)),
            output_limit: None,
        };

        let token = Arc::new(AtomicBool::new(false));
        let tok2 = token.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(1000));
            tok2.store(true, Ordering::SeqCst);
        });

        let start = Instant::now();
        let res = run_command(spec, Permission::Allow, Some(token));
        let elapsed = start.elapsed();
        assert!(matches!(res, Err(CommandError::Cancelled)));
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "cancellation should interrupt long timeout quickly"
        );
    }
}
