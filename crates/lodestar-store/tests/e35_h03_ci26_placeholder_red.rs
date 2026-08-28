//! E35-H03 CI26 — recuperación disk-backed en todas las salidas de la reautenticación Windows.
//!
//! Los runners Unix no ejecutan las ramas Win32 de publicación. Estas guardas portables fijan el
//! protocolo causal que C6 y ARCHITECTURE.md §20.12.2 exigen: una vez instalado el placeholder
//! temporal, ningún error puede abandonar `RootState` sin una conexión real sobre disco.

const STORE_SOURCE: &str = include_str!("../src/lib.rs");

fn braced_block_from<'a>(source: &'a str, marker_at: usize, marker: &str) -> &'a str {
    assert!(
        source[marker_at..].starts_with(marker),
        "guarda anti-vacuidad: se esperaba `{marker}` en byte {marker_at}"
    );
    let open_at = marker_at
        + source[marker_at..]
            .find('{')
            .unwrap_or_else(|| panic!("guarda anti-vacuidad: `{marker}` no abre bloque"));
    let mut depth = 0usize;
    for (offset, byte) in source.as_bytes()[open_at..].iter().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth = depth.checked_sub(1).unwrap_or_else(|| {
                    panic!("guarda anti-vacuidad: bloque `{marker}` desbalanceado")
                });
                if depth == 0 {
                    return &source[marker_at..=open_at + offset];
                }
            }
            _ => {}
        }
    }
    panic!("guarda anti-vacuidad: bloque `{marker}` sin cierre")
}

fn swap_protocol() -> &'static str {
    let start = STORE_SOURCE
        .find("    fn swap_active(")
        .expect("guarda anti-vacuidad: falta swap_active");
    let end = STORE_SOURCE[start..]
        .find("\n    /// Reabre la conexión compartida")
        .expect("guarda anti-vacuidad: falta el límite posterior de swap_active");
    &STORE_SOURCE[start..start + end]
}

fn authentication_match(protocol: &str) -> &str {
    let marker =
        "let published = match windows_vfs::connection_matches_path(&published, &active) {";
    let at = protocol
        .find(marker)
        .unwrap_or_else(|| panic!("guarda anti-vacuidad: falta `{marker}`"));
    assert_eq!(
        protocol.match_indices(marker).count(),
        1,
        "guarda anti-vacuidad: la autenticación exterior debe ser única"
    );
    braced_block_from(protocol, at, marker)
}

fn recovery_before_return(branch: &str) -> Result<(), String> {
    let return_at = branch
        .find("return Err(")
        .ok_or("guarda anti-vacuidad: el brazo de error no retorna Err")?;
    if branch[return_at + 1..].contains("return Err(") {
        return Err("guarda anti-vacuidad: se esperaba una única salida Err en el brazo".into());
    }
    let prefix = &branch[..return_at];
    if !prefix.contains("*guard = standby;")
        && !prefix.contains("restore_after_publication_failure(&mut guard, standby")
    {
        return Err(
            "C6: debe reinstalar una conexión disk-backed en guard antes de retornar el error"
                .into(),
        );
    }
    Ok(())
}

fn retry_contract(branch: &str) -> Result<(), String> {
    for required in [
        "open_sqlite(&active)",
        "connection_matches_path(&replacement, &active)",
    ] {
        if !branch.contains(required) {
            return Err(format!(
                "guarda anti-vacuidad: el reintento debe contener `{required}`"
            ));
        }
    }

    if branch.contains('?') {
        return Err(
            "C6: el reintento no puede propagar con `?` mientras guard conserva el placeholder"
                .into(),
        );
    }

    let drop_positions: Vec<_> = branch.match_indices("drop(standby);").collect();
    if drop_positions.len() != 1 {
        return Err(format!(
            "C6: standby debe liberarse exactamente una vez y solo tras autenticar el reemplazo; posiciones={drop_positions:?}"
        ));
    }
    let authenticated_at = branch
        .rfind("connection_matches_path(&replacement, &active)")
        .expect("required marker checked above");
    if drop_positions[0].0 < authenticated_at {
        return Err(
            "C6: standby no puede liberarse antes de que el reemplazo quede autenticado".into(),
        );
    }

    let mut exits = 0usize;
    for (return_at, _) in branch.match_indices("return Err(") {
        exits += 1;
        let arm_at = branch[..return_at]
            .rfind("=> {")
            .or_else(|| branch[..return_at].rfind("if "))
            .ok_or("guarda anti-vacuidad: salida Err fuera de un brazo explícito")?;
        let corridor = &branch[arm_at..return_at];
        if !corridor.contains("*guard = standby;")
            && !corridor.contains("restore_after_publication_failure(&mut guard, standby")
        {
            return Err(format!(
                "C6: la salida Err en byte {return_at} no restaura una conexión disk-backed"
            ));
        }
    }
    if exits < 2 {
        return Err(format!(
            "guarda anti-vacuidad: se esperaban al menos dos errores explícitos del reintento; observados={exits}"
        ));
    }
    Ok(())
}

