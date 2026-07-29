//! Puntos de caída inyectables en el orquestador transaccional (E24-H13).
//!
//! Existen **solo** bajo `--features test-failpoints`: en una compilación normal el macro
//! [`failpoint!`] no genera ni una instrucción, y este módulo no se compila.
//!
//! # Por qué hacían falta
//!
//! La feature `test-failpoints` existía desde E13-H06, pero **ningún fichero de `src/` la
//! referenciaba**: no había seam. Los cuatro tests de crash-recovery componían el estado
//! post-crash **a mano**, con las primitivas públicas, y en un orden que **no era el del
//! orquestador**:
//!
//! | | orden de pasos |
//! |---|---|
//! | Producción (`apply_transaction`) | staging → **backup** → **journal** → renames |
//! | Simulación (`simular_caida`) | staging → **journal** → **backup** → renames |
//!
//! Dos consecuencias medidas: `TrasJournalPrepared` describía un estado que el código real **no
//! puede producir** (y pasaba vacuamente, porque sin directorio de recuperación
//! `restore_from_recovery` devuelve `Ok(())` de inmediato), y el estado que el código real **sí**
//! produce —copias escritas, journal aún ausente— no estaba en la taxonomía.
//!
//! Con el seam real, el punto de caída se ejerce **dentro** de `apply_transaction`, así que la
//! taxonomía no puede volver a divergir del orden de producción: si alguien reordena los pasos, es
//! el propio orquestador el que cambia de comportamiento.
//!
//! # Cómo se usa
//!
//! ```ignore
//! ws.armar_failpoint(FailPoint::EntreRenames);
//! let err = ws.apply_transaction(&cs).unwrap_err();   // aborta ahí, dejando el estado en disco
//! ```
//!
//! El aborto es un `Err`, no un `panic!` ni un `process::abort()`: deja el disco **exactamente**
//! como lo dejaría un crash en ese punto (el `Drop` del lock se ejecuta, que es lo mismo que hace
//! el SO al morir el proceso), y permite al test seguir inspeccionando desde el mismo hilo. La
//! garantía de que un crash **de verdad** —`SIGKILL`, sin `Drop` ninguno— también converge la da
//! el test de señal de E24-H14, que mata el binario.

use std::cell::Cell;

/// Punto de la transacción en el que se puede inyectar una caída, **en el orden real de
/// `apply_transaction`**.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailPoint {
    /// Nada más tomar el lock: no hay staging, ni copias, ni journal.
    AlEntrar,
    /// Copias de recuperación escritas, journal **aún ausente**.
    ///
    /// Es el estado que la simulación de E13-H06 no modelaba, porque componía journal antes que
    /// backup. Al reabrir, `recovery_pending()` es `false` (solo mira journals), así que nadie
    /// recupera: hay que comprobar que el canónico está intacto —ningún rename ha ocurrido aún— y
    /// que el árbol de recuperación huérfano lo recoge el GC (E24-H06).
    TrasBackupSinJournal,
    /// Journal `prepared` y copias listas, aún 0 renames.
    TrasJournalPrepared,
    /// A mitad de los renames del canónico (lo inyecta `publish_result` tras el primero).
    EntreRenames,
    /// Todos los renames hechos y journal `applied`, pero **sin sellar**: el journal sigue en
    /// disco, así que al reabrir la recuperación debe **completar**, no restaurar.
    TrasPublicarSinSellar,
    /// Tras marcar el staging como consumido, justo antes de borrar staging y journal.
    AntesDeSellar,
}

thread_local! {
    /// Punto armado para el hilo actual, o `None`. Es `thread_local` a propósito: los tests del
    /// repo corren en paralelo dentro del mismo proceso, y un estado global haría que armar un
    /// failpoint en un test tumbara la transacción de otro.
    static ARMADO: Cell<Option<FailPoint>> = const { Cell::new(None) };
}

/// Arma un punto de caída para el hilo actual. Se **desarma solo** al dispararse, para que una
/// transacción posterior del mismo test no vuelva a caer.
pub fn armar(fp: FailPoint) {
    ARMADO.with(|a| a.set(Some(fp)));
}

/// Desarma cualquier punto de caída del hilo actual.
pub fn desarmar() {
    ARMADO.with(|a| a.set(None));
}

/// `true` si `fp` es el punto armado (y lo desarma).
pub fn disparado(fp: FailPoint) -> bool {
    ARMADO.with(|a| {
        if a.get() == Some(fp) {
            a.set(None);
            true
        } else {
            false
        }
    })
}
