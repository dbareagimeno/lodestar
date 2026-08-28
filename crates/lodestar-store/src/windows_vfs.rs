//! Lodestar-specific Windows VFS adapter.
//!
//! SQLite's built-in win32 VFS opens database files with read/write sharing but without delete
//! sharing. Windows then rejects the atomic replacement of `index.db` while another process still
//! has the previous generation open. Lodestar selects this named VFS for every database it owns.
//! Its `xOpen` delegates to the bundled `win32` VFS, then replaces the just-opened handle with a
//! `ReOpenFile` handle that includes `FILE_SHARE_DELETE`, before SQLite can acquire any locks.
//! SQLite's process-wide syscall table and default VFS remain untouched.

use std::ffi::{c_char, c_int, CStr};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use rusqlite::{ffi, Connection, OpenFlags};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS, ERROR_FILE_NOT_FOUND,
    ERROR_INVALID_FUNCTION, ERROR_NOT_SUPPORTED, ERROR_PATH_NOT_FOUND, GENERIC_READ, GENERIC_WRITE,
    HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, CreateHardLinkW, FileDispositionInfoEx, FileIdInfo, FileRenameInfoEx,
    FlushFileBuffers, GetFileInformationByHandleEx, LockFileEx, ReOpenFile,
    SetFileInformationByHandle, DELETE, FILE_ATTRIBUTE_NORMAL, FILE_DISPOSITION_FLAG_DELETE,
    FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE, FILE_DISPOSITION_FLAG_POSIX_SEMANTICS,
    FILE_DISPOSITION_INFO_EX, FILE_FLAG_OPEN_REPARSE_POINT, FILE_FLAG_WRITE_THROUGH, FILE_ID_INFO,
    FILE_RENAME_INFO, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    LOCKFILE_FAIL_IMMEDIATELY, OPEN_EXISTING,
};
use windows_sys::Win32::System::WindowsProgramming::{
    FILE_RENAME_FLAG_POSIX_SEMANTICS, FILE_RENAME_FLAG_REPLACE_IF_EXISTS,
};
use windows_sys::Win32::System::IO::OVERLAPPED;

#[path = "windows_nt_path.rs"]
mod windows_nt_path;

const VFS_NAME: &str = "lodestar-win32-delete-share";
const VFS_NAME_C: &[u8] = b"lodestar-win32-delete-share\0";
const WIN32_VFS_NAME_C: &[u8] = b"win32\0";
const EXCLUSIVE_URI_PARAM_C: &[u8] = b"exclusive\0";

type VfsOpen = unsafe extern "C" fn(
    *mut ffi::sqlite3_vfs,
    ffi::sqlite3_filename,
    *mut ffi::sqlite3_file,
    c_int,
    *mut c_int,
) -> c_int;

type VfsDelete = unsafe extern "C" fn(*mut ffi::sqlite3_vfs, *const c_char, c_int) -> c_int;

type ShmMap = unsafe extern "C" fn(
    *mut ffi::sqlite3_file,
    c_int,
    c_int,
    c_int,
    *mut *mut std::ffi::c_void,
) -> c_int;

type ShmUnmap = unsafe extern "C" fn(*mut ffi::sqlite3_file, c_int) -> c_int;

static INITIALIZED: OnceLock<Result<(), String>> = OnceLock::new();
static WIN32_OPEN: OnceLock<VfsOpen> = OnceLock::new();
static WIN32_DELETE: OnceLock<VfsDelete> = OnceLock::new();
static WIN32_SHM_MAP: OnceLock<ShmMap> = OnceLock::new();
static WIN32_SHM_UNMAP: OnceLock<ShmUnmap> = OnceLock::new();
static WIN32_IO_METHODS: OnceLock<usize> = OnceLock::new();
static LODESTAR_IO_METHODS: OnceLock<Result<usize, String>> = OnceLock::new();
static STALE_SIDECAR_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Stable prefix of SQLite's bundled win32 `winFile`. `sqlite3_file`/`pMethods` must be first by
/// the VFS contract; `pVfs` and `HANDLE h` immediately follow in the win32 implementation. We
/// validate `szOsFile` before registering the adapter and never inspect any later private field.
#[repr(C)]
struct WinFilePrefix {
    base: ffi::sqlite3_file,
    vfs: *mut ffi::sqlite3_vfs,
    handle: HANDLE,
}