fn assert_rejected(result: Result<(), String>, expected: &str) {
    let error = result.expect_err("la mutación contrafactual debía romper el contrato");
    assert!(
        error.contains(expected),
        "la mutación falló por otra razón: esperada `{expected}`, observada `{error}`"
    );
}

/// C6 / §20.12.2 — si autenticar la conexión recién abierta contra el pathname devuelve error,
/// `RootState` debe recuperar `standby` antes de propagarlo. Destruir el único handle disk-backed
/// deja instalado `Connection::open_in_memory()` y convierte cualquier consulta posterior en un
/// error de esquema, contradiciendo la garantía dinámica de `e35_h03_repair14_red`.
#[test]
fn c6_windows_error_de_autenticacion_restaura_standby_antes_de_retornar() {
    let compliant = r#"Err(error) => {
        *guard = standby;
        return Err(StoreError::Io(error.to_string()));
    }"#;
    recovery_before_return(compliant)
        .expect("guarda anti-vacuidad: el protocolo conforme debe ser aceptado");
    let without_recovery = compliant.replace("        *guard = standby;\n", "");
    assert_rejected(
        recovery_before_return(&without_recovery),
        "antes de retornar el error",
    );

    let auth = authentication_match(swap_protocol());
    let marker = "\n            Err(error) => {";
    let at = auth
        .rfind(marker)
        .unwrap_or_else(|| panic!("guarda anti-vacuidad: falta el brazo exterior `{marker}`"));
    let branch = braced_block_from(auth, at + 1, "            Err(error) => {");
    assert!(
        branch.contains("authenticate published SQLite connection"),
        "guarda anti-vacuidad: se inspeccionó un Err distinto del fallo de autenticación exterior"
    );
    recovery_before_return(branch).unwrap_or_else(|error| panic!("{error}"));
}

/// C6 / §20.12.2 — cuando la primera conexión no corresponde al pathname, `standby` sigue siendo
/// la única recuperación válida hasta que el segundo open y su reautenticación terminan. El open,
/// el error del probe y el resultado `false` deben restaurar disco explícitamente; `?` o un drop
/// anticipado pueden retornar dejando el placeholder en memoria.
#[test]
fn c6_windows_reintento_conserva_standby_hasta_autenticar_y_cubre_cada_error() {
    let compliant = r#"Ok(false) => {
        let replacement = match open_sqlite(&active) {
            Ok(replacement) => replacement,
            Err(error) => {
                *guard = standby;
                return Err(error);
            }
        };
        match windows_vfs::connection_matches_path(&replacement, &active) {
            Ok(true) => {
                drop(standby);
                replacement
            }
            Ok(false) => {
                *guard = standby;
                return Err(StoreError::Io("mismatch".into()));
            }
            Err(error) => {
                *guard = standby;
                return Err(StoreError::Io(error.to_string()));
            }
        }
    }"#;
    retry_contract(compliant)
        .expect("guarda anti-vacuidad: el protocolo conforme debe ser aceptado");

    let early_drop = compliant.replacen(
        "        let replacement = match open_sqlite(&active)",
        "        drop(standby);\n        let replacement = match open_sqlite(&active)",
        1,
    );
    assert_rejected(retry_contract(&early_drop), "exactamente una vez");

    let question_mark = compliant.replacen(
        "let replacement = match open_sqlite(&active)",
        "let replacement = open_sqlite(&active)?;\n        let _removed = match open_sqlite(&active)",
        1,
    );
    assert_rejected(retry_contract(&question_mark), "propagar con `?`");

    let unrestored = compliant.replacen("                *guard = standby;\n", "", 1);
    assert_rejected(
        retry_contract(&unrestored),
        "no restaura una conexión disk-backed",
    );

    let auth = authentication_match(swap_protocol());
    let marker = "Ok(false) => {";
    let at = auth
        .find(marker)
        .unwrap_or_else(|| panic!("guarda anti-vacuidad: falta el brazo `{marker}`"));
    let branch = braced_block_from(auth, at, marker);
    retry_contract(branch).unwrap_or_else(|error| panic!("{error}"));
}
