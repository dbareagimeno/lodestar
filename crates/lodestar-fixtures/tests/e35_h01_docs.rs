//! E35-H01 — C9, centinela documental de trazabilidad y deferencias ratificadas.
//!
//! C9 — auditoría de la spec, autoridades, trazabilidad, estado y deferencias.
//!
//! La spec conserva su frontmatter ratificado; el estado operativo se verifica en
//! `IMPLEMENTATION_STATUS.md`. La auditoría también rechaza frases transitorias que dejen la
//! implementación de E35-H01/MemoryBudget como pendiente, pero no confunde con ello las
//! deferencias legítimas (#55/#57/#59/#62) ni el estado abierto de §14.

fn repo_file(relative: &str) -> String {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|crates| crates.parent())
        .expect("raíz del repositorio");
    std::fs::read_to_string(root.join(relative)).expect("autoridad documental presente")
}

#[test]
fn c9_documentos_mantienen_formula_trazabilidad_y_estado_implementado() {
    let autoridades = [
        "ARCHITECTURE.md",
        "docs/REFACTOR_PHASE_2.md",
        "docs/SCALABILITY_ANALYSIS.md",
        "decisiones/README.md",
        "decisiones/14-store-sin-consumidor.md",
        "IMPLEMENTATION_STATUS.md",
        "requirements/epica-35-presupuesto-memoria.md",
    ];
    let frases_pendientes_obsoletas = [
        "todavía no está implementada",
        "pendiente de implementación",
        "la implementación de ese alcance todavía está pendiente",
        "no evidencia de que ese trabajo ya esté implementado",
        "implementa, cuando se ejecute",
    ];
    for autoridad in autoridades {
        let texto = repo_file(autoridad);
        assert!(
            texto.contains("E35-H01"),
            "{autoridad} debe enlazar E35-H01"
        );
        assert!(
            texto.contains("E34-H01 → E35-H01") || texto.contains("E34-H01 -> E35-H01"),
            "{autoridad} debe conservar la trazabilidad #53"
        );
        assert!(
            texto.contains("floor(30")
                && texto.contains("floor(20")
                && texto.contains("N - SQLite - W-TinyLFU"),
            "{autoridad} debe expresar fórmula y residuo a Work"
        );
        assert!(
            texto.contains("#55")
                && texto.contains("#57")
                && texto.contains("#59")
                && texto.contains("#62"),
            "{autoridad} debe deferir semántica fuera-cache/error/no-thrashing"
        );
        let normalizado = texto.to_lowercase();
        for frase in frases_pendientes_obsoletas {
            assert!(
                !normalizado.contains(frase),
                "{autoridad} conserva una frase transitoria obsoleta ({frase:?}) que declara E35-H01/MemoryBudget pendiente"
            );
        }
    }
    let spec = repo_file("requirements/epica-35-presupuesto-memoria.md");
    assert!(
        spec.lines()
            .take(8)
            .any(|line| line.trim() == "estado: \"ratificada\""),
        "la spec debe conservar el frontmatter estado: ratificada"
    );
    let decision = repo_file("decisiones/14-store-sin-consumidor.md");
    assert!(
        decision.contains("estado: \"abierta\"") || decision.contains("§14` permanece abierta")
    );
    assert!(
        !repo_file("contracts/mcp.yml").contains("E35-H01"),
        "C9 no añade delta MCP"
    );
    let status = repo_file("IMPLEMENTATION_STATUS.md");
    assert!(
        status.contains("### E35-H01 — presupuesto de memoria retenida (issue #53; implementada)"),
        "IMPLEMENTATION_STATUS debe marcar E35-H01 como implementada"
    );
    assert!(
        !status.contains(
            "### E35-H01 — presupuesto de memoria retenida (issue #53; diseño ratificado; implementación pendiente)"
        ),
        "IMPLEMENTATION_STATUS no puede conservar E35-H01 como pendiente"
    );
}