/// Prefix through `pShm` of the same bundled `winFile` structure.
#[repr(C)]
struct WinFileThroughShm {
    base: ffi::sqlite3_file,
    vfs: *mut ffi::sqlite3_vfs,
    handle: HANDLE,
    lock_type: u8,
    shared_lock_byte: i16,
    control_flags: u8,
    last_error: u32,
    shm: *mut WinShm,
}

#[repr(C)]
struct WinShm {
    node: *mut WinShmNodePrefix,
}

/// Only the immutable prefix of `winShmNode` needed to replace its embedded file handle.
#[repr(C)]
struct WinShmNodePrefix {
    mutex: *mut ffi::sqlite3_mutex,
    filename: *mut c_char,
    file: WinFilePrefix,
}

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

struct StagedSidecar {
    original: PathBuf,
    tombstone: PathBuf,
    renamed_handle: Option<OwnedHandle>,
}

/// Holds DELETE access to the active main file and stages both sidecars under reversible names.
/// Until `commit`, dropping the guard rolls every staged name back. Holding the main handle also
/// prevents a late non-cooperating opener from entering between sidecar staging and main-file
/// replacement.
pub(crate) struct PublicationGuard {
    _main: OwnedHandle,
    sidecars: Vec<StagedSidecar>,
    committed: bool,
}

impl PublicationGuard {
    pub(crate) fn commit(mut self) {
        self.committed = true;
        for staged in &self.sidecars {
            // Publication already committed. A driver without POSIX disposition may leave only a
            // uniquely named, unreachable tombstone; it can never alias a later active sidecar.
            let _ = remove_sidecar(&staged.tombstone);
        }
    }
}

impl Drop for PublicationGuard {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        for staged in self.sidecars.iter().rev() {
            if let Some(handle) = &staged.renamed_handle {
                let _ = rename_handle_to(&staged.original, handle.0, None);
            } else if create_hard_link(&staged.original, &staged.tombstone).is_ok() {
                let _ = remove_sidecar(&staged.tombstone);
            }
        }
    }
}

pub(crate) fn prepare_publication(
    db: &Path,
    inject_after_first: bool,
) -> std::io::Result<PublicationGuard> {
    let main = OwnedHandle(
        open_delete_handle(db)
            .map_err(|error| operation_error("CreateFileW DELETE handle", db, error))?,
    );
    let mut pending = Vec::new();
    // SHM is the file most likely to expose a non-cooperating mapped handle. Open every existing
    // target before mutating any name, so sharing violations fail without partial retirement.
    for suffix in ["-shm", "-wal"] {
        let mut os_path = db.as_os_str().to_os_string();
        os_path.push(suffix);
        let path = PathBuf::from(os_path);
        match open_delete_handle(&path) {
            Ok(handle) => pending.push((path, OwnedHandle(handle))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(operation_error(
                    "CreateFileW DELETE sidecar handle",
                    &path,
                    error,
                ));
            }
        }
    }

    let mut guard = PublicationGuard {
        _main: main,
        sidecars: Vec::new(),
        committed: false,
    };
    for (index, (original, handle)) in pending.into_iter().enumerate() {
        let (tombstone, renamed_handle) = stage_sidecar_tombstone(&original, handle)?;
        guard.sidecars.push(StagedSidecar {
            original,
            tombstone,
            renamed_handle,
        });
        if inject_after_first && index == 0 {
            let cache = db.parent().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "database path has no cache directory",
                )
            })?;
            let staged = cache.join("h03-sidecar-first-staged");
            let release = cache.join("h03-release-sidecar-first-staged");
            std::fs::write(&staged, b"staged\n")?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while !release.exists() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let released = release.exists();
            let _ = std::fs::remove_file(&staged);
            let _ = std::fs::remove_file(&release);
            if !released {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "release for staged-sidecar failpoint was not observed",
                ));
            }
            return Err(std::io::Error::other(
                "injected failure after first staged sidecar",
            ));
        }
    }
    Ok(guard)
}

