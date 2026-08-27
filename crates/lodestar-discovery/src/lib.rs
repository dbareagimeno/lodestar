//! Discovery compacta compartida por workspace y store.

use std::collections::BTreeSet;
use std::path::Path;

use ignore::overrides::{Override, OverrideBuilder};
use ignore::WalkBuilder;
use lodestar_core::types::{Check, CheckCode, RelPath, Severity};
use serde::Deserialize;
use thiserror::Error;

pub const CONTROL_PLANE_EXCLUDE: &str = ".lodestar/**";
pub const LODESTAR_IGNORE_FILENAME: &str = ".lodestarignore";
pub const DEFAULT_MAX_DOCUMENT_BYTES: usize = 10 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryPolicy {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub respect_gitignore: bool,
    pub respect_lodestar_ignore: bool,
    pub follow_symlinks: bool,
    pub max_document_bytes: usize,
}

impl Default for DiscoveryPolicy {
    fn default() -> Self {
        Self {
            include: vec!["**/*.md".into()],
            exclude: vec![".git/**".into(), CONTROL_PLANE_EXCLUDE.into()],
            respect_gitignore: true,
            respect_lodestar_ignore: true,
            follow_symlinks: false,
            max_document_bytes: DEFAULT_MAX_DOCUMENT_BYTES,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct DiscoveredInventory {
    pub documents: Vec<RelPath>,
    pub other_files: BTreeSet<RelPath>,
    /// Directorios recorridos en la fase de inventario. Permiten detectar altas, bajas y renames
    /// antes de leer cuerpos sin ejecutar un segundo walker.
    pub directories: Vec<RelPath>,
    /// Fingerprint capturado durante esta misma pasada canónica. El store lo compara antes de
    /// leer cuerpos: así el inventario no puede quedar obsoleto entre discovery y proyección.
    pub root_fingerprint: DiscoveryFingerprint,
    /// Fingerprint de la frontera real que recorre el walker cuando `root` es un enlace simbólico.
    /// `root_fingerprint` conserva la identidad del enlace; esta segunda huella evita que una alta
    /// dentro del destino seguido quede invisible para el store.
    pub root_target_fingerprint: DiscoveryFingerprint,
    pub directory_fingerprints: std::collections::BTreeMap<RelPath, DiscoveryFingerprint>,
    /// Fingerprints of every file entry admitted to `documents` or `other_files` during the
    /// canonical walk. They are part of the snapshot, not recaptured by the store later.
    pub entry_fingerprints: std::collections::BTreeMap<RelPath, DiscoveryFingerprint>,
    pub diagnostics: Vec<Check>,
}

/// Identidad observable de un fichero o directorio para cerrar la ventana entre las dos pasadas.
/// Se mantiene en discovery para que el snapshot canónico viaje junto con sus paths, sin que el
/// store tenga que ejecutar otro walker.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiscoveryFingerprint {
    pub kind: u8,
    pub size: i64,
    pub mtime_ns: i128,
    pub identity: u64,
    pub ctime_ns: i128,
}

#[derive(Debug, Error)]
pub enum DiscoveryError {
    #[error("discovery: {0}")]
    Io(String),
    #[error("política de discovery inválida: {0}")]
    Policy(String),
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct ConfigFile {
    #[serde(default)]
    discovery: DiscoverySection,
}

/// Sección `discovery` de la configuración efectiva de un workspace.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct DiscoverySection {
    /// Globs de documentos admitidos.
    pub include: Vec<String>,
    /// Globs excluidos; tienen precedencia sobre `include`.
    pub exclude: Vec<String>,
    /// Respeta `.gitignore`.
    pub respect_gitignore: bool,
    /// Respeta `.lodestarignore`.
    pub respect_lodestar_ignore: bool,
    /// Sigue enlaces simbólicos.
    pub follow_symlinks: bool,
    /// Tamaño máximo por documento en bytes.
    pub max_document_bytes: usize,
}

impl Default for DiscoverySection {
    fn default() -> Self {
        let p = DiscoveryPolicy::default();
        Self {
            include: p.include,
            exclude: p.exclude,
            respect_gitignore: p.respect_gitignore,
            respect_lodestar_ignore: p.respect_lodestar_ignore,
            follow_symlinks: p.follow_symlinks,
            max_document_bytes: p.max_document_bytes,
        }
    }
}

impl DiscoverySection {
    /// Convierte la sección declarada en la policy efectiva e inyecta el suelo duro del plano de
    /// control. La configuración puede añadir exclusiones, pero nunca reabrir `.lodestar/**`.
    pub fn policy(&self) -> DiscoveryPolicy {
        let mut exclude = self.exclude.clone();
        if !exclude
            .iter()
            .any(|pattern| pattern == CONTROL_PLANE_EXCLUDE)
        {
            exclude.push(CONTROL_PLANE_EXCLUDE.into());
        }
        DiscoveryPolicy {
            include: self.include.clone(),
            exclude,
            respect_gitignore: self.respect_gitignore,
            respect_lodestar_ignore: self.respect_lodestar_ignore,
            follow_symlinks: self.follow_symlinks,
            max_document_bytes: self.max_document_bytes,
        }
    }
}

pub fn load_policy(root: &Path) -> Result<DiscoveryPolicy, DiscoveryError> {
    let path = root.join(".lodestar/config.yaml");
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(DiscoveryPolicy::default())
        }
        Err(error) => return Err(DiscoveryError::Io(error.to_string())),
    };
    let config: ConfigFile =
        serde_yaml::from_str(&raw).map_err(|error| DiscoveryError::Policy(error.to_string()))?;
    Ok(config.discovery.policy())
}

