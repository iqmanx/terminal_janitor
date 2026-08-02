use std::path::Path;

use crate::model::{DiskCapacity, DiskError};

pub fn capacity_for(path: &Path) -> Result<DiskCapacity, DiskError> {
    super::measure(path)
}

pub fn file_identity(path: &Path) -> Result<(String, String), std::io::Error> {
    use std::os::windows::io::AsRawHandle;

    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };

    // Rust's std adds FILE_FLAG_BACKUP_SEMANTICS on Windows, so directories
    // can be opened for metadata queries without any manual CreateFileW call.
    let file = std::fs::File::open(path)?;
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: `file` owns a valid handle for the duration of the call and
    // `info` is a properly aligned, writable out-parameter.
    let ok = unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let file_index = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
    Ok((
        format!("winvol-serial:{:08x}", info.dwVolumeSerialNumber),
        format!("winfile-index:{file_index:016x}"),
    ))
}
