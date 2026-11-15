//! Checks before the node starts, so problems are named before they happen.

use std::{
    net::{Ipv4Addr, SocketAddrV4, TcpListener},
    path::Path,
};

use crate::{Error, GoMaxSettings, NodeLayout, Result};

/// Free space needed before the very first start: about 22 GB for the node
/// after a month, and room while it works (a Windows laptop peaked near
/// 19 GB in the 2022-23 spam blocks). 30 GB, the figure the page shows.
pub const FIRST_START_BYTES: u64 = 30 * GB;

/// Free space always kept, once the state exists, so the node never fills the
/// drive to the last byte.
pub const HEADROOM_BYTES: u64 = 2 * GB;

const GB: u64 = 1_000_000_000;

/// Checks free space and ports. Call before starting. `waiting` is told when
/// a port is still busy, most likely the node from last time still closing.
pub fn before_start(layout: &NodeLayout, settings: &GoMaxSettings, waiting: impl FnMut(&str)) -> Result<()> {
    before_start_waiting(layout, settings, PORT_WAIT_SECONDS, waiting)
}

/// [`before_start`], giving a busy port `wait_seconds` to come free.
pub fn before_start_waiting(
    layout: &NodeLayout,
    settings: &GoMaxSettings,
    wait_seconds: u64,
    waiting: impl FnMut(&str),
) -> Result<()> {
    disk(layout)?;
    ports(settings, wait_seconds, waiting)
}

fn disk(layout: &NodeLayout) -> Result<()> {
    let state = layout.zakura_state();
    let first_start = !has_any_state(&state);
    let needed = if first_start { FIRST_START_BYTES } else { HEADROOM_BYTES };
    let free = free_bytes(layout.root())?;
    if free < needed {
        return Err(Error::NotEnoughSpace { needed_gb: needed.div_ceil(GB), free_gb: free / GB });
    }
    Ok(())
}

fn has_any_state(state: &Path) -> bool {
    std::fs::read_dir(state).map(|mut entries| entries.next().is_some()).unwrap_or(false)
}

/// How long a busy port is given to come free: the node from last time can
/// take this long to close (as on the Mac).
const PORT_WAIT_SECONDS: u64 = 90;

fn ports(settings: &GoMaxSettings, wait_seconds: u64, mut waiting: impl FnMut(&str)) -> Result<()> {
    for (port, what) in [
        (settings.rpc_port, "the node's RPC"),
        (settings.wallet_port, "the wallet service"),
        (settings.p2p_port, "the Zcash network"),
    ] {
        let mut told = false;
        let mut waited = 0;
        // Binding and dropping at once: only asks whether the port is free.
        while TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)).is_err() {
            if waited >= wait_seconds {
                return Err(Error::PortInUse { port, what });
            }
            if !told {
                waiting(&crate::rules::words::port_still_closing(port));
                told = true;
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
            waited += 1;
        }
    }
    Ok(())
}

/// Free bytes on the drive that holds `path` (the nearest existing folder).
pub fn free_bytes(path: &Path) -> Result<u64> {
    let mut existing = path;
    while !existing.exists() {
        match existing.parent() {
            Some(parent) => existing = parent,
            None => break,
        }
    }
    platform::free_bytes(existing).map_err(|e| Error::io(format!("reading the free space of {}", existing.display()), e))
}

#[cfg(unix)]
mod platform {
    use std::{ffi::CString, os::unix::ffi::OsStrExt, path::Path};

    pub fn free_bytes(path: &Path) -> std::io::Result<u64> {
        let c_path = CString::new(path.as_os_str().as_bytes()).map_err(std::io::Error::other)?;
        let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: `c_path` is a valid C string and `stats` a valid out-pointer.
        if unsafe { libc::statvfs(c_path.as_ptr(), &mut stats) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        #[allow(clippy::unnecessary_cast)]
        Ok(stats.f_bavail as u64 * stats.f_frsize as u64)
    }
}

#[cfg(windows)]
mod platform {
    use std::{os::windows::ffi::OsStrExt, path::Path};

    use windows::{core::PCWSTR, Win32::Storage::FileSystem::GetDiskFreeSpaceExW};

    pub fn free_bytes(path: &Path) -> std::io::Result<u64> {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut free_to_caller = 0u64;
        // SAFETY: `wide` is null-terminated and outlives the call.
        unsafe { GetDiskFreeSpaceExW(PCWSTR(wide.as_ptr()), Some(&mut free_to_caller), None, None) }
            .map_err(|e| std::io::Error::other(e.message()))?;
        Ok(free_to_caller)
    }
}