pub fn discover_inventory(
    root: &Path,
    policy: &DiscoveryPolicy,
) -> Result<DiscoveredInventory, DiscoveryError> {
    let root_fingerprint = filesystem_fingerprint(root, false)?;
    let root_target_fingerprint = filesystem_fingerprint(root, true)?;
    let include = build_overrides(root, &policy.include, false)?;
    let excludes = build_overrides(root, &policy.exclude, true)?;
    let mut builder = WalkBuilder::new(root);
    builder
        .overrides(excludes)
        .hidden(false)
        .follow_links(policy.follow_symlinks)
        .git_ignore(policy.respect_gitignore)
        .require_git(false)
        .parents(false)
        .git_global(false)
        .git_exclude(false)
        .sort_by_file_name(|a, b| a.cmp(b));
    if policy.respect_lodestar_ignore {
        builder.add_custom_ignore_filename(LODESTAR_IGNORE_FILENAME);
    }
    let mut documents = Vec::new();
    let mut other_files = BTreeSet::new();
    let mut directories = Vec::new();
    let mut directory_fingerprints = std::collections::BTreeMap::new();
    let mut entry_fingerprints = std::collections::BTreeMap::new();
    let mut diagnostics = Vec::new();
    for entry in builder.build() {
        let Ok(entry) = entry else { continue };
        if entry.depth() == 0 {
            continue;
        }
        let path = entry.path();
        let relative = path.strip_prefix(root).unwrap_or(path);
        let relative_text = relative.to_string_lossy();
        let relative_text = normalize_native_separator(&relative_text);
        let Ok(rel) = RelPath::new(&relative_text) else {
            diagnostics.push(Check::new(
                Severity::Warn,
                CheckCode::PathNotUtf8,
                format!(
                    "«{}» no es una ruta representable: el documento no entra en el inventario",
                    relative.display()
                ),
                Vec::new(),
            ));
            continue;
        };
        if entry.file_type().is_some_and(|kind| kind.is_dir()) {
            directories.push(rel);
            let fingerprint = filesystem_fingerprint(path, false)?;
            directory_fingerprints.insert(
                directories
                    .last()
                    .expect("directorio recién insertado")
                    .clone(),
                fingerprint,
            );
            continue;
        }
        entry_fingerprints.insert(rel.clone(), filesystem_fingerprint(path, false)?);
        if entry.file_type().is_some_and(|kind| kind.is_symlink())
            && path.metadata().is_ok_and(|metadata| metadata.is_dir())
        {
            diagnostics.push(Check::new(
                Severity::Warn,
                CheckCode::SymlinkUnsupported,
                format!("«{}» es un enlace simbólico a un directorio", rel.as_str()),
                vec![rel.clone()],
            ));
            other_files.insert(rel);
            continue;
        }
        let included = include.matched(path, false).is_whitelist()
            || (path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
                && policy.include.iter().any(|pattern| pattern == "**/*.md"));
        if !included {
            other_files.insert(rel);
            continue;
        }
        if entry.file_type().is_some_and(|kind| kind.is_symlink()) {
            diagnostics.push(Check::new(
                Severity::Warn,
                CheckCode::SymlinkUnsupported,
                format!(
                    "«{}» es un enlace simbólico: Lodestar no sigue symlinks",
                    rel.as_str()
                ),
                vec![rel.clone()],
            ));
            other_files.insert(rel);
            continue;
        }
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            other_files.insert(rel);
            continue;
        }
        let size = entry.metadata().ok().map(|metadata| metadata.len());
        if size.is_some_and(|size| size > policy.max_document_bytes as u64) {
            diagnostics.push(Check::new(
                Severity::Warn,
                CheckCode::DocTooLarge,
                format!("«{}» supera el tamaño máximo por documento", rel.as_str()),
                vec![rel.clone()],
            ));
            other_files.insert(rel);
            continue;
        }
        // `include` define el inventario de documentos, no la extensión.  En particular,
        // `include: ["**/*"]` admite cualquier fichero regular UTF-8, igual que el descubrimiento
        // canónico del workspace; la extensión solo es una convención de la policy por defecto.
        documents.push(rel);
    }
    // `documents` is the first-pass set of candidates admitted by policy and size.  The body is
    // deliberately not opened here: UTF-8 classification belongs to the single read in the
    // consumer's second pass (workspace or store).  Consequently case collisions and the empty
    // workspace diagnostic are finalized by that consumer over the actually valid documents.
    documents.sort();
    diagnostics.sort_by(|left, right| {
        left.code
            .as_str()
            .cmp(right.code.as_str())
            .then_with(|| left.msg.cmp(&right.msg))
    });
    if filesystem_fingerprint(root, false)? != root_fingerprint {
        return Err(DiscoveryError::Io(
            "discovery inventory changed at workspace root".into(),
        ));
    }
    if filesystem_fingerprint(root, true)? != root_target_fingerprint {
        return Err(DiscoveryError::Io(
            "discovery inventory changed at workspace root target".into(),
        ));
    }
    for (directory, expected) in &directory_fingerprints {
        let current = filesystem_fingerprint(&root.join(directory.as_str()), false)?;
        if current != *expected {
            return Err(DiscoveryError::Io(format!(
                "discovery inventory changed at directory {}",
                directory.as_str()
            )));
        }
    }
    for (entry, expected) in &entry_fingerprints {
        let current = filesystem_fingerprint(&root.join(entry.as_str()), false)?;
        if current != *expected {
            return Err(DiscoveryError::Io(format!(
                "discovery inventory changed at entry {}",
                entry.as_str()
            )));
        }
    }
    Ok(DiscoveredInventory {
        documents,
        other_files,
        directories,
        root_fingerprint,
        root_target_fingerprint,
        directory_fingerprints,
        entry_fingerprints,
        diagnostics,
    })
}

