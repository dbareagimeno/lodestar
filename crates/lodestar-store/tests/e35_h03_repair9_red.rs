//! E35-H03 C2/C4 — la ruta canónica no debe leer cuerpos durante el inventario.
//!
//! `discover_inventory` conserva paths y metadata sin abrir candidatos; el store abre cada cuerpo
//! una sola vez para validarlo y proyectarlo. El informe cuenta esa operación
//! (`documents_read = N`), y este test la contrasta con el seam de auditoría de la lectura de
//! proyección y, donde el sistema la actualiza sin demora, con una señal de acceso al cuerpo entre
//! ambas fases.

use std::fs;
#[cfg(not(windows))]
use std::fs::{File, FileTimes};
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(windows)]
use std::os::windows::io::{FromRawHandle, OwnedHandle, RawHandle};
use std::path::Path;
use std::sync::{Mutex, OnceLock};
#[cfg(not(windows))]
use std::time::{SystemTime, UNIX_EPOCH};

use lodestar_core::types::RelPath;
use lodestar_discovery::{discover_inventory, DiscoveryPolicy};
use lodestar_store::Store;
#[cfg(windows)]
use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};

const STORE_SOURCE: &str = include_str!("../src/lib.rs");
const DISCOVERY_OWNER_MODULES: [(&str, &str); 2] = [
    (
        "config.rs",
        include_str!("../../lodestar-discovery/src/config.rs"),
    ),
    (
        "lib.rs",
        include_str!("../../lodestar-discovery/src/lib.rs"),
    ),
];
const STORE_OWNER_MODULES: [(&str, &str); 9] = [
    (
        "error.rs",
        include_str!("../../lodestar-store/src/error.rs"),
    ),
    (
        "event.rs",
        include_str!("../../lodestar-store/src/event.rs"),
    ),
    (
        "index.rs",
        include_str!("../../lodestar-store/src/index.rs"),
    ),
    ("lib.rs", STORE_SOURCE),
    (
        "schema.rs",
        include_str!("../../lodestar-store/src/schema.rs"),
    ),
    (
        "synth.rs",
        include_str!("../../lodestar-store/src/synth.rs"),
    ),
    (
        "watch.rs",
        include_str!("../../lodestar-store/src/watch.rs"),
    ),
    (
        "windows_rename_path.rs",
        include_str!("../../lodestar-store/src/windows_rename_path.rs"),
    ),
    (
        "windows_vfs.rs",
        include_str!("../../lodestar-store/src/windows_vfs.rs"),
    ),
];
const CORE_OWNER_MODULES: [(&str, &str); 17] = [
    (
        "conform.rs",
        include_str!("../../lodestar-core/src/conform.rs"),
    ),
    ("diff.rs", include_str!("../../lodestar-core/src/diff.rs")),
    (
        "document_set.rs",
        include_str!("../../lodestar-core/src/document_set.rs"),
    ),
    ("error.rs", include_str!("../../lodestar-core/src/error.rs")),
    ("eval.rs", include_str!("../../lodestar-core/src/eval.rs")),
    (
        "filter.rs",
        include_str!("../../lodestar-core/src/filter.rs"),
    ),
    ("graph.rs", include_str!("../../lodestar-core/src/graph.rs")),
    ("lib.rs", include_str!("../../lodestar-core/src/lib.rs")),
    ("links.rs", include_str!("../../lodestar-core/src/links.rs")),
    (
        "metadata.rs",
        include_str!("../../lodestar-core/src/metadata.rs"),
    ),
    ("model.rs", include_str!("../../lodestar-core/src/model.rs")),
    ("parse.rs", include_str!("../../lodestar-core/src/parse.rs")),
    ("plan.rs", include_str!("../../lodestar-core/src/plan.rs")),
    (
        "render.rs",
        include_str!("../../lodestar-core/src/render.rs"),
    ),
    (
        "store_trait.rs",
        include_str!("../../lodestar-core/src/store_trait.rs"),
    ),
    ("text.rs", include_str!("../../lodestar-core/src/text.rs")),
    ("types.rs", include_str!("../../lodestar-core/src/types.rs")),
];

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn write(root: &Path, path: &str, contents: impl AsRef<[u8]>) {
    let target = root.join(path);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(target, contents).unwrap();
}

fn large_utf8_markdown() -> String {
    let mut body = String::from("---\ntitle: repair9\n---\n\n# Repair 9\n\n");
    for _ in 0..45_000 {
        body.push_str("cuerpo UTF-8 grande: áéíóú · 東京 · 🚀\n");
    }
    body
}

#[cfg(not(windows))]
fn access_time_ns(path: &Path) -> u128 {
    fs::metadata(path)
        .expect("guard: el cuerpo admitido existe")
        .accessed()
        .expect("guard: el filesystem expone atime")
        .duration_since(UNIX_EPOCH)
        .expect("guard: atime no es anterior a epoch")
        .as_nanos()
}

#[cfg(not(windows))]
fn reset_access_time(path: &Path) {
    let file = File::options()
        .read(true)
        .write(true)
        .open(path)
        .expect("abrir cuerpo escribible para preparar el observador atime");
    file.set_times(FileTimes::new().set_accessed(SystemTime::UNIX_EPOCH))
        .expect("el filesystem debe permitir fijar atime para la prueba");
}

