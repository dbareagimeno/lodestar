//! Fase roja de las dos primitivas puras que necesita la semántica secuencial.
//!
//! Se mantienen separadas de la suite de `lodestar-app`: el rojo de servicio debe seguir siendo
//! ejecutable mientras estas referencias hacen visible que el diseño aún no expone las primitivas
//! de rebase/aplicación unitaria que la implementación necesitará.

use lodestar_core::plan;
use lodestar_core::DocumentSet;

#[test]
fn api_rebase_files_y_apply_normalized_operation_deben_existir() {
    // ROJO esperado: ambas APIs todavía no están expuestas por producción.
    let _rebase = DocumentSet::rebase_files;
    let _apply_one = plan::apply_normalized_operation;
}