/// Gives the old sidecar a private hard-link name before removing its active name. Unlike rename,
/// POSIX disposition is documented to unlink the name immediately even while the SHM remains
/// mapped. The private link keeps the bytes reachable for rollback; stale SQLite handles keep
/// using the same file object until they close.
fn stage_sidecar_tombstone(
    path: &Path,
    handle: OwnedHandle,
) -> std::io::Result<(PathBuf, Option<OwnedHandle>)> {
    let file_name = path.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "sidecar path has no file name",
        )
    })?;
    for _ in 0..16 {
        let tombstone = next_tombstone(path, file_name);
        match create_hard_link(&tombstone, path) {
            Ok(()) => {
                if let Err(error) = dispose_handle(handle.0) {
                    drop(handle);
                    let _ = remove_sidecar(&tombstone);
                    return Err(operation_error(
                        "FileDispositionInfoEx POSIX unlink",
                        path,
                        error,
                    ));
                }
                // POSIX disposition removes the opened link when this handle closes. The private
                // hard link and any stale SQLite mappings continue to reference the same bytes.
                drop(handle);
                return Ok((tombstone, None));
            }
            Err(error) => match error.raw_os_error() {
                Some(code)
                    if code == ERROR_ALREADY_EXISTS as i32 || code == ERROR_FILE_EXISTS as i32 =>
                {
                    continue;
                }
                Some(code)
                    if code == ERROR_INVALID_FUNCTION as i32
                        || code == ERROR_NOT_SUPPORTED as i32 =>
                {
                    // FAT/exFAT do not support hard links. Once the active connection has been
                    // checkpointed and closed, the classic same-directory rename remains
                    // reversible and retains compatibility with those filesystems.
                    let tombstone = rename_sidecar_tombstone(path, handle.0).map_err(|error| {
                        operation_error("FileRenameInfo fallback staging", path, error)
                    })?;
                    return Ok((tombstone, Some(handle)));
                }
                _ => return Err(operation_error("CreateHardLinkW staging", path, error)),
            },
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "no unique stale sidecar tombstone name available",
    ))
}

fn operation_error(operation: &str, path: &Path, error: std::io::Error) -> std::io::Error {
    std::io::Error::new(
        error.kind(),
        format!("{operation} for {} failed: {error}", path.display()),
    )
}

fn create_hard_link(link: &Path, existing: &Path) -> std::io::Result<()> {
    let link = wide_path(link);
    let existing = wide_path(existing);
    let created = unsafe { CreateHardLinkW(link.as_ptr(), existing.as_ptr(), ptr::null()) };
    if created == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn dispose_handle(handle: HANDLE) -> std::io::Result<()> {
    let disposition = FILE_DISPOSITION_INFO_EX {
        Flags: FILE_DISPOSITION_FLAG_DELETE
            | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS
            | FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE,
    };
    let disposed = unsafe {
        SetFileInformationByHandle(
            handle,
            FileDispositionInfoEx,
            (&disposition as *const FILE_DISPOSITION_INFO_EX).cast(),
            std::mem::size_of::<FILE_DISPOSITION_INFO_EX>() as u32,
        )
    };
    if disposed == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn open_delete_handle(path: &Path) -> std::io::Result<HANDLE> {
    let wide = wide_path(path);
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            DELETE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
            ptr::null_mut(),
        )
    };
    if handle != INVALID_HANDLE_VALUE {
        return Ok(handle);
    }
    let error = std::io::Error::last_os_error();
    match error.raw_os_error() {
        Some(code)
            if code == ERROR_FILE_NOT_FOUND as i32 || code == ERROR_PATH_NOT_FOUND as i32 =>
        {
            Err(std::io::Error::new(std::io::ErrorKind::NotFound, error))
        }
        _ => Err(error),
    }
}

pub(crate) fn open_with_flags(path: &Path, flags: OpenFlags) -> rusqlite::Result<Connection> {
    initialize().map_err(|message| {
        rusqlite::Error::SqliteFailure(
            ffi::Error::new(ffi::SQLITE_CANTOPEN),
            Some(format!("initialize {VFS_NAME}: {message}")),
        )
    })?;
    Connection::open_with_flags_and_vfs(path, flags, VFS_NAME)
}

pub(crate) fn open(path: &Path) -> rusqlite::Result<Connection> {
    open_with_flags(path, OpenFlags::default())
}

/// Owns the exact Windows file object exposed by the read-only SQLite validation connection.
/// The duplicate denies new writers while integrity/FK checks, the pause seam, and publication
/// complete, so pathname replacement cannot substitute different bytes after validation.
pub(crate) type FileIdentity = (u64, [u8; 16]);

pub(crate) struct PreparedCandidate {
    handle: OwnedHandle,
    identity: FileIdentity,
}