/// Conserva un handle que permite accesos de metadata pero niega nuevas aperturas de payload.
/// Así, `discover_inventory` solo puede completar si se mantiene en la primera pasada compacta.
#[cfg(windows)]
fn deny_payload_reads(path: &Path) -> OwnedHandle {
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide.push(0);
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    assert_ne!(
        handle,
        INVALID_HANDLE_VALUE,
        "guard anti-vacuidad: debe poder abrirse el handle que niega lecturas de payload: {}",
        std::io::Error::last_os_error()
    );
    unsafe { OwnedHandle::from_raw_handle(handle as RawHandle) }
}

fn rp(path: &str) -> RelPath {
    RelPath::new(path).expect("RelPath válido")
}

fn extract_function<'a>(source: &'a str, signature: &str) -> &'a str {
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("guard anti-vacuidad: falta {signature}"));
    let open = source[start..]
        .find('{')
        .map(|offset| start + offset)
        .expect("guard anti-vacuidad: la función debe tener cuerpo");
    let mut depth = 0_u32;
    for (offset, byte) in source[open..].bytes().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return &source[start..=open + offset];
                }
            }
            _ => {}
        }
    }
    panic!("guard anti-vacuidad: cuerpo sin cerrar para {signature}");
}

fn normalize_newlines(source: &str) -> String {
    source.replace("\r\n", "\n").replace('\r', "\n")
}

fn normalized_store_source() -> String {
    normalize_newlines(STORE_OWNER_MODULES[3].1)
}

fn owner_modules_hash<'a>(modules: impl IntoIterator<Item = (&'a str, &'a str)>) -> String {
    let mut hasher = blake3::Hasher::new();
    for (name, source) in modules {
        let normalized = normalize_newlines(source);
        hasher.update(&(name.len() as u64).to_le_bytes());
        hasher.update(name.as_bytes());
        hasher.update(&(normalized.len() as u64).to_le_bytes());
        hasher.update(normalized.as_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

fn function_contract(source: &str, signature: &str) -> (String, String) {
    let normalized = normalize_newlines(source);
    let function = extract_function(&normalized, signature).to_owned();
    let hash = blake3::hash(function.as_bytes()).to_hex().to_string();
    (function, hash)
}

fn replace_once(source: &str, needle: &str, replacement: &str, label: &str) -> String {
    assert_eq!(
        source.matches(needle).count(),
        1,
        "guard anti-vacuidad: {label} debe localizar un único anchor"
    );
    source.replacen(needle, replacement, 1)
}

fn code_identifier_occurrences(source: &str, expected: &str) -> usize {
    let bytes = source.as_bytes();
    let mut index = 0;
    let mut occurrences = 0;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"//") {
            index += 2;
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
            continue;
        }
        if bytes[index..].starts_with(b"/*") {
            index += 2;
            let mut depth = 1_u32;
            while index < bytes.len() && depth > 0 {
                if bytes[index..].starts_with(b"/*") {
                    depth += 1;
                    index += 2;
                } else if bytes[index..].starts_with(b"*/") {
                    depth -= 1;
                    index += 2;
                } else {
                    index += 1;
                }
            }
            continue;
        }
        if bytes[index] == b'"' {
            index += 1;
            while index < bytes.len() {
                match bytes[index] {
                    b'\\' => index = (index + 2).min(bytes.len()),
                    b'"' => {
                        index += 1;
                        break;
                    }
                    _ => index += 1,
                }
            }
            continue;
        }
        if bytes[index].is_ascii_alphabetic() || bytes[index] == b'_' {
            let start = index;
            index += 1;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            occurrences += usize::from(&source[start..index] == expected);
            continue;
        }
        index += 1;
    }
    occurrences
}

fn verify_payload_read_counters(function: &str) -> Result<(), String> {
    let statements: Vec<&str> = function
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("//"))
        .collect();

    if function.contains("\"open_count\": 1") || function.contains("\"read_count\": 1") {
        return Err("los contadores no pueden ser literales autodeclarados".into());
    }
    for (counter, event_field) in [
        ("open_count", "\"open_count\": open_count"),
        ("read_count", "\"read_count\": read_count"),
    ] {
        if !function.contains(&format!("let mut {counter} = 0")) {
            return Err(format!("falta inicializar {counter} desde cero"));
        }
        if !function.contains(event_field) {
            return Err(format!("el evento no serializa el {counter} derivado"));
        }
    }

    let open_lines: Vec<usize> = statements
        .iter()
        .enumerate()
        .filter_map(|(index, line)| line.contains("File::open(").then_some(index))
        .collect();
    if open_lines.len() != 1 {
        return Err(format!(
            "read_payload debe contener exactamente una apertura File::open real, encontradas {}",
            open_lines.len()
        ));
    }
    if statements.get(open_lines[0] + 1).copied() != Some("open_count += 1;") {
        return Err("open_count debe incrementarse inmediatamente tras la apertura exitosa".into());
    }

    let read_lines: Vec<usize> = statements
        .iter()
        .enumerate()
        .filter_map(|(index, line)| line.contains(".read_to_end(").then_some(index))
        .collect();
    if read_lines.len() != 1 {
        return Err(format!(
            "read_payload debe contener exactamente una operación read_to_end real, encontradas {}",
            read_lines.len()
        ));
    }
    if statements.get(read_lines[0] + 1).copied() != Some("read_count += 1;") {
        return Err("read_count debe incrementarse inmediatamente tras la lectura exitosa".into());
    }

    for (resource, expected, role) in [
        (
            "file",
            4,
            "los handles de payload y auditoría solo pueden declararse y consumirse una vez cada uno",
        ),
        (
            "root",
            6,
            "root solo puede alimentar el audit path, su canonicalización y strip_prefix",
        ),
        (
            "relative",
            2,
            "relative solo puede declararse y serializarse en el evento",
        ),
        (
            "path",
            4,
            "path solo puede aparecer en firma, File::open, canonicalize y fallback",
        ),
        (
            "canonical_path",
            3,
            "canonical_path solo puede aparecer en su declaración, strip_prefix y fallback",
        ),
    ] {
        let actual = code_identifier_occurrences(function, resource);
        if actual != expected {
            return Err(format!(
                "recurso {resource}: {role}; referencias esperadas {expected}, encontradas {actual}"
            ));
        }
    }

    Ok(())
}

