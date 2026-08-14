---
name: juzgar
description: Lanza jueces frescos con solo spec, diff completo, autoridades y gates; sintetiza por evidencia y no por el peor voto. Úsalo antes de cerrar un bug o historia y tras cada reparación.
argument-hint: "[ID E<n>-H<nn>] [--panel] [--rango <a..b>]"
---

# /juzgar — revisión fresca

1. Prepara el expediente sin contexto del implementador:
   - spec o reproducción;
   - diff committed contra `develop`, staged, unstaged y ficheros nuevos;
   - autoridades relevantes;
   - evidencia cruda de rojo, lock y gates.
2. Para un bug pequeño, lanza dos jueces nuevos: corrección y calidad de tests.
3. Con `--panel`, frontera MCP, dependencias, rutas, seguridad o transacciones, lanza tres jueces
   nuevos en paralelo: corrección, arquitectura y tests.
4. Mantén a los jueces en modo de solo lectura y no reutilices ninguno tras reparar.
5. Sintetiza por evidencia:
   - criterio incumplido, invariante roto o fallo reproducible bloquea;
   - riesgo plausible sin reproducción exige investigación;
   - sospecha especulativa no se vuelve bloqueante por votación.

Reporta veredicto, matriz criterio-evidencia, hallazgos con fichero/línea y gates. No apliques una
decisión normativa; sí puedes reparar un bug inequívoco solicitado por el ciclo.
