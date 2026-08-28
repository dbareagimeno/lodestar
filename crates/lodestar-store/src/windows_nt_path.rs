//! Pure UTF-16 conversion from Win32 paths to the NT namespace used by rename information.

use std::fmt;

const BACKSLASH: u16 = b'\\' as u16;
const COLON: u16 = b':' as u16;
const QUESTION: u16 = b'?' as u16;
const NT_PREFIX: &[u16] = &[BACKSLASH, QUESTION, QUESTION, BACKSLASH];
const VERBATIM_PREFIX: &[u16] = &[BACKSLASH, BACKSLASH, QUESTION, BACKSLASH];
const UNC_PREFIX: &[u16] = &[BACKSLASH, BACKSLASH];
const UNC_COMPONENT: &[u16] = &[b'U' as u16, b'N' as u16, b'C' as u16, BACKSLASH];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NtRenamePathError(&'static str);

impl fmt::Display for NtRenamePathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for NtRenamePathError {}

/// Converts an absolute DOS, UNC or verbatim path to an unterminated NT rename path.
///
/// `FILE_RENAME_INFO.FileNameLength` is measured in bytes and excludes a terminal NUL, so this
/// helper rejects embedded NULs and never appends one to its output.
pub(crate) fn to_nt_rename_path(path: &[u16]) -> Result<Vec<u16>, NtRenamePathError> {
    if path.is_empty() || path.contains(&0) {
        return Err(NtRenamePathError(
            "rename target must be an absolute NT path without NULs",
        ));
    }

    if let Some(rest) = path.strip_prefix(NT_PREFIX) {
        validate_nt_tail(rest)?;
        return Ok(path.to_vec());
    }

    if let Some(rest) = path.strip_prefix(VERBATIM_PREFIX) {
        let mut converted = NT_PREFIX.to_vec();
        if let Some(unc) = strip_ascii_case_prefix(rest, UNC_COMPONENT) {
            validate_unc_tail(unc)?;
            converted.extend_from_slice(UNC_COMPONENT);
            converted.extend_from_slice(unc);
        } else {
            validate_drive_absolute(rest)?;
            converted.extend_from_slice(rest);
        }
        return Ok(converted);
    }

    if let Some(unc) = path.strip_prefix(UNC_PREFIX) {
        validate_unc_tail(unc)?;
        let mut converted = NT_PREFIX.to_vec();
        converted.extend_from_slice(UNC_COMPONENT);
        converted.extend_from_slice(unc);
        return Ok(converted);
    }

    validate_drive_absolute(path)?;
    let mut converted = NT_PREFIX.to_vec();
    converted.extend_from_slice(path);
    Ok(converted)
}

fn validate_nt_tail(path: &[u16]) -> Result<(), NtRenamePathError> {
    if let Some(unc) = strip_ascii_case_prefix(path, UNC_COMPONENT) {
        validate_unc_tail(unc)
    } else {
        validate_drive_absolute(path)
    }
}

fn validate_drive_absolute(path: &[u16]) -> Result<(), NtRenamePathError> {
    let drive = path.first().copied().unwrap_or_default();
    if path.len() >= 3 && is_ascii_letter(drive) && path[1] == COLON && path[2] == BACKSLASH {
        Ok(())
    } else {
        Err(NtRenamePathError(
            "rename target must be an absolute drive, UNC or NT path",
        ))
    }
}

fn validate_unc_tail(path: &[u16]) -> Result<(), NtRenamePathError> {
    let Some(server_end) = path.iter().position(|word| *word == BACKSLASH) else {
        return Err(NtRenamePathError(
            "rename target must be an absolute UNC path with server and share",
        ));
    };
    let share = &path[server_end + 1..];
    let share_end = share
        .iter()
        .position(|word| *word == BACKSLASH)
        .unwrap_or(share.len());
    if server_end == 0 || share_end == 0 {
        Err(NtRenamePathError(
            "rename target must be an absolute UNC path with server and share",
        ))
    } else {
        Ok(())
    }
}

fn strip_ascii_case_prefix<'a>(path: &'a [u16], prefix: &[u16]) -> Option<&'a [u16]> {
    if path.len() < prefix.len()
        || !path
            .iter()
            .zip(prefix)
            .all(|(actual, expected)| ascii_lower(*actual) == ascii_lower(*expected))
    {
        None
    } else {
        Some(&path[prefix.len()..])
    }
}

fn is_ascii_letter(word: u16) -> bool {
    matches!(word, 0x41..=0x5a | 0x61..=0x7a)
}

fn ascii_lower(word: u16) -> u16 {
    if matches!(word, 0x41..=0x5a) {
        word + 0x20
    } else {
        word
    }
}