pub(crate) fn prepare_candidate(connection: &Connection) -> std::io::Result<PreparedCandidate> {
    let mut file: *mut ffi::sqlite3_file = ptr::null_mut();
    let result = unsafe {
        ffi::sqlite3_file_control(
            connection.handle(),
            c"main".as_ptr(),
            ffi::SQLITE_FCNTL_FILE_POINTER,
            ptr::addr_of_mut!(file).cast(),
        )
    };
    if result != ffi::SQLITE_OK || file.is_null() {
        return Err(std::io::Error::other(format!(
            "sqlite3_file_control SQLITE_FCNTL_FILE_POINTER returned {result}"
        )));
    }
    let original = unsafe { (*file.cast::<WinFilePrefix>()).handle };
    let handle = unsafe {
        ReOpenFile(
            original,
            GENERIC_READ | GENERIC_WRITE | DELETE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_WRITE_THROUGH,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error());
    }
    let identity = file_identity(handle);
    let identity = match identity {
        Ok(identity) => identity,
        Err(error) => {
            let _ = unsafe { CloseHandle(handle) };
            return Err(error);
        }
    };
    Ok(PreparedCandidate {
        handle: OwnedHandle(handle),
        identity,
    })
}

impl PreparedCandidate {
    pub(crate) fn identity(&self) -> FileIdentity {
        self.identity
    }

    pub(crate) fn sync(&self) -> std::io::Result<()> {
        if unsafe { FlushFileBuffers(self.handle.0) } == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

/// Publishes a complete candidate while readers and the rollback connection still hold the old
/// main file open. POSIX replacement keeps those handles attached to the previous file object and
/// makes later opens resolve the candidate. WRITE_THROUGH also flushes rename metadata on NTFS.
pub(crate) fn replace_durable(candidate: PreparedCandidate, active: &Path) -> std::io::Result<()> {
    rename_handle_to(
        active,
        candidate.handle.0,
        Some(FILE_RENAME_FLAG_REPLACE_IF_EXISTS | FILE_RENAME_FLAG_POSIX_SEMANTICS),
    )
    .map_err(|error| operation_error("FileRenameInfoEx POSIX replace", active, error))
}

/// Rust 1.80 implements `remove_file` with `DeleteFileW`, which rejects a mapped WAL shared-memory
/// file even when every opener supplied `FILE_SHARE_DELETE`. POSIX disposition removes the name
/// immediately while storage remains alive for stale mapped handles.
pub(crate) fn remove_sidecar(path: &Path) -> std::io::Result<()> {
    let wide = wide_path(path);
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            DELETE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error());
    }

