//! Free space on the volume a worktree would live on.
//!
//! A worktree is a full checkout, so provisioning one onto a nearly full disk
//! fails partway and leaves Git state to reconcile. The plan reports the numbers
//! up front instead, using the platform call directly rather than taking on a
//! dependency for two functions.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// Below this, provisioning is reported as a warning on the plan.
pub const LOW_DISK_THRESHOLD_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskReport {
    /// `None` when the platform call failed; the plan says so rather than
    /// guessing that there is room.
    pub available_bytes: Option<u64>,
    pub threshold_bytes: u64,
    pub low: bool,
}

#[must_use]
pub fn report_for(path: &Path) -> DiskReport {
    let available = available_bytes(path);
    DiskReport {
        available_bytes: available,
        threshold_bytes: LOW_DISK_THRESHOLD_BYTES,
        low: available.is_some_and(|bytes| bytes < LOW_DISK_THRESHOLD_BYTES),
    }
}

/// The nearest existing ancestor, since the worktree path itself does not exist
/// when the plan is built.
fn existing_ancestor(path: &Path) -> Option<&Path> {
    let mut candidate = Some(path);
    while let Some(current) = candidate {
        if current.exists() {
            return Some(current);
        }
        candidate = current.parent();
    }
    None
}

#[cfg(windows)]
#[must_use]
pub fn available_bytes(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;

    unsafe extern "system" {
        fn GetDiskFreeSpaceExW(
            directory_name: *const u16,
            free_bytes_available_to_caller: *mut u64,
            total_number_of_bytes: *mut u64,
            total_number_of_free_bytes: *mut u64,
        ) -> i32;
    }

    let target = existing_ancestor(path)?;
    let mut wide: Vec<u16> = target.as_os_str().encode_wide().collect();
    wide.push(0);
    let mut available: u64 = 0;
    let mut total: u64 = 0;
    let mut free: u64 = 0;
    // SAFETY: `wide` is a NUL-terminated UTF-16 path and the three out
    // parameters are live for the duration of the call.
    let succeeded = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &raw mut available,
            &raw mut total,
            &raw mut free,
        )
    };
    (succeeded != 0).then_some(available)
}

#[cfg(unix)]
#[must_use]
pub fn available_bytes(path: &Path) -> Option<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let target = existing_ancestor(path)?;
    let c_path = CString::new(target.as_os_str().as_bytes()).ok()?;
    // SAFETY: `statvfs` is zeroed before the call and `c_path` is NUL-terminated.
    let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
    let result = unsafe { libc::statvfs(c_path.as_ptr(), &raw mut stats) };
    if result != 0 {
        return None;
    }
    let block_size = if stats.f_frsize == 0 {
        stats.f_bsize
    } else {
        stats.f_frsize
    };
    u64::try_from(block_size).ok().and_then(|size| {
        u64::try_from(stats.f_bavail)
            .ok()
            .map(|blocks| blocks * size)
    })
}

#[cfg(not(any(unix, windows)))]
#[must_use]
pub fn available_bytes(_path: &Path) -> Option<u64> {
    None
}