/// Captures filesystem identity and change time without opening a payload. `follow_target` is
/// false for the directory entries returned by the walker (including reparse points) and true
/// for the target boundary used when the workspace root itself is a symlink.
pub fn filesystem_fingerprint(
    path: &Path,
    follow_target: bool,
) -> Result<DiscoveryFingerprint, DiscoveryError> {
    #[cfg(unix)]
    {
        let metadata = if follow_target {
            std::fs::metadata(path)
        } else {
            std::fs::symlink_metadata(path)
        }
        .map_err(|error| DiscoveryError::Io(format!("{}: {error}", path.display())))?;
        let file_type = metadata.file_type();
        let kind = if file_type.is_file() {
            1
        } else if file_type.is_dir() {
            2
        } else if file_type.is_symlink() {
            3
        } else {
            4
        };
        let mtime_ns = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos() as i128)
            .unwrap_or(0);
        use std::os::unix::fs::MetadataExt;
        let ctime_ns = (metadata.ctime() as i128)
            .saturating_mul(1_000_000_000)
            .saturating_add(metadata.ctime_nsec() as i128);
        Ok(DiscoveryFingerprint {
            kind,
            size: metadata.len() as i64,
            mtime_ns,
            identity: ((metadata.dev() as u64).rotate_left(17)) ^ metadata.ino() as u64,
            ctime_ns,
        })
    }
    #[cfg(windows)]
    {
        windows_handle_fingerprint(path, follow_target)
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(DiscoveryError::Io(
            "filesystem fingerprints are unsupported on this platform".into(),
        ))
    }
}

