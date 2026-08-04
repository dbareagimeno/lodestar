---
id: 20
titulo: "Renombrado del proyecto"
estado: "abierta"
prioridad: 5
etiquetas: ["marca", "distribucion", "docs", "comunidad"]
origen: "criterio-de-producto"
abierta_en: "2026-08-02"
revisada_en: "2026-08-02"
congela: [1, 17]
relacionadas: [1, 9, 17]
---

# §20 — Renombrado del proyecto

- **Contexto** (2026-08-02, en la repriorización de las decisiones): el usuario tiene intención
  firme de **cambiar el nombre del proyecto**. La ficha se abre porque la colisión de marca que
  [`§17`](17-superficie-externa-oss.md) dejó «solo anotada» —*Lodestar* es también un cliente de
  consenso de Ethereum muy conocido (ChainSafe), lo que hunde la descubribilidad en buscadores— ha
  dejado de ser una nota al pie.
- **Por qué es prioridad 5 pese a no tocar el motor**: E27 acaba de convertir el nombre en
  superficie pública. Hoy lo llevan el repo, el README, `docs/user/`, los binarios de Releases, el
  **nombre del servidor MCP que un usuario escribe en su configuración**, los crates, los tags y el
  directorio de estado `.lodestar/`. El coste del renombrado **crece con el tiempo publicado**:
  enlaces, releases y configuraciones ajenas que arrastrar.
- **Estado**: **nombre por elegir**. El primer paso de la ejecución es escogerlo y **verificar
  disponibilidad** en GitHub, crates.io, dominio y buscadores — la verificación que
  [`§17-DA`](17-superficie-externa-oss.md) dejó pendiente, ahora con el nombre correcto.
- **Alcance decidido: TOTAL**, incluido `.lodestar/` y los identificadores internos. Consecuencia
  que hay que resolver en la ejecución: renombrar el directorio de estado **rompe los workspaces ya
  creados**, así que la épica debe llevar migración o compatibilidad con el nombre viejo (leer el
  directorio antiguo si existe, o un paso de migración explícito).
- **Orden decidido: después de la épica de honestidad de superficie.** El renombrado es mecánico y
  no caduca; los defectos de superficie los sufre cualquiera que pase por ahí mientras tanto. Y
  hacerlo después evita tocar dos veces los mismos ficheros de `docs/user/` que la épica de
  honestidad va a corregir.
- **Qué CONGELA mientras siga abierta** (regla transversal, en la línea de `§21.5`):
  - **La firma/notarización de binarios** ([`§1`](01-build-fachada-escritorio.md),
    [`§9`](09-transversales-diferidas.md)): los certificados son del desarrollador, no del binario,
    pero cablear la notarización y publicar releases firmadas con un nombre a punto de cambiar es
    gastar el ciclo de release dos veces.
  - **crates.io** ([`§17-DA`](17-superficie-externa-oss.md)): no se reservan nombres que se van a
    abandonar. `E27-H10` sigue bloqueada, ahora por este motivo.
  - **Cualquier difusión adicional** del proyecto bajo el nombre actual.
- **Qué NO congela**: todo lo que toca el motor (la épica de honestidad, el banco de pruebas de
  [`§14`](14-store-sin-consumidor.md), la higiene de [`§16`](16-deuda-auditoria-e25-e26.md)). El
  nombre es etiqueta, no comportamiento.
