#!/usr/bin/env python3
"""Construye la página del Artifact con la matriz completa embebida."""
import argparse
import json
import html as H
from pathlib import Path


DEFAULT_INPUT_DIR = Path(__file__).resolve().parent
DEFAULT_OUTPUT = DEFAULT_INPUT_DIR / "artifact_lodestar.html"


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--input-dir", type=Path, default=DEFAULT_INPUT_DIR,
                    help="directorio con matriz_r1.json, matriz_r2.json y matriz_r3.json")
parser.add_argument("--out", type=Path, default=DEFAULT_OUTPUT,
                    help="fichero HTML de salida")
args = parser.parse_args()
input_dir = args.input_dir
output = args.out

casos = []
for n in (1, 2, 3):
    with (input_dir / f"matriz_r{n}.json").open(encoding="utf-8") as matriz:
        d = json.load(matriz)
    for c in d["casos"]:
        ver = c.get("verificacion") or {}
        final = ver.get("clasificacion") or ("PASS" if c["veredicto"] == "PASS" else c["veredicto"])
        casos.append({
            "id": c["id"], "ronda": n, "lote": c.get("lote", ""),
            "esperado": c.get("esperado", ""), "real": c.get("real", ""),
            "veredicto": c["veredicto"], "final": final,
            "bug": ver.get("es_bug_de", ""), "evidencia": ver.get("evidencia", ""),
        })

# H5 juzgado inline ya está en matriz_r3
resumen = {"total": len(casos)}
DATA = json.dumps(casos, ensure_ascii=False).replace("</", "<\\/")

FINDINGS = [
    ("motor", "M-01", "change_revert de un recibo -revert: no-op silencioso que destruye el redo",
     "Responde reverted:true con el MISMO receiptId, no restaura nada y sobrescribe las copias de recuperación del redo. Causa raíz: transaction_id se deriva del changeSetId original, así que el sufijo -revert nunca apila. (G1-18)"),
    ("doc", "D-01", "instructions nombra 10 tools bajo readonly; tools/list sirve 7",
     "Contra la garantía del contrato («nombra EXACTAMENTE las tools que sirve tools/list»). Además initialize acepta cualquier protocolVersion sin validar. (G1-24)"),
    ("doc", "D-02", "patch_frontmatter: §20.4 promete distinguir asignar-null de borrar; el wire es RFC 7386",
     "patch {clave: null} BORRA la clave; el brazo del core que asigna null explícito es inalcanzable desde MCP y ARCHITECTURE se contradice internamente. (G1-14)"),
    ("amb", "A-01", "sections con heading inexistente se omite en silencio", "body:\"\" o body acotado indistinguible de «todas las secciones existían». Solo lo fija un doc-comment interno. (PRJ-07)"),
    ("amb", "A-02", "Cursor malformado cae a offset 0 en silencio", "decode_cursor hace unwrap_or(0); choca con el principio declarado de rechazar valores que un despachador interpreta. (ROB-05)"),
    ("amb", "A-03", "Cursor de otra tool aceptado y reinterpretado", "Página válida pero semánticamente equivocada, sin señal de error. (ROB-06)"),
    ("amb", "A-04", "starts_with/ends_with sobre campo no-string: false silencioso", "Los 7 docs con priority:3 desaparecen de priority starts_with \"3\" sin error; eval.rs reconoce el hueco sin test. (G1-20)"),
    ("amb", "A-05", "create sobre path existente y move a destino ocupado: canApply true", "Aplicados pisarían conocimiento; ningún código de colisión declarado. El hueco con más riesgo práctico. (G1-11)"),
    ("amb", "A-06", "replace_text sin ocurrencias y sin aserción: plan no-op silencioso", "El vacío-sin-error solo está documentado para selecciones masivas. (G1-13)"),
    ("amb", "A-07", "knowledge_check scope paths traga paths inexistentes", "paths:[\"no-existe.md\"] a solas → 0 diagnósticos, sin error: un typo desaparece. (G1-23)"),
    ("amb", "A-08", "Sintaxis de validation sin documentar (familias, no códigos)", "LINK-TARGET-MISSING: ignore es silenciosamente inerte; las familias solo existen en config.rs. (G1-04)"),
    ("amb", "A-09", "La config se lee una vez por sesión y ninguna fuente lo declara", "Un config.yaml escrito con el servidor vivo no se aplica (GC siguió con maximumReceipts=20 cacheado). (G2-04)"),
    ("amb", "A-10", "Nit: «path que normaliza a un directorio» en mcp.yml es impreciso", "Solo la RAÍZ da workspaceDirectory; un directorio con nombre es missing. (G2-10)"),
]

