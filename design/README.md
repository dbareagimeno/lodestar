# design/ — especificación visual ratificable

Artefactos de UX generados por `/ux` (agente `disenador-ux`). Son **bocetos ratificables que
sirven de spec visual a las historias — no código de producción**: `frontend/` nunca los importa
y no se mantienen sincronizados con la app.

```
design/
├── flujos/     *.excalidraw       — flujos de usuario (abrir en excalidraw.com o extensión VS Code)
└── mockups/    <slug>--<estado>.html — un boceto autocontenido por pantalla/estado
```

Convenciones:

- **Flujos**: cubren camino feliz **y** caminos de error/cancelación. JSON de Excalidraw
  versionable; los flujos triviales van como Mermaid dentro de la propia historia.
- **Mockups**: un fichero por pantalla/estado (`--vacio`, `--cargando`, `--error`, `--exito`…),
  CSS inline con las variables del prototipo, datos de ejemplo del dominio (nada de lorem ipsum).
- **Ciclo de vida**: un artefacto se ratifica antes de `/historia`, la historia lo cita en sus
  Referencias, y cuando la historia cierra el artefacto queda como registro histórico (no se
  actualiza si la UI evoluciona después — la verdad viva es `frontend/`).
