# `lodestar-bench`

Banco interno, reproducible y permanente de rendimiento de Lodestar. El crate no se publica y sus
resultados son evidencia para detectar regresiones y evaluar optimizaciones; no forman parte del
contrato MCP ni prometen rendimiento al usuario final.

La documentación se mantiene separada en dos referencias:

- [Métricas actuales](../../docs/qa/benchmark-metricas-actuales.md): baseline H04/10k, sonda
  H09/100k, footprint, RSS y límites de interpretación.
- [Guía de uso](../../docs/qa/benchmark-guia-uso.md): modos smoke, full, extreme y gate, comandos,
  preflight, salidas y comparación de corridas.

Los resúmenes y el inventario de evidencia permanecen versionados; los volcados completos se
publican como artefactos comprimidos de la release y no entran en Git:

- [manifiesto de evidencia v0.6.2](../../docs/qa/corridas/v0.6.2/manifest.json), que enlaza la
  corrida H04 full y su
  [resumen](../../docs/qa/e33-h04-banco-rendimiento-2026-08-22.md);
- [resumen H09 Realista/100k](../../docs/qa/e33-h09-realista-100k-2026-08-23.md), cuyo bruto
  también está inventariado en el manifiesto.

En el manifiesto, `artifact` describe el asset `.json.gz` externo (URL, SHA-256, tamaño, tipo y
compresión), mientras `raw` describe el JSON descomprimido (SHA-256, tamaño y `schema_version`).
Los dos checksums son deliberadamente distintos: el primero permite verificar la descarga y el
segundo la carga útil original.

La sonda extrema acepta cualquier `--scale` positiva. El corpus generado es temporal y se elimina
al finalizar; las salidas JSON y Markdown solicitadas sobreviven. H09 queda fuera de full, smoke,
CI y del gate H05/10k.
