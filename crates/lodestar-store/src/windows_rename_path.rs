//! Validation for the Win32 path stored in `FILE_RENAME_INFO.FileName`.

use std::fmt;

const BACKSLASH: u16 = b'\\' as u16;
const COLON: u16 = b':' as u16;
const QUESTION: u16 = b'?' as u16;
const OBJECT_MANAGER_PREFIX: &[u16] = &[BACKSLASH, QUESTION, QUESTION, BACKSLASH];
const UNC_PREFIX: &[u16] = &[BACKSLASH, BACKSLASH];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Win32RenamePathError(&'static str);

impl fmt::Display for Win32RenamePathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for Win32RenamePathError {}

/// Validates an unterminated absolute Win32 drive or UNC path without changing its UTF-16 words.
///
/// `FILE_RENAME_INFO.FileNameLength` excludes a terminal NUL. The caller therefore removes the
/// terminator before validation and copies these exact words into `FileName`.
pub(crate) fn validate_win32_rename_path(path: &[u16]) -> Result<(), Win32RenamePathError> {
    if path.is_empty() || path.contains(&0) {
        return Err(Win32RenamePathError(
            "rename target must be an absolute Win32 drive or UNC path without NULs",
        ));
    }

    if path.starts_with(OBJECT_MANAGER_PREFIX) {
        return Err(Win32RenamePathError(
            "Object Manager paths are not valid Win32 rename targets",
        ));
    }

    if let Some(unc) = path.strip_prefix(UNC_PREFIX) {
        return validate_unc_tail(unc);
    }

    validate_drive_absolute(path)
}

fn validate_drive_absolute(path: &[u16]) -> Result<(), Win32RenamePathError> {
    let drive = path.first().copied().unwrap_or_default();
    if path.len() >= 3 && is_ascii_letter(drive) && path[1] == COLON && path[2] == BACKSLASH {
        Ok(())
    } else {
        Err(Win32RenamePathError(
            "rename target must be an absolute Win32 drive or UNC path",
        ))
    }
}

fn validate_unc_tail(path: &[u16]) -> Result<(), Win32RenamePathError> {
    let Some(server_end) = path.iter().position(|word| *word == BACKSLASH) else {
        return Err(Win32RenamePathError(
            "rename target must be an absolute Win32 UNC path with server and share",
        ));
    };
    let share = &path[server_end + 1..];
    let share_end = share
        .iter()
        .position(|word| *word == BACKSLASH)
        .unwrap_or(share.len());
    if server_end == 0 || share_end == 0 {
        Err(Win32RenamePathError(
            "rename target must be an absolute Win32 UNC path with server and share",
        ))
    } else {
        Ok(())
    }
}

fn is_ascii_letter(word: u16) -> bool {
    matches!(word, 0x41..=0x5a | 0x61..=0x7a)
}
