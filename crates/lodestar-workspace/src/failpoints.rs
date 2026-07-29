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

use std::cell::{Cell, RefCell};

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
    /// **En medio del sellado del aborto de ventana** (E25-H02): el `WRITE_CONFLICT` de la ventana
    /// `[T1, T3)` (E25-H01) ya se detectó, el fichero de journal de la transacción abortada ya se ha
    /// **borrado** y su árbol de recuperación **todavía no**.
    ///
    /// Modela el proceso que muere entre los dos borrados. El orden importa y es lo que este punto
    /// fija: el journal va primero porque es lo que levanta el gate de `recovery_pending`, así que
    /// lo que sobrevive a la interrupción es un árbol de recuperación **sin journal** — un huérfano
    /// legítimo que recoge el GC (E24-H06). Al revés quedaría un journal apuntando a copias que ya
    /// no están, y la recuperación sellaría un estado parcial en silencio.
    ///
    /// STUB de la fase roja de E25-H02: la variante existe para que el test compile; **nadie la
    /// dispara todavía** (el camino de aborto no la ejerce porque el sellado del aborto aún no
    /// existe). El implementador coloca el `failpoint!` correspondiente entre los dos borrados.
    EnMedioDelSelladoDelAborto,
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

/// Punto del orquestador donde se ejecuta un **gancho** del test y la transacción **continúa**
/// (E25-H01).
///
/// Es el complemento de [`FailPoint`]: aquel solo sabe **abortar** —devuelve `Err` y la transacción
/// muere ahí—, y hay defectos que solo se manifiestan si algo pasa *y el flujo sigue*. El caso que
/// lo motivó es la ventana `[T1, T3)` de la publicación: para reproducir una edición externa
/// concurrente hace falta modificar el disco **entre** el conjunto respaldado y el bucle de
/// renames, sin interrumpir la transacción.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PuntoDeGancho {
    /// Dentro de la ventana `[T1, T3)`, en su último instante: tras `create_journal` e
    /// **inmediatamente antes** de `publish_result`. Copias de recuperación y journal ya cubren el
    /// conjunto afectado calculado en T1, y el bucle de publicación aún no ha sustituido nada.
    AntesDePublicar,
}

/// Gancho armado (el punto que lo dispara y el cierre a ejecutar), o nada.
type GanchoArmado = Option<(PuntoDeGancho, Box<dyn Fn()>)>;

thread_local! {
    /// Gancho armado para el hilo actual, o `None`. `thread_local` por el mismo motivo que
    /// [`ARMADO`]: los tests del repo corren en paralelo dentro del mismo proceso y un estado
    /// global haría que el gancho de un test interfiriese con la transacción de otro.
    static GANCHO: RefCell<GanchoArmado> = const { RefCell::new(None) };
}

/// Arma un gancho para el **hilo actual**. Se dispara **una sola vez** y se desarma solo, de modo
/// que una transacción posterior del mismo test no vuelva a ejecutarlo. Armar un gancho nuevo
/// sustituye al anterior.
pub fn armar_gancho(punto: PuntoDeGancho, gancho: impl Fn() + 'static) {
    GANCHO.with(|g| *g.borrow_mut() = Some((punto, Box::new(gancho))));
}

/// Desarma cualquier gancho del hilo actual (higiene: el gancho puede no haberse disparado).
pub fn desarmar_ganchos() {
    GANCHO.with(|g| *g.borrow_mut() = None);
}

/// Ejecuta el gancho armado para `punto`, si lo hay, y **continúa**: a diferencia de
/// [`disparado`], no aborta nada.
///
/// El gancho se extrae del `thread_local` **antes** de invocarlo (y con ello se desarma), así que
/// un gancho que a su vez entrara en el orquestador no volvería a dispararse ni haría panicar el
/// `RefCell` por doble préstamo.
pub(crate) fn ejecutar_gancho(punto: PuntoDeGancho) {
    let armado = GANCHO.with(|g| {
        let mut slot = g.borrow_mut();
        match slot.as_ref() {
            Some((p, _)) if *p == punto => slot.take().map(|(_, gancho)| gancho),
            _ => None,
        }
    });
    if let Some(gancho) = armado {
        gancho();
    }
}