cards = "\n".join(
    f'<article class="card {k}"><div class="card-head"><span class="tag {k}">{ {"motor":"BUG DE MOTOR","doc":"DOC CONFIRMADA","amb":"CONTRATO AMBIGUO"}[k] }</span><span class="fid">{fid}</span></div>'
    f"<h3>{H.escape(t)}</h3><p>{H.escape(b)}</p></article>"
    for k, fid, t, b in FINDINGS)

page = """<title>lodestar × homelab — informe de pruebas</title>
<style>
:root{
  --bg:#f6f6f3; --panel:#ffffff; --ink:#1d2126; --muted:#5b6470; --line:#e3e2dc;
  --accent:#8a6d1f; --accent-ink:#6e5717;
  --ok:#2c7a4b; --err:#b42318; --warn:#b54708; --amb:#475467;
  --ok-bg:#e8f3ec; --err-bg:#fbeae8; --warn-bg:#fdf1e3; --amb-bg:#eceef2;
  --mono:ui-monospace,"SF Mono",SFMono-Regular,Menlo,Consolas,monospace;
}
@media (prefers-color-scheme: dark){:root{
  --bg:#12151a; --panel:#1a1e25; --ink:#e6e4dd; --muted:#98a1ad; --line:#2a2f38;
  --accent:#d4af4a; --accent-ink:#e2c469;
  --ok:#5dbb86; --err:#f08a7e; --warn:#e8a75c; --amb:#a5aebc;
  --ok-bg:#1a2b22; --err-bg:#33201d; --warn-bg:#32271a; --amb-bg:#232833;
}}
:root[data-theme="dark"]{
  --bg:#12151a; --panel:#1a1e25; --ink:#e6e4dd; --muted:#98a1ad; --line:#2a2f38;
  --accent:#d4af4a; --accent-ink:#e2c469;
  --ok:#5dbb86; --err:#f08a7e; --warn:#e8a75c; --amb:#a5aebc;
  --ok-bg:#1a2b22; --err-bg:#33201d; --warn-bg:#32271a; --amb-bg:#232833;
}
:root[data-theme="light"]{
  --bg:#f6f6f3; --panel:#ffffff; --ink:#1d2126; --muted:#5b6470; --line:#e3e2dc;
  --accent:#8a6d1f; --accent-ink:#6e5717;
  --ok:#2c7a4b; --err:#b42318; --warn:#b54708; --amb:#475467;
  --ok-bg:#e8f3ec; --err-bg:#fbeae8; --warn-bg:#fdf1e3; --amb-bg:#eceef2;
}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--ink);font:15px/1.55 system-ui,-apple-system,"Segoe UI",sans-serif}
.wrap{max-width:1080px;margin:0 auto;padding:40px 24px 80px}
header.top h1{font-size:1.7rem;margin:0 0 4px;letter-spacing:-.01em;text-wrap:balance}
header.top .sub{color:var(--muted);max-width:70ch}
.kicker{font-family:var(--mono);font-size:.72rem;letter-spacing:.14em;text-transform:uppercase;color:var(--accent-ink);margin-bottom:10px}
.stats{display:grid;grid-template-columns:repeat(auto-fit,minmax(140px,1fr));gap:12px;margin:28px 0 8px}
.stat{background:var(--panel);border:1px solid var(--line);border-radius:8px;padding:14px 16px}
.stat b{display:block;font-size:1.6rem;font-variant-numeric:tabular-nums;letter-spacing:-.02em}
.stat span{color:var(--muted);font-size:.8rem}
.stat.err b{color:var(--err)} .stat.warn b{color:var(--warn)} .stat.ok b{color:var(--ok)} .stat.amb b{color:var(--amb)}
h2{font-size:1.15rem;margin:44px 0 14px;padding-top:18px;border-top:1px solid var(--line)}
.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(320px,1fr));gap:14px}
.card{background:var(--panel);border:1px solid var(--line);border-left:3px solid var(--amb);border-radius:6px;padding:14px 16px;display:flex;flex-direction:column;gap:8px}
.card.motor{border-left-color:var(--err)} .card.doc{border-left-color:var(--warn)}
.card h3{margin:0;font-size:.95rem;line-height:1.35;text-wrap:balance}
.card p{margin:0;color:var(--muted);font-size:.84rem}
.card-head{display:flex;align-items:center;gap:10px}
.fid{font-family:var(--mono);font-size:.75rem;color:var(--muted)}
.tag{font-family:var(--mono);font-size:.66rem;letter-spacing:.08em;padding:2px 8px;border-radius:99px}
.tag.motor{background:var(--err-bg);color:var(--err)}
.tag.doc{background:var(--warn-bg);color:var(--warn)}
.tag.amb{background:var(--amb-bg);color:var(--amb)}
.controls{display:flex;flex-wrap:wrap;gap:8px;margin:0 0 14px;align-items:center}
.controls input{flex:1;min-width:220px;background:var(--panel);border:1px solid var(--line);border-radius:6px;padding:8px 12px;color:var(--ink);font:inherit}
.chip{background:var(--panel);border:1px solid var(--line);border-radius:99px;padding:5px 12px;font-size:.8rem;cursor:pointer;color:var(--muted)}
.chip[aria-pressed="true"]{border-color:var(--accent);color:var(--accent-ink)}
.chip:focus-visible,.controls input:focus-visible,details summary:focus-visible{outline:2px solid var(--accent);outline-offset:2px}
.tablewrap{overflow-x:auto;border:1px solid var(--line);border-radius:8px;background:var(--panel)}
table{border-collapse:collapse;width:100%;min-width:760px;font-size:.83rem}
th{position:sticky;top:0;background:var(--panel);text-align:left;font-family:var(--mono);font-size:.68rem;letter-spacing:.08em;text-transform:uppercase;color:var(--muted);padding:10px 12px;border-bottom:1px solid var(--line)}
td{padding:9px 12px;border-bottom:1px solid var(--line);vertical-align:top}
tr:last-child td{border-bottom:none}
td.id{font-family:var(--mono);white-space:nowrap}
.pill{display:inline-block;font-family:var(--mono);font-size:.68rem;padding:2px 8px;border-radius:99px;white-space:nowrap}
.pill.PASS,.pill.RECLASIFICADO-PASS{background:var(--ok-bg);color:var(--ok)}
.pill.CONFIRMADO-DISCREPANCIA{background:var(--err-bg);color:var(--err)}
.pill.CONTRATO-AMBIGUO{background:var(--warn-bg);color:var(--warn)}
.pill.UNCLEAR,.pill.FAIL{background:var(--amb-bg);color:var(--amb)}
details{margin-top:6px}
details summary{cursor:pointer;color:var(--muted);font-size:.78rem}
details .det{white-space:pre-wrap;font-size:.78rem;color:var(--muted);margin-top:6px;border-left:2px solid var(--line);padding-left:10px}
.count{color:var(--muted);font-size:.8rem;margin-left:auto;font-variant-numeric:tabular-nums}
footer{margin-top:48px;color:var(--muted);font-size:.8rem;border-top:1px solid var(--line);padding-top:16px}
code{font-family:var(--mono);font-size:.86em}
@media (prefers-reduced-motion: no-preference){.card{transition:border-color .15s}}
</style>
<div class="wrap">
<header class="top">
  <div class="kicker">testbench · 2026-08-06 · lodestar-mcp v0.5.0</div>
  <h1>lodestar × homelab — pruebas extensivas de la superficie MCP</h1>
  <p class="sub">189 casos con resultado esperado citando fuente (contrato, docs de usuario, ARCHITECTURE), ejecutados sobre el workspace real del homelab (lecturas en <code>readonly</code>) y worktrees efímeros (mutaciones). Todo veredicto no-PASS pasó verificación adversarial. Tres rondas hasta secar el bucle de casos esquina. El repo homelab quedó byte-idéntico (misma revisión blake3).</p>
</header>

<div class="stats">
  <div class="stat"><b>189</b><span>casos ejecutados</span></div>
  <div class="stat ok"><b>176</b><span>conformes con el contrato</span></div>
  <div class="stat err"><b>1</b><span>bug de motor</span></div>
  <div class="stat warn"><b>2</b><span>discrepancias doc confirmadas</span></div>
  <div class="stat amb"><b>10</b><span>huecos contractuales</span></div>
  <div class="stat"><b>12</b><span>esperados nuestros refutados</span></div>
</div>

<h2>Hallazgos</h2>
<div class="grid">
__CARDS__
</div>

<h2>Matriz completa</h2>
<div class="controls">
  <input id="q" type="search" placeholder="Filtrar por id, tool, texto…" aria-label="Filtrar casos">
  <button class="chip" data-f="todos" aria-pressed="true">Todos</button>
  <button class="chip" data-f="hallazgos" aria-pressed="false">Solo hallazgos</button>
  <button class="chip" data-f="r1" aria-pressed="false">Ronda 1</button>
  <button class="chip" data-f="r2" aria-pressed="false">Ronda 2</button>
  <button class="chip" data-f="r3" aria-pressed="false">Ronda 3</button>
  <span class="count" id="count"></span>
</div>
<div class="tablewrap">
<table>
<thead><tr><th>Caso</th><th>R</th><th>Resultado final</th><th>Comportamiento real (resumen)</th></tr></thead>
<tbody id="rows"></tbody>
</table>
</div>

<footer>
Informe completo: <code>docs/qa/informe-homelab-2026-08-06.md</code> en el repo lodestar. Arnés y specs reproducibles en el scratchpad del testbench (<code>lodestar_harness.py</code>, <code>batches/*.json</code>). «Resultado final» = veredicto del ejecutor corregido por la verificación adversarial: RECLASIFICADO-PASS significa que el motor coincidía con la fuente y el esperado del test estaba mal derivado.
</footer>
</div>
<script>
const DATA = __DATA__;
const rows = document.getElementById('rows');
const q = document.getElementById('q');
const count = document.getElementById('count');
let filtro = 'todos';
const esc = s => (s||'').replace(/[&<>"]/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[c]));
const esHallazgo = c => c.final === 'CONFIRMADO-DISCREPANCIA' || c.final === 'CONTRATO-AMBIGUO';
function render(){
  const term = q.value.toLowerCase();
  let vis = 0, out = [];
  for (const c of DATA){
    if (filtro === 'hallazgos' && !esHallazgo(c)) continue;
    if (filtro.startsWith('r') && filtro.length === 2 && c.ronda !== +filtro[1]) continue;
    if (term && !(c.id + ' ' + c.lote + ' ' + c.esperado + ' ' + c.real + ' ' + c.bug).toLowerCase().includes(term)) continue;
    vis++;
    const det = (c.evidencia || c.esperado) ?
      `<details><summary>esperado · evidencia</summary><div class="det">${esc('ESPERADO: ' + c.esperado + (c.evidencia ? '\\n\\nVERIFICACIÓN: ' + c.evidencia : ''))}</div></details>` : '';
    out.push(`<tr><td class="id">${esc(c.id)}</td><td>${c.ronda}</td><td><span class="pill ${esc(c.final)}">${esc(c.final)}</span>${c.bug && c.bug !== 'ninguno' ? `<div class="fid">bug de: ${esc(c.bug)}</div>` : ''}</td><td>${esc(c.real.slice(0, 400))}${c.real.length > 400 ? '…' : ''}${det}</td></tr>`);
  }
  rows.innerHTML = out.join('');
  count.textContent = vis + ' / ' + DATA.length;
}
document.querySelectorAll('.chip').forEach(b => b.addEventListener('click', () => {
  filtro = b.dataset.f;
  document.querySelectorAll('.chip').forEach(x => x.setAttribute('aria-pressed', String(x === b)));
  render();
}));
q.addEventListener('input', render);
render();
</script>
"""
page = page.replace("__CARDS__", cards).replace("__DATA__", DATA)
output.parent.mkdir(parents=True, exist_ok=True)
with output.open("w", encoding="utf-8") as artifact:
    artifact.write(page)
print("artifact generado:", len(page), "bytes,", len(casos), "casos")
