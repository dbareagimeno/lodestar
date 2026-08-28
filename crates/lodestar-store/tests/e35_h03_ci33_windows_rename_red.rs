//! E35-H03 CI33 — reproducción Win32 acotada de la publicación por handle.
//!
//! El test incluye el mismo adaptador privado que compila `lodestar-store` y separa el cwd del
//! directorio destino. Así distingue un rename realmente anclado a `active` de un éxito nativo
//! que resuelva `index.db` fuera del directorio de la cache.

#[cfg(windows)]
#[allow(dead_code)]
#[path = "../src/windows_vfs.rs"]
mod windows_vfs;

#[cfg(not(windows))]
const WINDOWS_VFS_SOURCE: &str = include_str!("../src/windows_vfs.rs");
#[cfg(not(windows))]
const HARNESS_SOURCE: &str = include_str!("e35_h03_ci33_windows_rename_red.rs");

#[cfg(windows)]
struct CurrentDirGuard(std::path::PathBuf);

#[cfg(windows)]
impl CurrentDirGuard {
    fn switch_to(path: &std::path::Path) -> Self {
        let original = std::env::current_dir().expect("cwd original");
        std::env::set_current_dir(path).expect("separar cwd del target de cache");
        Self(original)
    }
}

#[cfg(windows)]
impl Drop for CurrentDirGuard {
    fn drop(&mut self) {
        std::env::set_current_dir(&self.0).expect("restaurar cwd tras el syscall");
    }
}

#[cfg(windows)]
fn sqlite_with_sentinel(path: &std::path::Path, sentinel: &str) -> rusqlite::Connection {
    let connection =
        windows_vfs::open(path).expect("abrir SQLite mediante el VFS real de Lodestar");
    connection
        .execute_batch(&format!(
            "PRAGMA journal_mode=DELETE; CREATE TABLE generation(value TEXT NOT NULL); INSERT INTO generation VALUES ('{sentinel}');"
        ))
        .expect("crear una generación SQLite autocontenida");
    connection
}

#[cfg(windows)]
fn sentinel(connection: &rusqlite::Connection) -> String {
    connection
        .query_row("SELECT value FROM generation", [], |row| row.get(0))
        .expect("leer el sentinela de generación")
}

/// C5/C6 — con la generación anterior todavía abierta, el único `FileRenameInfoEx` debe mover el
/// mismo FILE_ID que pasó validación al `active` usando su destino Win32 absoluto preservado. Una apertura
/// posterior ve el candidato, el handle antiguo conserva el snapshot anterior y un `index.db`
/// homónimo bajo el cwd —incluso en otra unidad— queda byte a byte intacto.
#[cfg(windows)]
#[test]
fn c5_c6_win32_publica_candidate_file_id_en_active_sin_tocar_cwd_homonimo() {
    let sandbox = tempfile::tempdir().expect("sandbox Win32");
    let cache = sandbox.path().join("cache");
    let unrelated_cwd = sandbox.path().join("cwd");
    std::fs::create_dir_all(&cache).expect("directorio de cache");
    std::fs::create_dir_all(&unrelated_cwd).expect("cwd homónimo");

    let active = cache.join("index.db");
    let next = cache.join("index.db.next");
    let cwd_homonym = unrelated_cwd.join("index.db");
    std::fs::write(&cwd_homonym, b"CI33-CWD-MUST-NOT-CHANGE\n")
        .expect("centinela homónimo fuera de la cache");

    let old = sqlite_with_sentinel(&active, "old-generation");
    let candidate_connection = sqlite_with_sentinel(&next, "candidate-generation");
    let candidate = windows_vfs::prepare_candidate(&candidate_connection)
        .expect("fijar por handle el objeto candidato validado");
    let candidate_id = candidate.identity();
    drop(candidate_connection);
    candidate
        .sync()
        .expect("sincronizar candidato antes del swap");

    let publication = {
        let _cwd_guard = CurrentDirGuard::switch_to(&unrelated_cwd);
        windows_vfs::replace_durable(candidate, &active)
    };
    publication.expect("C5: el swap atómico del candidato válido debe publicarse");

    let active_id = windows_vfs::path_identity(&active)
        .expect("identificar el pathname publicado con un handle nuevo");
    assert_eq!(
        active_id, candidate_id,
        "rojo causal CI33: FileRenameInfoEx devolvió éxito pero active conserva otro FILE_ID; candidate={candidate_id:?}; active={active_id:?}"
    );

    let published = windows_vfs::open(&active).expect("abrir la generación por el pathname final");
    assert_eq!(
        sentinel(&published),
        "candidate-generation",
        "C5: toda apertura nueva debe observar exactamente el snapshot candidato"
    );
    assert_eq!(
        sentinel(&old),
        "old-generation",
        "C6: el handle previo debe conservar consultable el snapshot anterior"
    );
    assert_eq!(
        std::fs::read(&cwd_homonym).expect("releer homónimo del cwd"),
        b"CI33-CWD-MUST-NOT-CHANGE\n",
        "C6: publicar la cache no puede reemplazar un pathname homónimo resuelto contra el cwd"
    );
}