/// C2/C4 — el evento de auditoría no puede declarar contadores constantes: cada valor debe
/// derivarse de la única apertura y la única lectura de payload que realmente ejecuta
/// `read_payload`. Las contrafactuales cubren una segunda lectura por la misma API, por otra API,
/// por copia y por delegación para demostrar que la guarda no depende de una denylist de llamadas.
#[test]
fn c2_c4_payload_audit_counters_derive_from_single_real_io_operations() {
    let compliant_reference = r#"
fn read_payload(root: &Path, path: &Path) {
    let mut open_count = 0_u64;
    let mut read_count = 0_u64;
    let mut file = std::fs::File::open(path)?;
    open_count += 1;
    let mut content = Vec::new();
    file.read_to_end(&mut content)?;
    read_count += 1;
    let Some(audit) = payload_audit_path(root) else {
        return Ok(content);
    };
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let canonical_path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let relative = canonical_path
        .strip_prefix(&root)
        .unwrap_or(&canonical_path)
        .to_string_lossy();
    let Some(mut file) = open_payload_audit(&audit) else {
        return Ok(content);
    };
    writeln!(file, "{}", json!({
        "path": relative,
        "open_count": open_count,
        "read_count": read_count,
    }))?;
}
"#;
    verify_payload_read_counters(compliant_reference)
        .expect("guard anti-vacuidad: la forma que deriva contadores de I/O debe admitirse");

    let duplicated_read = compliant_reference.replacen(
        "    file.read_to_end(&mut content)?;\n",
        "    file.read_to_end(&mut content)?;\n    file.read_to_end(&mut content)?;\n",
        1,
    );
    let duplicate_error = verify_payload_read_counters(&duplicated_read)
        .expect_err("guard anti-vacuidad: una segunda lectura real debe ser rechazada");
    assert!(
        duplicate_error.contains("exactamente una operación read_to_end real"),
        "la mutación de lectura duplicada debe morir por la razón causal: {duplicate_error}"
    );

    for (label, injected, resource) in [
        ("std::fs::read", "    std::fs::read(path)?;\n"),
        ("std::fs::copy", "    std::fs::copy(path, temp_path)?;\n"),
        ("delegación", "    duplicate_payload_read(path)?;\n"),
        (
            "alias",
            "    let payload_alias = path;\n    duplicate_payload_read(payload_alias)?;\n",
        ),
    ]
    .map(|(label, injected)| (label, injected, "path"))
    {
        let mutant = compliant_reference.replacen(
            "    let canonical_path =",
            &format!("{injected}    let canonical_path ="),
            1,
        );
        let error = match verify_payload_read_counters(&mutant) {
            Ok(()) => panic!("guard anti-vacuidad: debe rechazar {label}"),
            Err(error) => error,
        };
        assert!(
            error.contains(&format!("recurso {resource}:")),
            "la mutación {label} debe morir por la referencia adicional a {resource}: {error}"
        );
    }

    for (label, injected, resource) in [
        (
            "lectura por canonical_path",
            "    std::fs::read(&canonical_path)?;\n",
            "canonical_path",
        ),
        (
            "alias de canonical_path",
            "    let canonical_alias = &canonical_path;\n    duplicate_payload_read(canonical_alias)?;\n",
            "canonical_path",
        ),
    ] {
        let mutant = compliant_reference.replacen(
            "    let relative =",
            &format!("{injected}    let relative ="),
            1,
        );
        let error = match verify_payload_read_counters(&mutant) {
            Ok(()) => panic!("guard anti-vacuidad: debe rechazar {label}"),
            Err(error) => error,
        };
        assert!(
            error.contains(&format!("recurso {resource}:")),
            "la mutación {label} debe morir por la referencia adicional a {resource}: {error}"
        );
    }

    for (label, injected) in [
        (
            "rewind y copy sobre el mismo handle",
            "    file.rewind()?;\n    std::io::copy(&mut file, &mut sink)?;\n",
        ),
        (
            "helper por referencia mutable al handle",
            "    duplicate_payload_read(&mut file)?;\n",
        ),
        (
            "alias del handle",
            "    let payload_handle = &mut file;\n    duplicate_payload_read(payload_handle)?;\n",
        ),
    ] {
        let mutant = compliant_reference.replacen(
            "    read_count += 1;\n",
            &format!("    read_count += 1;\n{injected}"),
            1,
        );
        let error = match verify_payload_read_counters(&mutant) {
            Ok(()) => panic!("guard anti-vacuidad: debe rechazar {label}"),
            Err(error) => error,
        };
        assert!(
            error.contains("recurso file:"),
            "la mutación {label} debe morir por reutilizar el handle de payload: {error}"
        );
    }

    for (label, injected) in [
        (
            "read por root.join(relative)",
            "    std::fs::read(root.join(relative))?;\n",
        ),
        (
            "copy por root.join(relative)",
            "    std::fs::copy(root.join(relative), temp_path)?;\n",
        ),
        (
            "helper por root.join(relative)",
            "    duplicate_payload_read(root.join(relative))?;\n",
        ),
        (
            "aliases de root y relative",
            "    let root_alias = &root;\n    let relative_alias = &relative;\n    duplicate_payload_read(root_alias.join(relative_alias))?;\n",
        ),
    ] {
        let mutant = compliant_reference.replacen(
            "    let Some(mut file) = open_payload_audit",
            &format!("{injected}    let Some(mut file) = open_payload_audit"),
            1,
        );
        let error = match verify_payload_read_counters(&mutant) {
            Ok(()) => panic!("guard anti-vacuidad: debe rechazar {label}"),
            Err(error) => error,
        };
        assert!(
            error.contains("recurso root:") || error.contains("recurso relative:"),
            "la mutación {label} debe morir por reconstruir el path del payload: {error}"
        );
    }

    let ignored_non_code = compliant_reference.replacen(
        "    let Some(mut file) = open_payload_audit",
        "    // file root relative path canonical_path no son referencias ejecutables\n    let note = \"file root relative path canonical_path\";\n    let _ = note;\n    let Some(mut file) = open_payload_audit",
        1,
    );
    verify_payload_read_counters(&ignored_non_code)
        .expect("guard anti-vacuidad: comentarios y literales no alteran el contrato de recursos");

    let store_source = normalized_store_source();
    let read_payload = extract_function(&store_source, "fn read_payload(");
    verify_payload_read_counters(read_payload).unwrap_or_else(|reason| {
        panic!("C2/C4: el evento payload_read debe derivar sus contadores de I/O real: {reason}")
    });
}

