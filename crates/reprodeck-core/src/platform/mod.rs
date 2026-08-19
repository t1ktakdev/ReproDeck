#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{spawn_process, ProcessSupervisor};

#[cfg(not(windows))]
mod unix;
#[cfg(not(windows))]
pub use unix::{spawn_process, ProcessSupervisor};
