//! Contabilidad determinista de memoria retenida (`ARCHITECTURE.md §23`).

/// Presupuesto total y sus tres reservas internas.
///
/// Este tipo no conecta ningún consumidor: es únicamente la contabilidad que construye el
/// `Workspace` al abrirse. Las cuotas se calculan con división entera sin multiplicar `N` antes
/// de dividir, para que incluso un `u64::MAX` no pueda desbordar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryBudget {
    total_bytes: u64,
    sqlite_bytes: u64,
    w_tiny_lfu_bytes: u64,
    work_bytes: u64,
}

impl MemoryBudget {
    /// Construye la partición exacta para un presupuesto positivo `N`.
    pub fn from_bytes(total_bytes: u64) -> Result<Self, String> {
        if total_bytes == 0 {
            return Err("el presupuesto de memoria debe ser mayor que cero".to_string());
        }

        let sqlite_bytes = cuota(total_bytes, 30);
        let w_tiny_lfu_bytes = cuota(total_bytes, 20);
        let work_bytes = total_bytes - sqlite_bytes - w_tiny_lfu_bytes;

        Ok(Self {
            total_bytes,
            sqlite_bytes,
            w_tiny_lfu_bytes,
            work_bytes,
        })
    }

    pub fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    pub fn sqlite_bytes(&self) -> u64 {
        self.sqlite_bytes
    }

    pub fn w_tiny_lfu_bytes(&self) -> u64 {
        self.w_tiny_lfu_bytes
    }

    pub fn work_bytes(&self) -> u64 {
        self.work_bytes
    }
}

/// `floor(percent * n / 100)` sin que `percent * n` pueda desbordar.
fn cuota(n: u64, percent: u64) -> u64 {
    (n / 100) * percent + ((n % 100) * percent) / 100
}
