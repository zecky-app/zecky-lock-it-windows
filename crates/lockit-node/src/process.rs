//! Starting and stopping the node programs.
//!
//! # The clean stop, and why Windows needs a helper
//!
//! Zakura and Zaino finish their database writes and exit cleanly when they
//! receive Ctrl-C; a hard kill can leave the database needing repair. On
//! macOS and Linux, Lock it sends `SIGINT`. Windows has no such signal, and
//! Zakura's RPC `stop` works on Regtest only, so the only clean way in is the
//! console's Ctrl-C event. A windowed app cannot send that directly:
//!
//! 1. Each node program is started with its own hidden console
//!    (`CREATE_NO_WINDOW`).
//! 2. To stop one, Lock it starts **a copy of itself** with
//!    [`STOP_HELPER_FLAG`] and the program's process id. That short-lived
//!    helper attaches to the node's console, ignores Ctrl-C itself, sends
//!    Ctrl-C to everything on that console, and exits.
//! 3. Lock it waits for the node to exit, and only kills it after
//!    [`STOP_GRACE`] has passed.
//!
//! Attaching to another console changes process-wide state, which is why it
//! happens in a separate process and never inside the app itself.
//!
//! The app's `main` must call [`run_stop_helper_if_asked`] first thing.
//!
//! # Children never outlive the app
//!
//! On Windows every node program is put in a job object that kills it when
//! the app's last handle to the job closes, so a crash of Lock it cannot leave
//! a node running in the background.

use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};

use crate::{Error, Result};

/// The argument that turns the app into the stop helper.
pub const STOP_HELPER_FLAG: &str = "--lockit-send-ctrl-c";

/// How long a node program gets to finish after Ctrl-C before it is killed.
pub const STOP_GRACE: Duration = Duration::from_secs(90);

/// If this process was started as the stop helper, does the helper's work and
/// returns the exit code to end with. Otherwise returns `None` at once.
///
/// Call this first thing in `main`:
///
/// ```ignore
/// if let Some(code) = lockit_node::process::run_stop_helper_if_asked() {
///     std::process::exit(code);
/// }
/// ```
pub fn run_stop_helper_if_asked() -> Option<i32> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some(STOP_HELPER_FLAG) {
        return None;
    }
    let Some(pid) = args.next().and_then(|text| text.parse::<u32>().ok()) else {
        return Some(2);
    };
    Some(match platform::send_ctrl_c(pid) {
        Ok(()) => 0,
        Err(_) => 1,
    })
}

/// Keeps the node programs tied to the app's lifetime.
pub struct ProcessGroup {
    inner: platform::Group,
}

impl ProcessGroup {
    /// Creates the group.
    pub fn new() -> Result<Self> {
        Ok(Self { inner: platform::Group::new()? })
    }
}

/// One running node program.
pub struct NodeProcess {
    name: String,
    child: Child,
    log: PathBuf,
}

impl NodeProcess {
    /// Starts `program` with `args`, its output going to `log`.
    ///
    /// The previous log is kept as `<name>.old.log`.
    pub fn start(
        group: &ProcessGroup,
        name: &str,
        program: &Path,
        args: &[&std::ffi::OsStr],
        log: &Path,
    ) -> Result<Self> {
        if !program.is_file() {
            return Err(Error::MissingProgram(program.display().to_string()));
        }
        if log.exists() {
            let old = log.with_extension("old.log");
            let _ = std::fs::rename(log, old);
        }
        let out = open_log(log)?;
        let err = out.try_clone().map_err(|e| Error::io("opening the node log", e))?;

        let mut command = Command::new(program);
        command.args(args).stdin(Stdio::null()).stdout(out).stderr(err);
        platform::prepare(&mut command);
        let child = command.spawn().map_err(|source| Error::Spawn { program: name.to_owned(), source })?;
        group.inner.adopt(&child)?;
        Ok(Self { name: name.to_owned(), child, log: log.to_owned() })
    }

    /// The program's name, for messages.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The program's process id.
    pub fn id(&self) -> u32 {
        self.child.id()
    }

    /// Where its output goes.
    pub fn log(&self) -> &Path {
        &self.log
    }

    /// `Ok(())` while running; an error saying how it ended otherwise.
    pub fn check_running(&mut self) -> Result<()> {
        match self.child.try_wait() {
            Ok(None) => Ok(()),
            Ok(Some(status)) => Err(self.exited(status)),
            Err(e) => Err(Error::io(format!("checking on {}", self.name), e)),
        }
    }