/// C2/C4 — la adquisición y segunda pasada completas quedan autenticadas, no solo sus contadores.
/// El contrato normaliza CRLF para ser idéntico en los tres runners y cubre desde la firma hasta
/// la llave final de cada función. Cualquier lectura añadida antes del caller, dentro de
/// `read_payload` o al consumir el iterador cambia necesariamente una de las huellas.
#[test]
fn c2_c4_second_pass_full_function_contract_allows_exactly_one_payload_read_call() {
    const CONTRACTS: [(&str, &str, &str); 3] = [
        (
            "fn rebuild_from_inventory_with_duration(",
            "19120b26d775af76fd893724992f4dcc4afea4092a8593b383c2f0369f7a1fff",
            "            diagnostics,\n        )\n    }",
        ),
        (
            "fn rebuild_iter<I>(",
            "e714ac49008770e9adde7e6dfc50bf143cef7863912006e7cf0ef60dd40187eb",
            "            \"diagnostics\": diagnostics,\n        }))\n    }",
        ),
        (
            "fn read_payload(",
            "aa5431567e6421ad72812601fcaa04f27d9b502b54319963b903425a53077be0",
            "    Ok(content)\n}",
        ),
    ];

    let store_source = normalized_store_source();
    let crlf_source = store_source.replace('\n', "\r\n");
    for (signature, expected_hash, terminal) in CONTRACTS {
        let (function, actual_hash) = function_contract(&store_source, signature);
        assert!(
            function.starts_with(signature),
            "guard anti-vacuidad: el balance de llaves debe incluir la firma {signature}"
        );
        assert!(
            function.ends_with(terminal),
            "guard anti-vacuidad: el balance de llaves debe alcanzar el terminal real de {signature}"
        );
        assert_eq!(
            function_contract(&crlf_source, signature).1,
            actual_hash,
            "el contrato de {signature} debe normalizar CRLF antes de hashear"
        );
        assert_eq!(
            actual_hash, expected_hash,
            "C2/C4: cambió la función completa que gobierna adquisición/segunda pasada: {signature}"
        );
    }

    let (acquisition, acquisition_hash) =
        function_contract(&store_source, "fn rebuild_from_inventory_with_duration(");
    let (rebuild_iter, rebuild_iter_hash) = function_contract(&store_source, "fn rebuild_iter<I>(");
    let (read_payload, read_payload_hash) = function_contract(&store_source, "fn read_payload(");

    assert_eq!(
        code_identifier_occurrences(&store_source, "read_payload"),
        2,
        "C2/C4: globalmente debe existir una definición y exactamente un caller productivo de read_payload"
    );
    assert_eq!(
        code_identifier_occurrences(&read_payload, "read_payload"),
        1,
        "guard anti-vacuidad: una de las dos ocurrencias globales es la definición"
    );
    assert_eq!(
        code_identifier_occurrences(&acquisition, "read_payload"),
        1,
        "C2/C4: la única llamada debe vivir en la adquisición lazy de la segunda pasada"
    );
    assert_eq!(
        code_identifier_occurrences(&rebuild_iter, "read_payload"),
        0,
        "C2/C4: rebuild_iter consume payloads ya adquiridos y no debe releerlos"
    );

    let caller_mutant = replace_once(
        &acquisition,
        "            let content = read_payload(&docs_root, &full).map_err(|error| {",
        "            let _unexpected = std::fs::read(&full)?;\n            let content = read_payload(&docs_root, &full).map_err(|error| {",
        "lectura extra antes del caller",
    );
    assert!(
        caller_mutant.contains("std::fs::read(&full)"),
        "guard anti-vacuidad: el primer contrafactual debe leer el mismo payload antes del caller"
    );
    assert_ne!(
        blake3::hash(caller_mutant.as_bytes()).to_hex().to_string(),
        acquisition_hash,
        "el contrato debe matar una lectura de payload anterior al caller aunque el caller siga único"
    );

    let compensated_read_mutant = replace_once(
        &read_payload,
        "    let canonical_path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());",
        "    let _compensated_extra_read = std::fs::read(path)?;\n    let canonical_path = std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::new());",
        "lectura extra con referencias compensadas dentro de read_payload",
    );
    assert_eq!(
        code_identifier_occurrences(&compensated_read_mutant, "path"),
        code_identifier_occurrences(&read_payload, "path"),
        "guard anti-vacuidad: el segundo contrafactual compensa su referencia extra a path"
    );
    for budget in ["File", "read_to_end", "open_count", "read_count"] {
        assert_eq!(
            code_identifier_occurrences(&compensated_read_mutant, budget),
            code_identifier_occurrences(&read_payload, budget),
            "guard anti-vacuidad: el mutante debe conservar el presupuesto compensable de {budget}"
        );
    }
    assert!(
        compensated_read_mutant.contains("_compensated_extra_read"),
        "guard anti-vacuidad: la lectura extra compensada debe existir"
    );
    assert_ne!(
        blake3::hash(compensated_read_mutant.as_bytes())
            .to_hex()
            .to_string(),
        read_payload_hash,
        "el contrato completo debe matar una lectura que conserva todos los conteos compensables"
    );

    let iter_mutant = replace_once(
        &rebuild_iter,
        "        for item in docs {",
        "        for item in docs {\n            let _unexpected = std::fs::read(self.root.join(\"payload.md\"))?;",
        "lectura extra dentro de rebuild_iter",
    );
    assert!(
        iter_mutant.contains("std::fs::read(self.root.join"),
        "guard anti-vacuidad: el tercer contrafactual debe introducir I/O en rebuild_iter"
    );
    assert_ne!(
        blake3::hash(iter_mutant.as_bytes()).to_hex().to_string(),
        rebuild_iter_hash,
        "el contrato debe matar cualquier lectura añadida al consumidor de la segunda pasada"
    );
}

