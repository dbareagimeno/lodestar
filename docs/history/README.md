# docs/history/ — specs y propuestas superseded

Documentos que **ya no gobiernan** el diseño ni el comportamiento. Se
conservan sin editar como registro de ingeniería (`ARCHITECTURE.md §21.3`);
la autoridad viva es `ARCHITECTURE.md` (§19/§20/§21) + `docs/REFACTOR_PHASE_2.md`.

| Documento | Qué fue | Qué lo supersedió |
|---|---|---|
| [`REFACTOR.md`](REFACTOR.md) | La spec del giro a motor headless (E9–E14) | Ratificado como `ARCHITECTURE.md §19`; el detalle vigente vive ahí y en `contracts/mcp.yml` |
| [`REFACTOR_DISENO_PROPUESTA.md`](REFACTOR_DISENO_PROPUESTA.md) | La propuesta de diseño (fase A de `/planificar`) de ese giro | Su contenido ratificado se fundió en `§19`; el resto no se adoptó |
| [`PROPUESTA_CLI.md`](PROPUESTA_CLI.md) | Propuesta no ratificada: la CLI como gestor de KB | Nada la implementa; la CLI vigente es `check` + `reindex`. Si se retoma, pasa por `/planificar` |
| [`PROPUESTA_FIXES.md`](PROPUESTA_FIXES.md) | Análisis de reactivar los arreglos sugeridos (`Fix`/`apply_fix`) | La op se retiró (E23-H05); el wire (`fixes: []`, `includeSuggestedFixes`) se conserva y este análisis documenta cómo volvería |