    /// Asks the program to stop cleanly, waits up to `grace`, then kills it.
    ///
    /// Returns `true` if it stopped by itself.
    pub fn stop(mut self, grace: Duration) -> Result<bool> {
        if let Ok(Some(_)) = self.child.try_wait() {
            return Ok(true);
        }
        // A failed request is not fatal: the kill below still ends it.
        let _ = platform::request_stop(&self.child);

        let deadline = Instant::now() + grace;
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(_)) => return Ok(true),
                Ok(None) => std::thread::sleep(Duration::from_millis(200)),
                Err(e) => return Err(Error::io(format!("waiting for {}", self.name), e)),
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        Ok(false)
    }

    fn exited(&self, status: ExitStatus) -> Error {
        Error::Exited { program: self.name.clone(), code: status.code(), log: self.log.display().to_string() }
    }
}

fn open_log(path: &Path) -> Result<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| Error::io(format!("opening {}", path.display()), e))
}

#[cfg(unix)]
mod platform {
    use std::process::{Child, Command};

    use crate::Result;

    pub struct Group;

    impl Group {
        pub fn new() -> Result<Self> {
            Ok(Self)
        }

        pub fn adopt(&self, _child: &Child) -> Result<()> {
            Ok(())
        }
    }

    pub fn prepare(command: &mut Command) {
        use std::os::unix::process::CommandExt;
        // Its own process group, so a Ctrl-C typed in a terminal running the
        // app reaches the app only; the app then stops the node in order.
        command.process_group(0);
    }

    pub fn request_stop(child: &Child) -> std::io::Result<()> {
        let pid = child.id() as libc::pid_t;
        // SAFETY: sends a signal to our own child; no memory is involved.
        if unsafe { libc::kill(pid, libc::SIGINT) } == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }

    pub fn send_ctrl_c(_pid: u32) -> std::io::Result<()> {
        Err(std::io::Error::other("the stop helper is only needed on Windows"))
    }
}

#[cfg(windows)]
mod platform {
    use std::{
        os::windows::{io::AsRawHandle, process::CommandExt},
        process::{Child, Command, Stdio},
        time::Duration,
    };

    use windows::Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::{
            Console::{AttachConsole, FreeConsole, GenerateConsoleCtrlEvent, SetConsoleCtrlHandler, CTRL_C_EVENT},
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
                SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            },
            Threading::{CREATE_NO_WINDOW, DETACHED_PROCESS},
        },
    };

    use super::STOP_HELPER_FLAG;
    use crate::{Error, Result};

    pub struct Group {
        job: HANDLE,
    }

    // The job handle is only used through thread-safe system calls.
    unsafe impl Send for Group {}
    unsafe impl Sync for Group {}

    impl Group {
        pub fn new() -> Result<Self> {
            // SAFETY: creates an unnamed job object owned by this process.
            let job = unsafe { CreateJobObjectW(None, None) }.map_err(platform)?;
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            // SAFETY: `limits` is the structure this information class expects.
            unsafe {
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            }
            .map_err(platform)?;
            Ok(Self { job })
        }

        pub fn adopt(&self, child: &Child) -> Result<()> {
            let process = HANDLE(child.as_raw_handle());
            // SAFETY: both handles are valid for the duration of the call.
            unsafe { AssignProcessToJobObject(self.job, process) }.map_err(platform)
        }
    }

    impl Drop for Group {
        fn drop(&mut self) {
            // Closing the last handle kills whatever is still in the job.
            // SAFETY: the handle is ours and closed once.
            unsafe {
                let _ = CloseHandle(self.job);
            }
        }
    }

    pub fn prepare(command: &mut Command) {
        // A console of its own, without a window: needed for the clean stop.
        command.creation_flags(CREATE_NO_WINDOW.0);
    }

    /// Starts the stop helper (a copy of this program) and waits for it.
    pub fn request_stop(child: &Child) -> std::io::Result<()> {
        let me = std::env::current_exe()?;
        let mut helper = Command::new(me)
            .arg(STOP_HELPER_FLAG)
            .arg(child.id().to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(DETACHED_PROCESS.0)
            .spawn()?;
        for _ in 0..50 {
            if helper.try_wait()?.is_some() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = helper.kill();
        Err(std::io::Error::other("the stop helper did not finish"))
    }

    /// The helper's work. Runs in its own short-lived process.
    pub fn send_ctrl_c(pid: u32) -> std::io::Result<()> {
        // SAFETY: plain console calls on this process's own console state.
        unsafe {
            let _ = FreeConsole();
            AttachConsole(pid).map_err(io)?;
            // Ignore Ctrl-C ourselves; it goes to everyone on the console.
            SetConsoleCtrlHandler(None, true).map_err(io)?;
            let sent = GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0).map_err(io);
            let _ = FreeConsole();
            sent
        }
    }

    fn platform(error: windows::core::Error) -> Error {
        Error::Platform(error.message())
    }

    fn io(error: windows::core::Error) -> std::io::Error {
        std::io::Error::other(error.message())
    }
}
