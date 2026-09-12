//! Repair17: el pico RSS del rebuild debe ser una lectura residente del proceso, no un
//! número positivo de respaldo.

#![cfg(any(target_os = "linux", target_os = "macos"))]

use lodestar_store::Store;
use std::path::Path;

fn write(path: &Path, contents: impl AsRef<[u8]>) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

#[allow(deprecated)]
fn independent_process_rss_bytes() -> u64 {
    #[cfg(target_os = "linux")]
    {
        let statm = std::fs::read_to_string("/proc/self/statm")
            .expect("repair17: lectura independiente de /proc/self/statm");
        let pages = statm
            .split_whitespace()
            .nth(1)
            .and_then(|value| value.parse::<u64>().ok())
            .expect("repair17: /proc/self/statm debe exponer páginas RSS");
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        assert!(page_size > 0, "repair17: tamaño de página válido");
        pages.saturating_mul(page_size as u64)
    }

    #[cfg(target_os = "macos")]
    {
        // La misma fuente de verdad del kernel que la ruta de producción macOS, llamada desde
        // este test de integración y no desde el helper privado del store.
        let mut info = std::mem::MaybeUninit::<libc::mach_task_basic_info_data_t>::zeroed();
        let mut count = libc::MACH_TASK_BASIC_INFO_COUNT;
        let result = unsafe {
            libc::task_info(
                libc::mach_task_self(),
                libc::MACH_TASK_BASIC_INFO,
                info.as_mut_ptr() as libc::task_info_t,
                &mut count,
            )
        };
        assert_eq!(
            result, 0,
            "repair17: task_info independiente debe funcionar"
        );
        unsafe { info.assume_init().resident_size }
    }
}

/// C4/ARCH §20.12.2 — el informe del rebuild debe quedar cerca de una observación RSS
/// independiente del mismo proceso. La mitad permite que el proceso cambie entre las dos
/// lecturas, pero hace imposible que `process_rss_bytes -> Ok(1)` sobreviva.
#[test]
fn c4_peak_rss_bytes_matches_independent_process_rss_and_rejects_sentinel() {
    let before = independent_process_rss_bytes();
    assert!(
        before > 8 * 1024,
        "repair17: RSS independiente no vacua: {before}"
    );

    let root = tempfile::tempdir().unwrap();
    for index in 0..32 {
        write(
            &root.path().join(format!("docs/{index:02}.md")),
            format!(
                "---\ntitle: repair17-{index}\n---\n\n# repair17-{index}\n\n{}\n",
                "resident rss evidence ".repeat(512)
            ),
        );
    }

    let report = Store::open(root.path()).unwrap().rebuild().unwrap();
    let after = independent_process_rss_bytes();
    assert!(
        after > 8 * 1024,
        "repair17: RSS independiente posterior no vacua: {after}"
    );
    let independent_floor = before.min(after) / 2;
    assert!(
        independent_floor > 4 * 1024,
        "repair17: tolerancia RSS independiente no vacua: before={before}, after={after}"
    );

    let phases = report["phases"]
        .as_array()
        .expect("repair17: phases debe ser un array");
    assert_eq!(
        phases.len(),
        4,
        "repair17: rebuild completo con cuatro fases"
    );
    let peaks: Vec<u64> = phases
        .iter()
        .map(|phase| {
            phase["peak_rss_bytes"]
                .as_u64()
                .unwrap_or_else(|| panic!("repair17: falta peak_rss_bytes: {phase}"))
        })
        .collect();
    assert!(
        peaks.iter().all(|peak| *peak >= independent_floor),
        "C4: cada ventana debe observar RSS residente real; peaks={peaks:?}, before={before}, after={after}, floor={independent_floor}"
    );

    let reported_peak = report["peak_rss_bytes"]
        .as_u64()
        .expect("repair17: peak_rss_bytes global");
    assert_eq!(reported_peak, peaks.iter().copied().max().unwrap());
    assert!(
        reported_peak >= independent_floor,
        "C4: el pico global no puede ser un centinela positivo: reportado={reported_peak}, RSS independiente before={before}, after={after}"
    );

    // Guarda anti-vacuidad: el rebuild realmente procesó el corpus que hace significativa la
    // observación, y no una respuesta sintética sin trabajo.
    assert_eq!(
        report["documents_read"].as_u64(),
        Some(32),
        "repair17: documents_read debe reflejar el corpus real"
    );
    assert!(
        report["rows_written"]
            .as_u64()
            .is_some_and(|rows| rows > 32),
        "repair17: rows_written debe demostrar proyección real"
    );
}