/// C2/C4 — la huella de las tres funciones de orquestación no puede cerrar un grafo de llamadas
/// abierto: todos los módulos de producción de `lodestar-store`, `lodestar-core` y
/// `lodestar-discovery` autentican también helpers de snapshot, config, fingerprint, parseo y
/// proyección.
/// Cada agregado incluye nombre, longitud y bytes normalizados de cada módulo, por lo que ni un
/// helper indirecto ni un módulo propietario pueden leer o parsear de nuevo un payload sin cambiar
/// el contrato.
#[test]
fn c2_c4_second_pass_owner_modules_close_indirect_payload_read_graph() {
    const STORE_OWNER_MODULES_HASH: &str =
        "bfbbb8df68b83390ad2d6c97361956ba8786869cc1d657c1b86ac067f70d8e64";
    const CORE_OWNER_MODULES_HASH: &str =
        "3de1342405eb39db24884e4f9d7f6f1eeb30e8f9c9a0eacc1c9b50e9d67e18e7";
    const DISCOVERY_OWNER_MODULES_HASH: &str =
        "75b2a58e2515f9e54152009f0bb190e40ff68af6b5a23f3599f993b3a91a5e3d";

    let discovery_modules: Vec<(&str, String)> = DISCOVERY_OWNER_MODULES
        .iter()
        .map(|(name, source)| (*name, normalize_newlines(source)))
        .collect();
    let discovery_hash = owner_modules_hash(
        discovery_modules
            .iter()
            .map(|(name, source)| (*name, source.as_str())),
    );
    let store_modules: Vec<(&str, String)> = STORE_OWNER_MODULES
        .iter()
        .map(|(name, source)| (*name, normalize_newlines(source)))
        .collect();
    let store_hash = owner_modules_hash(
        store_modules
            .iter()
            .map(|(name, source)| (*name, source.as_str())),
    );
    let core_modules: Vec<(&str, String)> = CORE_OWNER_MODULES
        .iter()
        .map(|(name, source)| (*name, normalize_newlines(source)))
        .collect();
    let core_hash = owner_modules_hash(
        core_modules
            .iter()
            .map(|(name, source)| (*name, source.as_str())),
    );
    assert_eq!(
        STORE_OWNER_MODULES.map(|(name, _)| name),
        [
            "error.rs",
            "event.rs",
            "index.rs",
            "lib.rs",
            "schema.rs",
            "synth.rs",
            "watch.rs",
            "windows_rename_path.rs",
            "windows_vfs.rs",
        ],
        "guard anti-vacuidad: el contrato debe enumerar exactamente todos los módulos store actuales"
    );
    assert_eq!(
        CORE_OWNER_MODULES.map(|(name, _)| name),
        [
            "conform.rs",
            "diff.rs",
            "document_set.rs",
            "error.rs",
            "eval.rs",
            "filter.rs",
            "graph.rs",
            "lib.rs",
            "links.rs",
            "metadata.rs",
            "model.rs",
            "parse.rs",
            "plan.rs",
            "render.rs",
            "store_trait.rs",
            "text.rs",
            "types.rs",
        ],
        "guard anti-vacuidad: el contrato debe enumerar exactamente todos los módulos core actuales"
    );
    assert_eq!(
        DISCOVERY_OWNER_MODULES.map(|(name, _)| name),
        ["config.rs", "lib.rs"],
        "guard anti-vacuidad: el contrato debe enumerar exactamente todos los módulos discovery actuales"
    );
    let crlf_modules: Vec<(&str, String)> = store_modules
        .iter()
        .map(|(name, source)| (*name, source.replace('\n', "\r\n")))
        .collect();
    let core_crlf_modules: Vec<(&str, String)> = core_modules
        .iter()
        .map(|(name, source)| (*name, source.replace('\n', "\r\n")))
        .collect();
    let discovery_crlf_modules: Vec<(&str, String)> = discovery_modules
        .iter()
        .map(|(name, source)| (*name, source.replace('\n', "\r\n")))
        .collect();
    assert_eq!(
        owner_modules_hash(
            crlf_modules
                .iter()
                .map(|(name, source)| (*name, source.as_str())),
        ),
        store_hash,
        "la huella agregada de todos los módulos store debe ser estable ante CRLF"
    );
    assert_eq!(
        owner_modules_hash(
            core_crlf_modules
                .iter()
                .map(|(name, source)| (*name, source.as_str())),
        ),
        core_hash,
        "la huella agregada de todos los módulos core debe ser estable ante CRLF"
    );
    assert_eq!(
        owner_modules_hash(
            discovery_crlf_modules
                .iter()
                .map(|(name, source)| (*name, source.as_str())),
        ),
        discovery_hash,
        "la huella agregada de todos los módulos discovery debe ser estable ante CRLF"
    );
    assert_eq!(
        store_hash, STORE_OWNER_MODULES_HASH,
        "C2/C4: cambió algún módulo propietario de la segunda pasada"
    );
    assert_eq!(
        core_hash, CORE_OWNER_MODULES_HASH,
        "C2/C4: cambió algún módulo core propietario del parseo/proyección"
    );
    assert_eq!(
        discovery_hash, DISCOVERY_OWNER_MODULES_HASH,
        "C2/C4: cambió algún módulo propietario de discovery/config/fingerprint"
    );

    let mut snapshot_read_modules = store_modules.clone();
    snapshot_read_modules[3].1 = replace_once(
        &snapshot_read_modules[3].1,
        "fn verify_rebuild_snapshot(snapshot: &RebuildSnapshot) -> Result<(), StoreError> {\n",
        "fn verify_rebuild_snapshot(snapshot: &RebuildSnapshot) -> Result<(), StoreError> {\n    let _unexpected_payload = std::fs::read(&snapshot.root)?;\n",
        "lectura delegada desde verify_rebuild_snapshot",
    );
    assert!(
        snapshot_read_modules[3]
            .1
            .contains("std::fs::read(&snapshot.root)"),
        "guard anti-vacuidad: el mutante debe leer un payload desde verify_rebuild_snapshot"
    );
    assert_ne!(
        owner_modules_hash(
            snapshot_read_modules
                .iter()
                .map(|(name, source)| (*name, source.as_str())),
        ),
        store_hash,
        "la huella de store debe matar lecturas delegadas desde verify_rebuild_snapshot"
    );

    let mut store_fingerprint_modules = store_modules.clone();
    store_fingerprint_modules[3].1 = replace_once(
        &store_fingerprint_modules[3].1,
        "fn fs_fingerprint(path: &Path) -> Result<FileFingerprint, std::io::Error> {\n",
        "fn fs_fingerprint(path: &Path) -> Result<FileFingerprint, std::io::Error> {\n    let _unexpected_payload = std::fs::read(path)?;\n",
        "lectura delegada desde fs_fingerprint",
    );
    assert!(
        store_fingerprint_modules[3]
            .1
            .contains("std::fs::read(path)"),
        "guard anti-vacuidad: el mutante debe leer un payload desde fs_fingerprint"
    );
    assert_ne!(
        owner_modules_hash(
            store_fingerprint_modules
                .iter()
                .map(|(name, source)| (*name, source.as_str())),
        ),
        store_hash,
        "la huella de store debe matar lecturas delegadas desde fs_fingerprint"
    );

    let mut index_parse_modules = store_modules.clone();
    index_parse_modules[2].1 = replace_once(
        &index_parse_modules[2].1,
        "        } = document;\n        // Updating the external-content row",
        "        } = document;\n        let _unexpected_parse = model::parse_file(path.as_str(), raw);\n        // Updating the external-content row",
        "parse duplicado dentro de StreamingProjection::insert",
    );
    assert!(
        index_parse_modules[2]
            .1
            .contains("let _unexpected_parse = model::parse_file(path.as_str(), raw)"),
        "guard anti-vacuidad: el mutante de index.rs debe parsear otra vez dentro de StreamingProjection::insert"
    );
    assert_ne!(
        owner_modules_hash(
            index_parse_modules
                .iter()
                .map(|(name, source)| (*name, source.as_str())),
        ),
        store_hash,
        "el agregado debe matar un parse duplicado dentro de index.rs"
    );

    let mut other_module_mutant = store_modules.clone();
    other_module_mutant[1]
        .1
        .push_str("\n// ci88 contrafactual en otro módulo\n");
    assert_ne!(
        owner_modules_hash(
            other_module_mutant
                .iter()
                .map(|(name, source)| (*name, source.as_str())),
        ),
        store_hash,
        "guard anti-vacuidad: cambiar un segundo módulo también debe cambiar el agregado"
    );

    let mut duplicate_core_parse = core_modules.clone();
    duplicate_core_parse[10].1 = replace_once(
        &duplicate_core_parse[10].1,
        "pub fn parse_file(_path: &str, raw: &str) -> Parsed {\n    let sf = split_front(raw);",
        "pub fn parse_file(_path: &str, raw: &str) -> Parsed {\n    let _unexpected_duplicate_split = split_front(raw);\n    let sf = split_front(raw);",
        "parse_file invoca dos veces el helper split_front",
    );
    assert_eq!(
        duplicate_core_parse[10].1.matches("split_front(raw)").count(),
        CORE_OWNER_MODULES[10].1.matches("split_front(raw)").count() + 1,
        "guard anti-vacuidad: el mutante de model.rs debe ejecutar una segunda pasada de parseo conservando el resultado original"
    );
    assert_ne!(
        owner_modules_hash(
            duplicate_core_parse
                .iter()
                .map(|(name, source)| (*name, source.as_str())),
        ),
        core_hash,
        "el agregado core debe matar un parse duplicado dentro de model::parse_file"
    );

    let mut second_core_module_mutant = core_modules.clone();
    second_core_module_mutant[0].1 = replace_once(
        &second_core_module_mutant[0].1,
        "pub(crate) fn validate_file(path: &RelPath, parsed: &Parsed, raw: &str) -> Vec<Check> {\n",
        "pub(crate) fn validate_file(path: &RelPath, parsed: &Parsed, raw: &str) -> Vec<Check> {\n    let _unexpected_duplicate_parse = model::parse_file(path.as_str(), raw);\n",
        "parse duplicado desde un segundo módulo core",
    );
    assert!(
        second_core_module_mutant[0]
            .1
            .contains("let _unexpected_duplicate_parse = model::parse_file(path.as_str(), raw)"),
        "guard anti-vacuidad: el mutante multimódulo debe parsear de nuevo desde conform.rs"
    );
    assert_ne!(
        owner_modules_hash(
            second_core_module_mutant
                .iter()
                .map(|(name, source)| (*name, source.as_str())),
        ),
        core_hash,
        "guard anti-vacuidad: cambiar otro módulo core también debe cambiar el agregado"
    );

    let mut discovery_fingerprint_read_modules = discovery_modules.clone();
    discovery_fingerprint_read_modules[1].1 = replace_once(
        &discovery_fingerprint_read_modules[1].1,
        "pub fn filesystem_fingerprint(\n    path: &Path,\n    follow_target: bool,\n) -> Result<DiscoveryFingerprint, DiscoveryError> {\n",
        "pub fn filesystem_fingerprint(\n    path: &Path,\n    follow_target: bool,\n) -> Result<DiscoveryFingerprint, DiscoveryError> {\n    let _unexpected_payload = std::fs::read(path)?;\n",
        "lectura delegada desde filesystem_fingerprint",
    );
    assert!(
        discovery_fingerprint_read_modules[1]
            .1
            .contains("std::fs::read(path)"),
        "guard anti-vacuidad: el mutante debe leer un payload desde filesystem_fingerprint"
    );
    assert_ne!(
        owner_modules_hash(
            discovery_fingerprint_read_modules
                .iter()
                .map(|(name, source)| (*name, source.as_str())),
        ),
        discovery_hash,
        "la huella de discovery debe matar lecturas delegadas desde filesystem_fingerprint"
    );

    let mut config_payload_read_modules = discovery_modules.clone();
    config_payload_read_modules[0].1 = replace_once(
        &config_payload_read_modules[0].1,
        "    pub fn load(root: &Path) -> Result<WorkspaceConfig, String> {\n",
        "    pub fn load(root: &Path) -> Result<WorkspaceConfig, String> {\n        let _unexpected_markdown = std::fs::read(root.join(\"docs/large-utf8.md\"));\n",
        "lectura de Markdown delegada desde WorkspaceConfig::load",
    );
    assert!(
        config_payload_read_modules[0]
            .1
            .contains("std::fs::read(root.join(\"docs/large-utf8.md\"))"),
        "guard anti-vacuidad: el mutante de config.rs debe leer el Markdown de la prueba"
    );
    assert_ne!(
        owner_modules_hash(
            config_payload_read_modules
                .iter()
                .map(|(name, source)| (*name, source.as_str())),
        ),
        discovery_hash,
        "el agregado discovery debe matar una lectura de Markdown delegada desde config.rs"
    );
}