    let disposition = FILE_DISPOSITION_INFO_EX {
        Flags: FILE_DISPOSITION_FLAG_DELETE
            | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS
            | FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE,
    };
    let disposed = unsafe {
        SetFileInformationByHandle(
            handle,
            FileDispositionInfoEx,
            (&disposition as *const FILE_DISPOSITION_INFO_EX).cast(),
            std::mem::size_of::<FILE_DISPOSITION_INFO_EX>() as u32,
        )
    };
    if disposed == 0 {
        let disposition_error = std::io::Error::last_os_error();
        // FAT/exFAT and some filesystem drivers do not implement POSIX disposition. A normal
        // same-directory handle rename is still sufficient for publication. `ReplaceIfExists=0`
        // makes reservation atomic and cannot follow or overwrite a pre-created reparse target.
        let renamed = rename_sidecar_tombstone(path, handle);
        let close_result = unsafe { CloseHandle(handle) };
        return match renamed {
            Ok(tombstone) if close_result != 0 => {
                let _ = std::fs::remove_file(tombstone);
                Ok(())
            }
            Ok(_) => Err(std::io::Error::last_os_error()),
            Err(rename_error) => Err(std::io::Error::new(
                rename_error.kind(),
                format!(
                    "POSIX sidecar disposition failed ({disposition_error}); fallback rename failed ({rename_error})"
                ),
            )),
        };
    }
    if unsafe { CloseHandle(handle) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

pub(crate) fn connection_identity(connection: &Connection) -> std::io::Result<FileIdentity> {
    file_identity(connection_main_handle(connection)?)
}

pub(crate) fn path_identity(path: &Path) -> std::io::Result<FileIdentity> {
    let path_handle = open_read_handle(path)?;
    let path_handle = OwnedHandle(path_handle);
    file_identity(path_handle.0)
}

pub(crate) fn sidecar_diagnostics(path: &Path) -> String {
    ["-wal", "-shm"]
        .into_iter()
        .map(|suffix| {
            let mut os_path = path.as_os_str().to_os_string();
            os_path.push(suffix);
            let sidecar = PathBuf::from(os_path);
            match path_identity(&sidecar) {
                Ok(identity) => format!("{suffix}=present:{identity:?}"),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    format!("{suffix}=absent")
                }
                Err(error) => format!("{suffix}=error:{error}"),
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn connection_main_handle(connection: &Connection) -> std::io::Result<HANDLE> {
    let mut file: *mut ffi::sqlite3_file = ptr::null_mut();
    let result = unsafe {
        ffi::sqlite3_file_control(
            connection.handle(),
            c"main".as_ptr(),
            ffi::SQLITE_FCNTL_FILE_POINTER,
            ptr::addr_of_mut!(file).cast(),
        )
    };
    if result != ffi::SQLITE_OK || file.is_null() {
        return Err(std::io::Error::other(format!(
            "sqlite3_file_control SQLITE_FCNTL_FILE_POINTER returned {result}"
        )));
    }
    Ok(unsafe { (*file.cast::<WinFilePrefix>()).handle })
}

fn open_read_handle(path: &Path) -> std::io::Result<HANDLE> {
    let wide = wide_path(path);
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(handle)
    }
}

fn file_identity(handle: HANDLE) -> std::io::Result<FileIdentity> {
    let mut identity = FILE_ID_INFO::default();
    let result = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileIdInfo,
            ptr::addr_of_mut!(identity).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok((identity.VolumeSerialNumber, identity.FileId.Identifier))
    }
}

fn rename_sidecar_tombstone(path: &Path, handle: HANDLE) -> std::io::Result<PathBuf> {
    let file_name = path.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "sidecar path has no file name",
        )
    })?;
    for _ in 0..16 {
        let tombstone = next_tombstone(path, file_name);
        let error = match rename_handle_to(&tombstone, handle, None) {
            Ok(()) => return Ok(tombstone),
            Err(error) => error,
        };
        match error.raw_os_error() {
            Some(code)
                if code == ERROR_ALREADY_EXISTS as i32 || code == ERROR_FILE_EXISTS as i32 =>
            {
                continue;
            }
            _ => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "no unique stale sidecar tombstone name available",
    ))
}

fn next_tombstone(path: &Path, file_name: &std::ffi::OsStr) -> PathBuf {
    let sequence = STALE_SIDECAR_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut tombstone_name = file_name.to_os_string();
    tombstone_name.push(format!(".lodestar-stale-{}-{sequence}", std::process::id()));
    path.with_file_name(tombstone_name)
}

fn rename_handle_to(
    target: &Path,
    handle: HANDLE,
    extended_flags: Option<u32>,
) -> std::io::Result<()> {
    let mut wide = wide_path(target);
    if wide.pop() != Some(0) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "rename target is not NUL terminated",
        ));
    }
    let wide = windows_nt_path::to_nt_rename_path(&wide).map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("invalid absolute rename target: {error}"),
        )
    })?;
    let name_bytes = wide.len().checked_mul(2).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "rename target is too long",
        )
    })?;
    let header_bytes = std::mem::offset_of!(FILE_RENAME_INFO, FileName);
    let total_bytes = header_bytes.checked_add(name_bytes).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "rename buffer is too large",
        )
    })?;
    let words = total_bytes.div_ceil(std::mem::size_of::<usize>());
    let mut storage = vec![0usize; words];
    let info = storage.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    unsafe {
        (*info).Anonymous.Flags = extended_flags.unwrap_or(0);
        (*info).RootDirectory = ptr::null_mut();
        (*info).FileNameLength = name_bytes as u32;
        ptr::copy_nonoverlapping(
            wide.as_ptr(),
            ptr::addr_of_mut!((*info).FileName).cast::<u16>(),
            wide.len(),
        );
    }
    let renamed = unsafe {
        SetFileInformationByHandle(handle, FileRenameInfoEx, info.cast(), total_bytes as u32)
    };
    if renamed == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn wide_path(path: &Path) -> Vec<u16> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;

        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }
    #[cfg(not(windows))]
    {
        // This branch only lets the host-side Windows API compile harness type-check the module.
        path.to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect()
    }
}

