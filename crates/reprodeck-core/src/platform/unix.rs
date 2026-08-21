use crate::runner::CommandError;
use std::process::Child;
use std::time::Duration;

pub trait ProcessHandle {
    fn take_stdout(&mut self) -> Option<Box<dyn std::io::Read + Send>>;
    fn take_stderr(&mut self) -> Option<Box<dyn std::io::Read + Send>>;
    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>>;
    fn wait(&mut self) -> std::io::Result<std::process::ExitStatus>;
    fn kill(&mut self) -> std::io::Result<()>;
    fn wait_timeout(&mut self, dur: Duration) -> std::io::Result<Option<std::process::ExitStatus>>;
}

pub struct ProcessSupervisor {}

impl ProcessSupervisor {
    pub fn new_for_child(_child: &Child) -> Result<Self, CommandError> {
        // On Unix currently no-op; future: setpgid / setsid to control process group
        Ok(ProcessSupervisor {})
    }

    pub fn terminate(&self) -> Result<(), CommandError> {
        // no-op for now
        Ok(())
    }
}

pub fn spawn_process(
    cmd: &mut std::process::Command,
) -> Result<Box<dyn ProcessHandle>, CommandError> {
    // Arrange for the child to run in its own process group so we can kill the entire group
    #[cfg(not(target_os = "android"))]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                // setpgid(0,0) makes child its own process group leader.
                if libc::setpgid(0, 0) == 0 {
                    Ok(())
                } else {
                    Err(std::io::Error::last_os_error())
                }
            });
        }
    }

    let child = cmd
        .spawn()
        .map_err(|e| CommandError::SpawnFailed(e.to_string()))?;
    struct StdChildHandle(std::process::Child);
    impl ProcessHandle for StdChildHandle {
        fn take_stdout(&mut self) -> Option<Box<dyn std::io::Read + Send>> {
            self.0
                .stdout
                .take()
                .map(|stdout| Box::new(stdout) as Box<dyn std::io::Read + Send>)
        }
        fn take_stderr(&mut self) -> Option<Box<dyn std::io::Read + Send>> {
            self.0
                .stderr
                .take()
                .map(|stderr| Box::new(stderr) as Box<dyn std::io::Read + Send>)
        }
        fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
            self.0.try_wait()
        }
        fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
            self.0.wait()
        }
        fn kill(&mut self) -> std::io::Result<()> {
            // send SIGKILL to the process group so descendants are also terminated
            #[cfg(unix)]
            {
                let pid = self.0.id() as i32;
                // negative pid means kill the process group
                let r = unsafe { libc::kill(-pid, libc::SIGKILL) };
                if r == 0 {
                    Ok(())
                } else {
                    Err(std::io::Error::last_os_error())
                }
            }
            #[cfg(not(unix))]
            {
                self.0.kill()
            }
        }
        fn wait_timeout(
            &mut self,
            dur: Duration,
        ) -> std::io::Result<Option<std::process::ExitStatus>> {
            use wait_timeout::ChildExt;
            self.0.wait_timeout(dur)
        }
    }
    Ok(Box::new(StdChildHandle(child)))
}