/// CI41 — `prepare_candidate` reabre el mismo objeto mientras la conexión SQLite escribible sigue
/// viva. Windows exige que el nuevo handle comparta también escritura: omitir `FILE_SHARE_WRITE`
/// produce `ERROR_SHARING_VIOLATION` antes de poder fijar la identidad validada.
#[cfg(not(windows))]
#[test]
fn ci41_prepare_candidate_reopen_comparte_lectura_escritura_y_borrado() {
    let source = WINDOWS_VFS_SOURCE.replace("\r\n", "\n");
    let start = source
        .find("pub(crate) fn prepare_candidate(")
        .expect("guarda anti-vacuidad: existe prepare_candidate");
    let tail = &source[start..];
    let end = tail
        .find("\nimpl PreparedCandidate {")
        .expect("guarda anti-vacuidad: termina prepare_candidate");
    let prepare_candidate = &tail[..end];

    assert_eq!(
        prepare_candidate.matches("ReOpenFile(").count(),
        1,
        "guarda anti-vacuidad: prepare_candidate debe tener exactamente un ReOpenFile"
    );
    assert!(
        prepare_candidate.contains("GENERIC_READ | GENERIC_WRITE | DELETE"),
        "guarda anti-vacuidad: la reapertura candidata debe coexistir con el handle SQLite escribible"
    );

    let compact: String = prepare_candidate
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect();
    assert!(
        !compact.contains(concat!(
            "ReOpenFile(original,",
            "GENERIC_READ|GENERIC_WRITE|DELETE,",
            "FILE_SHARE_READ|FILE_SHARE_DELETE,"
        )),
        "rojo causal CI41: prepare_candidate omite FILE_SHARE_WRITE y Windows rechaza la reapertura escribible con ERROR_SHARING_VIOLATION"
    );
    assert!(
        compact.contains(concat!(
            "ReOpenFile(original,",
            "GENERIC_READ|GENERIC_WRITE|DELETE,",
            "FILE_SHARE_READ|FILE_SHARE_WRITE|FILE_SHARE_DELETE,",
            "FILE_FLAG_OPEN_REPARSE_POINT|FILE_FLAG_WRITE_THROUGH,)"
        )),
        "rojo causal CI41: ReOpenFile solicita escritura mientras la conexión SQLite sigue abierta, pero prepare_candidate no usa la máscara completa FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE"
    );
}

/// El mismo criterio se ejecuta en hosts no Windows como guarda estructural del artefacto que el
/// harness nativo compilará: un único syscall, destino Win32 absoluto preservado y
/// `RootDirectory = NULL`.
#[cfg(not(windows))]
#[test]
fn c5_c6_win32_publica_candidate_file_id_en_active_sin_tocar_cwd_homonimo() {
    let harness_declarations = HARNESS_SOURCE
        .split("#[cfg(not(windows))]\nconst WINDOWS_VFS_SOURCE")
        .next()
        .expect("guarda anti-vacuidad: prefijo de declaraciones del harness");
    assert_eq!(
        harness_declarations.matches("#[allow(dead_code)]").count(),
        1,
        "CI34: la excepción dead_code debe existir exactamente una vez y solo en el módulo privado del harness"
    );
    assert!(
        harness_declarations.contains(
            "#[cfg(windows)]\n#[allow(dead_code)]\n#[path = \"../src/windows_vfs.rs\"]\nmod windows_vfs;"
        ) && !harness_declarations.contains("#![allow(dead_code)]"),
        "CI34: dead_code debe acotarse al módulo windows_vfs incluido, nunca al crate de test"
    );

    let start = WINDOWS_VFS_SOURCE
        .find("fn rename_handle_to(")
        .expect("guarda anti-vacuidad: existe el helper real de publicación Win32");
    let tail = &WINDOWS_VFS_SOURCE[start..];
    let end = tail
        .find("\nfn wide_path(path: &Path)")
        .expect("guarda anti-vacuidad: termina la sección de rename Win32");
    let rename = &tail[..end];

    assert_eq!(
        rename.matches("SetFileInformationByHandle(").count(),
        1,
        "rojo causal CI33: la topología actual aún permite un relative-primary y un segundo syscall; la publicación C5 debe ser un único swap atómico"
    );
    assert!(
        rename.contains("wide_path(target)")
            && rename.contains("windows_rename_path::validate_win32_rename_path(&wide)")
            && rename.contains("(*info).RootDirectory = ptr::null_mut()")
            && !rename.contains("(*info).RootDirectory = parent_handle")
            && !rename.contains("CreateFileW(")
            && !rename.contains("target.parent()")
            && !rename.contains("target.file_name()")
            && !rename.contains("wide_path(Path::new(file_name))")
            && !rename.contains(r#"\??\"#),
        "C5/C6: el artefacto Win32 debe preservar active como destino Win32 absoluto con RootDirectory=NULL, sin prefijo Object Manager"
    );
}