fn initialize() -> Result<(), String> {
    INITIALIZED
        .get_or_init(|| {
            let initialized = unsafe { ffi::sqlite3_initialize() };
            if initialized != ffi::SQLITE_OK {
                return Err(format!("sqlite3_initialize returned {initialized}"));
            }

            // `rusqlite` is built with its bundled SQLite, so this explicitly named VFS is the
            // concrete win32 implementation whose `winFile` prefix is compiled with the same
            // headers. Never clone an arbitrary process default VFS.
            let win32 = unsafe {
                ffi::sqlite3_vfs_find(WIN32_VFS_NAME_C.as_ptr().cast::<c_char>())
            };
            if win32.is_null() {
                return Err("bundled SQLite win32 VFS not found".into());
            }
            let version = unsafe { (*win32).iVersion };
            if version < 1 {
                return Err(format!("win32 VFS version {version} is invalid"));
            }
            let os_file_size = unsafe { (*win32).szOsFile };
            let prefix_size = std::mem::size_of::<WinFilePrefix>();
            if os_file_size < prefix_size as c_int {
                return Err(format!(
                    "win32 VFS file size {os_file_size} is smaller than required prefix {prefix_size}"
                ));
            }
            let win32_open = unsafe { (*win32).xOpen }
                .ok_or_else(|| "win32 VFS has no xOpen".to_string())?;
            WIN32_OPEN
                .set(win32_open)
                .map_err(|_| "win32 xOpen already initialized".to_string())?;
            let win32_delete = unsafe { (*win32).xDelete }
                .ok_or_else(|| "win32 VFS has no xDelete".to_string())?;
            WIN32_DELETE
                .set(win32_delete)
                .map_err(|_| "win32 xDelete already initialized".to_string())?;

            let mut lodestar = Box::new(unsafe { ptr::read(win32) });
            // Version 1 contains every callback the adapter needs. Capping it prevents SQLite from
            // interpreting a copied future tail while all v1 operations continue to delegate to
            // the known bundled win32 table and pAppData.
            lodestar.iVersion = 1;
            lodestar.pNext = ptr::null_mut();
            lodestar.zName = VFS_NAME_C.as_ptr().cast::<c_char>();
            lodestar.xOpen = Some(lodestar_open);
            lodestar.xDelete = Some(lodestar_delete);
            let lodestar = Box::into_raw(lodestar);
            let registered = unsafe { ffi::sqlite3_vfs_register(lodestar, 0) };
            if registered != ffi::SQLITE_OK {
                // Registration failed, so SQLite cannot retain this allocation.
                drop(unsafe { Box::from_raw(lodestar) });
                return Err(format!("sqlite3_vfs_register returned {registered}"));
            }
            Ok(())
        })
        .clone()
}

unsafe extern "C" fn lodestar_delete(
    vfs: *mut ffi::sqlite3_vfs,
    name: *const c_char,
    sync_dir: c_int,
) -> c_int {
    // SQLite deletes WAL by pathname when a connection closes. After Lodestar atomically replaces
    // the main database, a connection on the previous inode still carries that same pathname; its
    // late close must not unlink the new generation's WAL. SHM deletion inside winShmPurge bypasses
    // this VFS callback and is disabled independently by `lodestar_shm_unmap` below. Lodestar owns
    // retirement of both sidecars immediately before publication, while the writer gate is held.
    if !name.is_null() {
        let bytes = unsafe { CStr::from_ptr(name) }.to_bytes();
        if bytes.ends_with(b"-wal") || bytes.ends_with(b"-shm") {
            return ffi::SQLITE_OK;
        }
    }
    let delete = *WIN32_DELETE
        .get()
        .expect("named VFS is registered only after win32 xDelete is saved");
    unsafe { delete(vfs, name, sync_dir) }
}