/// C2/C4 — **Dado** un documento UTF-8 grande admitido por discovery, **cuando** se ejecuta la
/// ruta canónica `discover_inventory` + `rebuild_from_discovered_inventory`, **entonces** el
/// cuerpo debe abrirse una sola vez, en la segunda pasada, y el inventario no debe producir ningún
/// acceso observable al payload.
#[test]
fn c2_c4_canonical_inventory_and_rebuild_read_each_admitted_body_once() {
    let _env = env_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let root = tempfile::tempdir().unwrap();
    let document = root.path().join("docs/large-utf8.md");
    let payload = large_utf8_markdown();
    assert!(
        payload.len() > 1_000_000,
        "guard anti-vacuidad: el cuerpo debe ser grande y UTF-8, no un fixture vacío"
    );
    write(root.path(), "docs/large-utf8.md", &payload);

    // El sidecar vive en el plano de control y existe antes de abrir Store o tomar el snapshot,
    // para que la ventana observada cubra también la carga de config/policy de `Store::open`.
    let audit = root.path().join(".lodestar/h03-repair9-read-audit.ndjson");
    write(root.path(), ".lodestar/h03-repair9-read-audit.ndjson", b"");

    #[cfg(not(windows))]
    let before_discovery = {
        reset_access_time(&document);
        access_time_ns(&document)
    };
    #[cfg(windows)]
    let payload_read_guard = {
        let guard = deny_payload_reads(&document);
        let denied = fs::read(&document)
            .expect_err("guard anti-vacuidad: una apertura de payload debe quedar bloqueada");
        assert!(
            matches!(denied.raw_os_error(), Some(5 | 32)),
            "guard anti-vacuidad: se esperaba access denied/sharing violation, no {denied:?}"
        );
        guard
    };
    let store = Store::open(root.path()).expect("abrir cache derivada vacía sin leer payloads");
    let discovered = discover_inventory(root.path(), &DiscoveryPolicy::default())
        .expect("discovery canónico debe completar");
    #[cfg(windows)]
    drop(payload_read_guard);
    assert_eq!(
        discovered.documents,
        vec![rp("docs/large-utf8.md")],
        "guard anti-vacuidad: el cuerpo grande debe ser un documento admitido"
    );
    #[cfg(not(windows))]
    let (after_discovery, discovery_body_read) = {
        let after_discovery = access_time_ns(&document);
        let discovery_body_read = after_discovery > before_discovery;
        assert!(
            !discovery_body_read,
            "C2/C4: Store::open + discovery no deben abrir/leer el cuerpo admitido (atime: {before_discovery}->{after_discovery})"
        );
        (after_discovery, discovery_body_read)
    };

    // Conserva una única línea temporal de atime: el siguiente cambio solo puede provenir de la
    // lectura de proyección posterior al inventario descubierto. Windows/NTFS puede diferir o
    // desactivar LastAccessTime, por lo que allí mandan los eventos de lectura reales de abajo.
    #[cfg(not(windows))]
    let before_rebuild = after_discovery;
    std::env::set_var("LODESTAR_H03_TEST_READ_AUDIT", &audit);
    let report = store
        .rebuild_from_discovered_inventory(&discovered)
        .expect("rebuild canónico con el snapshot descubierto");
    std::env::remove_var("LODESTAR_H03_TEST_READ_AUDIT");
    let events: Vec<serde_json::Value> = fs::read_to_string(&audit)
        .expect("seam H03 de auditoría de lectura real")
        .lines()
        .map(|line| serde_json::from_str(line).expect("evento NDJSON válido"))
        .collect();
    assert_eq!(
        events.len(),
        1,
        "la proyección debe registrar una lectura del cuerpo"
    );
    assert_eq!(events[0]["event"].as_str(), Some("payload_read"));
    assert_eq!(events[0]["path"].as_str(), Some("docs/large-utf8.md"));
    assert_eq!(events[0]["open_count"].as_u64(), Some(1));
    assert_eq!(events[0]["read_count"].as_u64(), Some(1));
    assert_eq!(events[0]["bytes"].as_u64(), Some(payload.len() as u64));
    assert_eq!(
        report["documents_read"].as_u64(),
        Some(1),
        "guard anti-vacuidad: el informe debe admitir exactamente un cuerpo"
    );

    #[cfg(not(windows))]
    {
        let after_rebuild = access_time_ns(&document);
        let rebuild_body_read = after_rebuild > before_rebuild;
        assert!(
            rebuild_body_read,
            "guard del observador: la segunda pasada no produjo una señal de lectura del cuerpo"
        );
        let observed_body_reads = u64::from(discovery_body_read) + u64::from(rebuild_body_read);
        assert_eq!(
            observed_body_reads,
            1,
            "C2/C4: el cuerpo admitido fue abierto/leído {observed_body_reads} veces (discovery atime: {before_discovery}->{after_discovery}; rebuild atime: {before_rebuild}->{after_rebuild}; documents_read={}; audit_events={events:?})",
            report["documents_read"]
        );
    }
}