#[cfg(windows)]
fn windows_handle_fingerprint(
    path: &Path,
    follow_target: bool,
) -> Result<DiscoveryFingerprint, DiscoveryError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FileBasicInfo, FileIdInfo, GetFileInformationByHandle,
        GetFileInformationByHandleEx, BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_BASIC_INFO, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };

    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut flags = FILE_FLAG_BACKUP_SEMANTICS;
    if !follow_target {
        flags |= FILE_FLAG_OPEN_REPARSE_POINT;
    }
    // The handle is metadata-only: sharing all access modes prevents a concurrent writer from
    // being blocked while the fingerprint is captured.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            flags,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(DiscoveryError::Io(format!(
            "{}: {}",
            path.display(),
            std::io::Error::last_os_error()
        )));
    }
    let result = (|| {
        // These values all come from the same handle. The no-follow path opens the reparse point
        // itself; the target path follows it. This avoids mixing a path metadata race with the
        // identity/change-time observation used by TOCTOU validation.
        let mut by_handle = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::zeroed();
        let by_handle_ok = unsafe { GetFileInformationByHandle(handle, by_handle.as_mut_ptr()) };
        if by_handle_ok == 0 {
            return Err(DiscoveryError::Io(format!(
                "{}: no se pudo capturar la información del handle: {}",
                path.display(),
                std::io::Error::last_os_error()
            )));
        }
        let by_handle = unsafe { by_handle.assume_init() };
        let kind = if by_handle.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            3
        } else if by_handle.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
            2
        } else {
            1
        };
        let size = (u64::from(by_handle.nFileSizeHigh) << 32 | u64::from(by_handle.nFileSizeLow))
            .try_into()
            .map_err(|_| {
                DiscoveryError::Io(format!("{}: tamaño de fichero inválido", path.display()))
            })?;
        let filetime_to_unix_ns = |time: windows_sys::Win32::Foundation::FILETIME| {
            let ticks = (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
            i128::from(ticks)
                .saturating_sub(116_444_736_000_000_000_i128)
                .saturating_mul(100)
        };
        let mtime_ns = filetime_to_unix_ns(by_handle.ftLastWriteTime);

        let mut id = std::mem::MaybeUninit::<FILE_ID_INFO>::zeroed();
        let id_ok = unsafe {
            GetFileInformationByHandleEx(
                handle,
                FileIdInfo,
                id.as_mut_ptr().cast(),
                std::mem::size_of::<FILE_ID_INFO>() as u32,
            )
        };
        if id_ok == 0 {
            return Err(DiscoveryError::Io(format!(
                "{}: no se pudo capturar FileIdInfo: {}",
                path.display(),
                std::io::Error::last_os_error()
            )));
        }
        let id = unsafe { id.assume_init() };
        let mut identity = id.VolumeSerialNumber;
        for (index, byte) in id.FileId.Identifier.iter().enumerate() {
            identity ^= u64::from(*byte).rotate_left((index as u32 * 5) % 64);
            identity = identity.wrapping_mul(0x100000001b3);
        }
        if identity == 0 {
            return Err(DiscoveryError::Io(format!(
                "{}: FileIdInfo devolvió una identidad vacía",
                path.display()
            )));
        }

        let mut basic = std::mem::MaybeUninit::<FILE_BASIC_INFO>::zeroed();
        let basic_ok = unsafe {
            GetFileInformationByHandleEx(
                handle,
                FileBasicInfo,
                basic.as_mut_ptr().cast(),
                std::mem::size_of::<FILE_BASIC_INFO>() as u32,
            )
        };
        if basic_ok == 0 {
            return Err(DiscoveryError::Io(format!(
                "{}: no se pudo capturar FileBasicInfo: {}",
                path.display(),
                std::io::Error::last_os_error()
            )));
        }
        let change_time_ticks = unsafe { basic.assume_init() }.ChangeTime;
        if change_time_ticks == 0 {
            return Err(DiscoveryError::Io(format!(
                "{}: FileBasicInfo devolvió un ChangeTime vacío",
                path.display()
            )));
        }
        // FILE_BASIC_INFO uses 100 ns ticks since 1601, whereas the portable fingerprint names
        // this field in Unix-epoch nanoseconds.
        let ctime_ns = i128::from(change_time_ticks)
            .saturating_sub(116_444_736_000_000_000_i128)
            .saturating_mul(100);
        Ok(DiscoveryFingerprint {
            kind,
            size,
            mtime_ns,
            identity,
            ctime_ns,
        })
    })();
    unsafe { CloseHandle(handle) };
    result
}

/// `Path` uses backslashes as separators on Windows, but a backslash is a literal POSIX
/// filename character.  Keep the conversion at the OS boundary; doing it unconditionally would
/// turn a real Unix path such as `a\\b.md` into the unrelated path `a/b.md`.
fn normalize_native_separator(path: &str) -> String {
    #[cfg(windows)]
    {
        path.replace('\\', "/")
    }
    #[cfg(not(windows))]
    {
        path.to_string()
    }
}

fn build_overrides(
    root: &Path,
    patterns: &[String],
    excludes: bool,
) -> Result<Override, DiscoveryError> {
    let mut builder = OverrideBuilder::new(root);
    for pattern in patterns {
        if excludes {
            if let Some(dir) = pattern.strip_suffix("/**") {
                builder
                    .add(&format!("!{dir}"))
                    .map_err(|error| DiscoveryError::Policy(error.to_string()))?;
            }
            builder
                .add(&format!("!{pattern}"))
                .map_err(|error| DiscoveryError::Policy(error.to_string()))?;
        } else {
            builder
                .add(pattern)
                .map_err(|error| DiscoveryError::Policy(error.to_string()))?;
        }
    }
    builder
        .build()
        .map_err(|error| DiscoveryError::Policy(error.to_string()))
}