unsafe extern "C" fn lodestar_open(
    vfs: *mut ffi::sqlite3_vfs,
    name: ffi::sqlite3_filename,
    file: *mut ffi::sqlite3_file,
    flags: c_int,
    out_flags: *mut c_int,
) -> c_int {
    let open = *WIN32_OPEN
        .get()
        .expect("named VFS is registered only after win32 xOpen is saved");
    let result = unsafe { open(vfs, name, file, flags, out_flags) };
    if result != ffi::SQLITE_OK {
        return result;
    }

    // Delete-on-close temporary files and URI `exclusive=1` deliberately use special sharing.
    // They are not published generations, so preserve the exact built-in semantics.
    let is_delete_on_close = flags & ffi::SQLITE_OPEN_DELETEONCLOSE != 0;
    let is_exclusive = !name.is_null()
        && unsafe {
            ffi::sqlite3_uri_boolean(name, EXCLUSIVE_URI_PARAM_C.as_ptr().cast::<c_char>(), 0) != 0
        };
    if is_delete_on_close || is_exclusive {
        return ffi::SQLITE_OK;
    }

    let actual_flags = if out_flags.is_null() {
        flags
    } else {
        unsafe { *out_flags }
    };
    let desired_access = if actual_flags & ffi::SQLITE_OPEN_READWRITE != 0 {
        GENERIC_READ | GENERIC_WRITE
    } else {
        GENERIC_READ
    };
    let prefix = file.cast::<WinFilePrefix>();
    let original = unsafe { (*prefix).handle };
    let reopened = unsafe {
        ReOpenFile(
            original,
            desired_access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            0,
        )
    };
    if reopened == INVALID_HANDLE_VALUE {
        return close_failed_open(file, ffi::SQLITE_CANTOPEN);
    }
    if unsafe { CloseHandle(original) } == 0 {
        let _ = unsafe { CloseHandle(reopened) };
        return close_failed_open(file, ffi::SQLITE_IOERR_CLOSE);
    }
    unsafe { (*prefix).handle = reopened };
    let methods = match wrapped_io_methods(unsafe { (*file).pMethods }) {
        Ok(methods) => methods,
        Err(()) => return close_failed_open(file, ffi::SQLITE_CANTOPEN),
    };
    unsafe { (*file).pMethods = methods };
    ffi::SQLITE_OK
}

fn wrapped_io_methods(
    methods: *const ffi::sqlite3_io_methods,
) -> Result<*const ffi::sqlite3_io_methods, ()> {
    let result = LODESTAR_IO_METHODS.get_or_init(|| {
        if methods.is_null() {
            return Err("win32 xOpen returned null io methods".into());
        }
        let version = unsafe { (*methods).iVersion };
        if version < 2 {
            return Err(format!(
                "win32 io methods version {version} has no shared-memory callbacks"
            ));
        }
        let shm_map = unsafe { (*methods).xShmMap }
            .ok_or_else(|| "win32 io methods have no xShmMap".to_string())?;
        let shm_unmap = unsafe { (*methods).xShmUnmap }
            .ok_or_else(|| "win32 io methods have no xShmUnmap".to_string())?;
        WIN32_SHM_MAP
            .set(shm_map)
            .map_err(|_| "win32 xShmMap already initialized".to_string())?;
        WIN32_SHM_UNMAP
            .set(shm_unmap)
            .map_err(|_| "win32 xShmUnmap already initialized".to_string())?;
        WIN32_IO_METHODS
            .set(methods as usize)
            .map_err(|_| "win32 io methods already initialized".to_string())?;

        let mut wrapped = Box::new(unsafe { *methods });
        wrapped.xShmMap = Some(lodestar_shm_map);
        wrapped.xShmUnmap = Some(lodestar_shm_unmap);
        Ok(Box::into_raw(wrapped) as usize)
    });
    if WIN32_IO_METHODS.get().copied() != Some(methods as usize) {
        return Err(());
    }
    result
        .as_ref()
        .map(|address| *address as *const ffi::sqlite3_io_methods)
        .map_err(|_| ())
}

unsafe extern "C" fn lodestar_shm_unmap(file: *mut ffi::sqlite3_file, _delete: c_int) -> c_int {
    let unmap = *WIN32_SHM_UNMAP
        .get()
        .expect("wrapped io methods are published only after xShmUnmap is saved");
    // `winShmPurge` calls the private winDelete directly, so xDelete cannot protect a newly
    // published generation from a stale connection's pathname-based SHM cleanup. Never ask the
    // bundled VFS to delete SHM on close; publication removes the active sidecars explicitly.
    unsafe { unmap(file, 0) }
}

