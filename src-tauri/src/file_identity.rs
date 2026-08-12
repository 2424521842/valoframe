//! Read-only filesystem identity snapshots shared by indexing and destructive workflows.
//!
//! A stable Windows identity is the volume serial plus the high/low halves of the file index.
//! Callers that merely index media may treat an error as "identity unavailable" and continue.
//! Destructive callers must keep treating every error as a verification failure.

use std::{fs, io, path::Path};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StableFileIdentity {
    pub volume_serial: u32,
    pub file_index_high: u32,
    pub file_index_low: u32,
}

impl StableFileIdentity {
    pub(crate) fn from_database_parts(
        volume_serial: Option<i64>,
        file_index_high: Option<i64>,
        file_index_low: Option<i64>,
    ) -> Option<Self> {
        match (volume_serial, file_index_high, file_index_low) {
            (Some(volume_serial), Some(file_index_high), Some(file_index_low)) => Some(Self {
                volume_serial: u32::try_from(volume_serial).ok()?,
                file_index_high: u32::try_from(file_index_high).ok()?,
                file_index_low: u32::try_from(file_index_low).ok()?,
            }),
            _ => None,
        }
    }

    pub(crate) const fn database_parts(self) -> (i64, i64, i64) {
        (
            self.volume_serial as i64,
            self.file_index_high as i64,
            self.file_index_low as i64,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StableFileSnapshot {
    pub(crate) size_bytes: i64,
    pub(crate) modified_ticks: i64,
    pub(crate) identity: Option<StableFileIdentity>,
}

/// Best-effort, read-only snapshot for ordinary indexing.
///
/// On Windows this opens the path without following a reparse point and reads the same handle
/// identity used by permanent deletion. On other platforms the stable identity is unavailable,
/// but size and modification time remain available for legacy matching.
pub(crate) fn read_stable_file_snapshot(path: &Path) -> io::Result<StableFileSnapshot> {
    #[cfg(windows)]
    {
        let handle = open_windows_file_for_identity(path)?;
        snapshot_windows_handle(&handle, false)
    }

    #[cfg(not(windows))]
    {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "stable identity target is not a regular file",
            ));
        }
        snapshot_metadata(&metadata)
    }
}

#[cfg(not(windows))]
pub(crate) fn snapshot_metadata(metadata: &fs::Metadata) -> io::Result<StableFileSnapshot> {
    use std::time::UNIX_EPOCH;

    let size_bytes = i64::try_from(metadata.len()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "file size exceeds the supported snapshot range",
        )
    })?;
    let modified_ticks = metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "file modification time predates the supported epoch",
            )
        })?
        .as_nanos();
    let modified_ticks = i64::try_from(modified_ticks).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "file modification time exceeds the supported snapshot range",
        )
    })?;

    Ok(StableFileSnapshot {
        size_bytes,
        modified_ticks,
        identity: None,
    })
}

#[cfg(windows)]
fn open_windows_file_for_identity(path: &Path) -> io::Result<fs::File> {
    use std::os::windows::fs::OpenOptionsExt;

    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE,
    };

    fs::OpenOptions::new()
        .read(true)
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(windows)]
pub(crate) fn snapshot_windows_handle(
    handle: &fs::File,
    expect_directory: bool,
) -> io::Result<StableFileSnapshot> {
    use std::os::windows::io::AsRawHandle;

    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY,
        FILE_ATTRIBUTE_REPARSE_POINT,
    };

    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    let succeeded = unsafe {
        GetFileInformationByHandle(
            handle.as_raw_handle(),
            &mut information as *mut BY_HANDLE_FILE_INFORMATION,
        )
    };
    if succeeded == 0 {
        return Err(io::Error::last_os_error());
    }

    let is_directory = information.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0;
    if information.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || is_directory != expect_directory
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "stable identity handle is a reparse point or has an unexpected type",
        ));
    }

    let size = (u64::from(information.nFileSizeHigh) << 32) | u64::from(information.nFileSizeLow);
    let modified = (u64::from(information.ftLastWriteTime.dwHighDateTime) << 32)
        | u64::from(information.ftLastWriteTime.dwLowDateTime);

    Ok(StableFileSnapshot {
        size_bytes: i64::try_from(size).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "file size exceeds the supported snapshot range",
            )
        })?,
        modified_ticks: i64::try_from(modified).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "file modification time exceeds the supported snapshot range",
            )
        })?,
        identity: Some(StableFileIdentity {
            volume_serial: information.dwVolumeSerialNumber,
            file_index_high: information.nFileIndexHigh,
            file_index_low: information.nFileIndexLow,
        }),
    })
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{read_stable_file_snapshot, StableFileIdentity};

    #[test]
    fn database_identity_parts_require_all_three_valid_values() {
        assert!(StableFileIdentity::from_database_parts(Some(1), Some(2), Some(3)).is_some());
        assert!(StableFileIdentity::from_database_parts(Some(1), None, Some(3)).is_none());
        assert!(StableFileIdentity::from_database_parts(Some(-1), Some(2), Some(3)).is_none());
        assert!(StableFileIdentity::from_database_parts(
            Some(i64::from(u32::MAX) + 1),
            Some(2),
            Some(3),
        )
        .is_none());
    }

    #[test]
    fn read_only_snapshot_rejects_directories() {
        let root = std::env::temp_dir().join(format!(
            "vhm-file-identity-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("temporary directory should exist");
        assert!(read_stable_file_snapshot(&root).is_err());
        fs::remove_dir_all(root).expect("temporary directory should be removable");
    }
}
