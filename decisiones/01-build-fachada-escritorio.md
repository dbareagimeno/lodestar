---
id: 1
titulo: "Firma/notarización de binarios (ex-build de la fachada de escritorio Tauri)"
estado: "diferida"
prioridad: 3
etiquetas: ["distribucion", "ui", "seguridad"]
origen: "puerta-de-diseno"
abierta_en: "2026-07-01"
revisada_en: "2026-08-02"
epica: "E6"
historias: ["E8-H06"]
congelada_por: 20
relacionadas: [2, 17, 20]
---

# §1 — Build de la fachada de escritorio Tauri (E6)

- **Estado**: `src-tauri` es ahora una **fachada Tauri v2 real y compilada**: tabla de comandos con
  los nombres congelados (`open_bundle`/`get_snapshot`/`read_concept`/`write_concept`/`create_concept`/
  `conformance`/`query`/`backlinks`/`graph_model`/… + `history`/`diff_working`/`commit`), estado del
  bundle abierto, y un **forwarder** que reemite el bus `IndexEvent` de la cache como `bundle:changed`
  (watcher + escrituras → UI en vivo). Compila en este entorno (webkit disponible) y produce el binario
  `lodestar-desktop`. El **CI de Rust** ya instala las libs de sistema (`libwebkit2gtk-4.1-dev`,
  `libsoup-3.0-dev`, …) y construye el `frontend/dist` antes del `cargo build` (Tauri lo embebe).
- **Empaquetado/release — PARCIALMENTE RESUELTO (v0.1.0)**:
  - **Plataformas objetivo cerradas**: **macOS Apple Silicon (arm64)**, **Windows** y **Linux**.
    Existe pipeline de release (`.github/workflows/release.yml`) que se dispara con el tag `vX.Y.Z`,
    compila las tres plataformas y crea un GitHub Release en **borrador** con los bundles (dmg/deb/
    appimage/nsis) + los binarios de CLI/MCP. `bundle.active = true` y los **iconos de marca** (la
    estrella dorada) ya están integrados. Runbook en `RELEASING.md`.
  - **Firma/notarización — DIFERIDA (no cerrada)**: los bundles de v0.1.0 salen **SIN FIRMAR** para
    las tres plataformas (avisos de Gatekeeper/SmartScreen al instalar). Queda pendiente decidir e
    integrar certificados + notarización cuando se quiera distribución sin fricción (§12 packaging,
    E8-H06). **No es un no-go**; es trabajo de infraestructura + secretos.
  - **Updater**: sigue sin cablear (no bloquea; la distribución es por descarga manual del Release).
  - **crates.io — PREPARADO, SIN PUBLICAR**: el orden topológico y los `publish = false` (fixtures,
    tauri) están listos (ver `RELEASING.md`), pero **no se publica**: el repo es
    **privado** y `cargo publish` haría el código público y permanente. Queda a criterio del usuario.
- **Recomendación**: v0.1.0 ya se distribuye por Release multiplataforma sin firmar; abordar la
  firma/notarización (y opcionalmente el updater) en una iteración posterior, según necesidad real de
  distribución amplia.

## Repriorización 2026-08-02

- **La única mitad viva de esta ficha es la firma/notarización**, y **sube a prioridad 3**: lo que
  la mantenía baja era que la distribución fuese hipotética, y E27 convirtió los binarios de
  GitHub Releases en el **camino de instalación recomendado** del proyecto. El aviso de Gatekeeper /
  SmartScreen es hoy la primera experiencia de cualquier desconocido.
- **CONGELADA por [`§20`](20-renombrado-del-proyecto.md)** (renombrado del proyecto, alcance total
  incluidos los nombres de los binarios): los certificados son del desarrollador, no del binario,
  pero publicar releases firmadas y notarizadas con un nombre a punto de cambiar es gastar el ciclo
  de release dos veces. **Mientras tanto**, lo barato y sin coste hundido: documentar el aviso y
  cómo saltarlo en `docs/user/`.
- **La mitad de crates.io** de esta ficha está superada por [`§17-DA`](17-superficie-externa-oss.md),
  y también congelada por §20 — no se reservan nombres que se van a abandonar. Nota histórica: el
  párrafo dice «el repo es privado»; **ya no lo es** desde la apertura OSS de E27.
- La mitad de la **fachada de escritorio** es archivo: `src-tauri`/`frontend` viven en
  `experimental/ui-desktop` desde el giro headless.
