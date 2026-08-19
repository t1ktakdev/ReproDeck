use crate::runner::CommandError;
use std::process::Child;
use std::time::Duration;

pub trait ProcessHandle {
    fn take_stdout(&mut self) -> Option<std::fs::File>;
    fn take_stderr(&mut self) -> Option<std::fs::File>;
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
        cmd.before_exec(|| {
            // setpgid(0,0) makes child its own process group leader
            unsafe { libc::setpgid(0, 0) };
            Ok(())
        });
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| CommandError::SpawnFailed(e.to_string()))?;
    struct StdChildHandle(std::process::Child);
    impl ProcessHandle for StdChildHandle {
        fn take_stdout(&mut self) -> Option<std::fs::File> {
            self.0.stdout.take()
        }
        fn take_stderr(&mut self) -> Option<std::fs::File> {
            self.0.stderr.take()
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
                    return Ok(());
                } else {
                    return Err(std::io::Error::last_os_error());
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