unsafe extern "C" fn lodestar_shm_map(
    file: *mut ffi::sqlite3_file,
    page: c_int,
    page_size: c_int,
    extend: c_int,
    output: *mut *mut std::ffi::c_void,
) -> c_int {
    let win_file = file.cast::<WinFileThroughShm>();
    let had_shm = !unsafe { (*win_file).shm }.is_null();
    let map = *WIN32_SHM_MAP
        .get()
        .expect("wrapped io methods are published only after xShmMap is saved");
    let result = unsafe { map(file, page, page_size, extend, output) };
    if result != ffi::SQLITE_OK {
        if !had_shm && !unsafe { (*win_file).shm }.is_null() {
            return rollback_shm_open(file, output, result);
        }
        if !output.is_null() {
            unsafe { *output = ptr::null_mut() };
        }
        return result;
    }
    if had_shm {
        return result;
    }

    let shm = unsafe { (*win_file).shm };
    if shm.is_null() {
        return rollback_shm_open(file, output, ffi::SQLITE_IOERR_SHMOPEN);
    }
    let node = unsafe { (*shm).node };
    if node.is_null() || unsafe { (*node).mutex }.is_null() {
        return rollback_shm_open(file, output, ffi::SQLITE_IOERR_SHMOPEN);
    }
    // A node for the same path created by another VFS cannot be safely rewritten because it may
    // already own byte-range locks with different semantics.
    if unsafe { (*node).file.vfs != (*win_file).vfs } {
        return rollback_shm_open(file, output, ffi::SQLITE_IOERR_SHMOPEN);
    }

    unsafe { ffi::sqlite3_mutex_enter((*node).mutex) };
    let result = reopen_shared_memory_handle(node);
    unsafe { ffi::sqlite3_mutex_leave((*node).mutex) };
    if result == ffi::SQLITE_OK {
        result
    } else {
        rollback_shm_open(file, output, result)
    }
}

fn rollback_shm_open(
    file: *mut ffi::sqlite3_file,
    output: *mut *mut std::ffi::c_void,
    code: c_int,
) -> c_int {
    if !output.is_null() {
        unsafe { *output = ptr::null_mut() };
    }
    let unmap = *WIN32_SHM_UNMAP
        .get()
        .expect("wrapped io methods are published only after xShmUnmap is saved");
    let _ = unsafe { unmap(file, 0) };
    code
}

fn reopen_shared_memory_handle(node: *mut WinShmNodePrefix) -> c_int {
    let original = unsafe { (*node).file.handle };
    let share = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;

    // A DELETE-access probe succeeds only if every existing handle already shares deletion. This
    // makes the operation idempotent when several Lodestar connections reach the same node.
    let probe = unsafe { ReOpenFile(original, DELETE, share, 0) };
    if probe != INVALID_HANDLE_VALUE {
        let _ = unsafe { CloseHandle(probe) };
        return ffi::SQLITE_OK;
    }

    let mut reopened = unsafe { ReOpenFile(original, GENERIC_READ | GENERIC_WRITE, share, 0) };
    if reopened == INVALID_HANDLE_VALUE {
        reopened = unsafe { ReOpenFile(original, GENERIC_READ, share, 0) };
    }
    if reopened == INVALID_HANDLE_VALUE {
        return ffi::SQLITE_IOERR_SHMOPEN;
    }

    // winOpenSharedMemory leaves a shared dead-man-switch lock on the original handle. Acquire the
    // same shared lock on the replacement before closing the original, so there is no unlocked
    // interval and subsequent xShmLock/xShmUnmap operations continue on the replacement handle.
    const WIN_SHM_BASE: u32 = (22 + ffi::SQLITE_SHM_NLOCK as u32) * 4;
    const WIN_SHM_DMS: u32 = WIN_SHM_BASE + ffi::SQLITE_SHM_NLOCK as u32;
    let mut overlapped = OVERLAPPED::default();
    overlapped.Anonymous.Anonymous.Offset = WIN_SHM_DMS;
    let locked = unsafe {
        LockFileEx(
            reopened,
            LOCKFILE_FAIL_IMMEDIATELY,
            0,
            1,
            0,
            &mut overlapped,
        )
    };
    if locked == 0 {
        let _ = unsafe { CloseHandle(reopened) };
        return ffi::SQLITE_IOERR_SHMLOCK;
    }
    if unsafe { CloseHandle(original) } == 0 {
        let _ = unsafe { CloseHandle(reopened) };
        return ffi::SQLITE_IOERR_CLOSE;
    }
    unsafe { (*node).file.handle = reopened };
    ffi::SQLITE_OK
}

fn close_failed_open(file: *mut ffi::sqlite3_file, code: c_int) -> c_int {
    let methods = unsafe { (*file).pMethods };
    if !methods.is_null() {
        if let Some(close) = unsafe { (*methods).xClose } {
            let _ = unsafe { close(file) };
        }
        unsafe { (*file).pMethods = ptr::null() };
    }
    code
}

// The registered VFS is leaked intentionally: SQLite retains the pointer until process shutdown.
// Its copied function table and static name contain no Rust-owned references.
