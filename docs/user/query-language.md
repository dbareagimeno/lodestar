# The query language

Lodestar can select documents by the **real types** of their YAML frontmatter and by **computed
properties** of the graph: `priority >= 2` is a number comparison, `owners contains "platform"` is
list membership, `graph.backlinks = 0` asks the link graph. There is no schema to declare and no
index to build first — the language runs over the Markdown as it is on disk.

The same language has two surfaces, and they are equivalent by construction:

- **`where`** — a string, for humans and for models that write queries as text;
- **`filter`** — a JSON tree, for clients that build queries programmatically.

Both compile to the same expression, so the same question always gets the same answer whichever way
you ask it.

Every request and response on this page comes from a real run against
[`examples/demo/`](../../examples/demo/README.md) — the ten-document demo workspace — unless a
section says otherwise. Responses are trimmed to the fields under discussion; content-derived values
(`blake3:…` revisions) differ in your run. To reproduce them by hand, see
[Poking the server by hand](mcp-clients.md#poking-the-server-by-hand). Long error messages are
wrapped here for readability; the engine emits each one on a single line.

- [Where the language is accepted](#where-the-language-is-accepted)
- [Values and types](#values-and-types)
- [Fields and dot-paths](#fields-and-dot-paths)
- [Reserved namespaces](#reserved-namespaces)
- [Operators](#operators)
- [`has` and `missing`](#has-and-missing)
- [Combining conditions](#combining-conditions)
- [The JSON `filter` form](#the-json-filter-form)
- [Projecting fields with `include`](#projecting-fields-with-include)
- [When a query fails](#when-a-query-fails)
- [Declared limits](#declared-limits)
- [Reference](#reference)

## Where the language is accepted

| Tool | Parameter | What it does |
|---|---|---|
| `knowledge_search` | `where` / `filter` | Selects the documents returned. Intersected with `text` (free-text search) when both are given. |
| `change_plan` | `selection.where` / `selection.filter` | Selects the documents a bulk operation expands over — see [safe-changes.md](safe-changes.md#bulk-selections). |

Passing both `where` and `filter` to `knowledge_search` combines them with **and**.

```json
knowledge_search
{"where": "has(service) and service.tier = 1", "include": ["frontmatter.oncall"]}
```

```json
{
  "results": [
    {"path": "runbooks/deploy.md",            "title": "Runbook: deploy",            "frontmatter": {"oncall": "platform"}},
    {"path": "runbooks/incident-response.md", "title": "Runbook: incident response", "frontmatter": {"oncall": "platform"}}
  ],
  "totalApproximate": 2
}
```

## Values and types

The type of a literal comes from how you **write** it, and nothing is coerced afterwards:

| You write | Type | Notes |
|---|---|---|
| `"active"` | string | Double quotes. `\"` and `\\` escape inside them. |
| `2`, `-3`, `1.5` | number | |
| `true`, `false` | boolean | |
| `null` | null | Matches an explicit `key:` with no value, not an absent key. |
| `["a", "b"]` | list | Only as the right-hand side of `contains_any` / `contains_all`. |
| `draft` | string | A bare word that is not a number, a boolean or `null` is a string. |

Single quotes are **not** string delimiters:

```json
knowledge_search
{"where": "status = 'active'"}
```

```text
INVALID_SCHEMA: «where» inválido: carácter inesperado: '\''
```

Types never cross. `service.tier` is the number `1` in the demo, so comparing it to the string
`"1"` matches nothing — and that is a legitimate answer, not an error:

```json
knowledge_search
{"where": "service.tier = \"1\""}
```

```json
{"results": [], "totalApproximate": 0}
```

`=` and `!=` are **never** type errors: a type mismatch is simply `false`. Ordering comparisons are
stricter — see [When a query fails](#when-a-query-fails).

## Fields and dot-paths

A field is addressed with dot-notation, descending into nested YAML maps:

```yaml
---
service:
  name: atlas-api
  tier: 1
oncall: platform
---
```

```text
oncall = "platform"
service.tier = 1
```

A bare name is the frontmatter of the document. Writing `frontmatter.` in front of it is optional
and means the same thing — `frontmatter.status` ≡ `status` — with one deliberate exception: the
prefix is an **anchor**, so it forces the lookup into your frontmatter even when the rest of the
path starts with a reserved namespace. `frontmatter.graph.backlinks` reads *your* `graph:` key;
`graph.backlinks` reads the link graph.

A field that no document has is **not** an error. It matches nothing, which is what "no document has
that field" should look like:

```json
knowledge_search
{"where": "nonexistent_field = 3"}
```

```json
{"results": [], "totalApproximate": 0}
```

## Reserved namespaces

Two namespaces are computed by the engine rather than read from frontmatter. They **require** the
explicit prefix, and every document has them.

| Property | Type | Meaning |
|---|---|---|
| `document.path` | string | Path relative to the workspace root, with `/` separators |
| `document.title` | string | Derived title: `frontmatter.title` → first H1 → file name |
| `document.has_frontmatter` | boolean | Whether the document has a frontmatter block at all |
| `graph.backlinks` | number | Internal links pointing at this document |
| `graph.outgoing_links` | number | Internal links leaving this document |
| `graph.dangling_links` | number | Links from this document with no target |
| `graph.isolated` | boolean | No internal links in **or** out |

```json
knowledge_search
{"where": "graph.backlinks >= 2 and document.path starts_with \"runbooks/\""}
```

```json
{
  "results": [
    {"path": "runbooks/backup.md",            "title": "Runbook: backups"},
    {"path": "runbooks/deploy.md",            "title": "Runbook: deploy"},
    {"path": "runbooks/incident-response.md", "title": "Runbook: incident response"}
  ],
  "totalApproximate": 3
}
```

Under a reserved namespace an unknown property is an **error**, not an empty result — a typo in
`graph.backlinks` used to look exactly like "nothing matched":

```json
knowledge_search
{"where": "graph.backlink > 0"}
```

```text
INVALID_SCHEMA: «where» inválido: `graph.backlink` no existe: el namespace `graph` solo tiene
`graph.backlinks`, `graph.outgoing_links`, `graph.dangling_links`, `graph.isolated`. Para consultar
una clave de TU frontmatter con ese nombre, ánclala con `frontmatter.graph.backlink`
```

(The message is in Spanish, like every message the engine emits today; the code, the property names
and the field names are stable and in English. See the note in
[quickstart.md](quickstart.md#2-run-your-first-check).)

## Operators

| `where` | `filter` `operator` | Applies to | Meaning |
|---|---|---|---|
| `=` | `equals` | any | Equal in value **and** type; a type mismatch is `false` |
| `!=` | `not_equals` | any | The negation of `=` |
| `>` | `greater_than` | number vs number, string vs string | Ordering; strings compare lexicographically |
| `>=` | `greater_than_or_equal` | idem | |
| `<` | `less_than` | idem | |
| `<=` | `less_than_or_equal` | idem | |
| `contains` | `contains` | string or list | Substring on a string, membership on a list — the **field's** type decides |
| `starts_with` | `starts_with` | string (on **both** sides) | Prefix — a non-string field *or* literal is a type error, not `false` |
| `ends_with` | `ends_with` | string (on **both** sides) | Suffix — same rule |
| `contains_any` | `contains_any` | list | Shares at least one element with the literal list |
| `contains_all` | `contains_all` | list | Contains every element of the literal list |

```json
knowledge_search
{"where": "tags contains \"architecture\"", "include": ["frontmatter.tags"]}
```

```json
{
  "results": [
    {"path": "adr/0001-markdown-source-of-truth.md", "frontmatter": {"tags": ["architecture", "storage"]}},
    {"path": "adr/0002-event-bus.md",                "frontmatter": {"tags": ["architecture", "queue"]}}
  ],
  "totalApproximate": 2
}
```

`contains_all` takes a literal list:

```json
knowledge_search
{"where": "tags contains_all [\"architecture\", \"storage\"]"}
```

```json
{
  "results": [{"path": "adr/0001-markdown-source-of-truth.md", "title": "ADR-0001: Markdown files are the source of truth"}],
  "totalApproximate": 1
}
```

## `has` and `missing`

`has(field)` is "this property is present", `missing(field)` its negation. Presence is judged on the
key, not the value: a key whose value is `null`, `""` or `[]` is present.

```json
knowledge_search
{"where": "missing(status) and has(oncall)", "include": ["frontmatter.oncall"]}
```

```json
{
  "results": [
    {"path": "runbooks/deploy.md",            "frontmatter": {"oncall": "platform"}},
    {"path": "runbooks/incident-response.md", "frontmatter": {"oncall": "platform"}}
  ],
  "totalApproximate": 2
}
```

Both functions respect namespaces exactly as a comparison does, so `has(graph.backlinks)` asks about
a computed property — which every document has, making it trivially true — while
`has(frontmatter.graph)` asks whether your frontmatter has a `graph:` key.

`frontmatter` on its own is the one argument that does not name a key: it names the **block**.
`has(frontmatter)` matches the documents that carry a frontmatter block and `missing(frontmatter)`
those that do not — the same answer as `document.has_frontmatter`, which remains available and is
the form to use inside a larger comparison. A block that opens and closes with no keys (`---\n---`)
counts as present, by both routes.

```json
knowledge_search
{"where": "has(frontmatter)", "limit": 20}
```

```json
{"totalApproximate": 7}
```

(Seven of the demo's ten documents carry frontmatter.)

**Bare `frontmatter` inside a comparison** resolves to that same presence flag — a **boolean** — so
it behaves like any boolean field. `frontmatter = true` matches exactly the documents that carry a
block (the same set as `has(frontmatter)` and as `document.has_frontmatter = true`), and any
operator that is not defined on a boolean is a type error rather than a quiet `false`:
`frontmatter > 1` and `frontmatter contains "x"` both abort with `INVALID_SCHEMA` naming `bool` as
the type found, and so does `frontmatter starts_with "x"` — a boolean has no prefix. In a document
with **no** block the anchor resolves to nothing at all, which is absence, so those same
comparisons are an ordinary `false` there. That is why `frontmatter = false` matches nobody while
`missing(frontmatter)` matches the documents without a block: absence is not the boolean `false`.
`document.has_frontmatter` remains the explicit form, and reads better in a larger expression.

## Combining conditions

`and`, `or`, `not` and parentheses, with `not` binding tightest and `or` loosest:

```json
knowledge_search
{"where": "not (status = \"accepted\") and has(status)", "include": ["frontmatter.status"]}
```

```json
{
  "results": [
    {"path": "adr/0002-event-bus.md", "frontmatter": {"status": "proposed"}},
    {"path": "overview.md",           "frontmatter": {"status": "active"}}
  ],
  "totalApproximate": 2
}
```

`text` is free-text search over the file name, the frontmatter values and the body. Combined with a
query, the two intersect:

```json
knowledge_search
{"text": "bus", "where": "status = \"proposed\""}
```

```json
{
  "results": [{"path": "adr/0002-event-bus.md", "title": "ADR-0002: A single event bus between collector and writers"}],
  "totalApproximate": 1
}
```

## The JSON `filter` form

A comparison is `{"field": …, "operator": …, "value": …}`; the wrappers are `{"and": [...]}`,
`{"or": [...]}`, `{"not": …}`, `{"has": {"field": …}}` and `{"missing": {"field": …}}`. Operator
names are the long ones from the [operators table](#operators).

This filter and `tags contains "architecture"` are the same query, and return the same two documents
shown above:

```json
knowledge_search
{"filter": {"and": [{"field": "tags", "operator": "contains", "value": "architecture"}]},
 "include": ["frontmatter.tags"]}
```

```json
{
  "results": [
    {"path": "adr/0001-markdown-source-of-truth.md", "frontmatter": {"tags": ["architecture", "storage"]}},
    {"path": "adr/0002-event-bus.md",                "frontmatter": {"tags": ["architecture", "queue"]}}
  ],
  "totalApproximate": 2
}
```

A larger one — "tagged `queue` or `overview`, or without a `status`":

```json
knowledge_search
{"filter": {"or": [{"field": "tags", "operator": "contains_any", "value": ["queue", "overview"]},
                   {"missing": {"field": "status"}}]},
 "limit": 20}
```

```json
{"totalApproximate": 9}
```

Nine of the demo's ten documents: the seven with no `status` key, plus the two tagged `queue` or
`overview`.

## Projecting fields with `include`

`knowledge_search` never returns document bodies. To see metadata without one `knowledge_get` per
result, ask for it:

```json
knowledge_search
{"where": "has(last_reviewed)", "include": ["frontmatter.last_reviewed", "frontmatter.oncall"]}
```

```json
{
  "results": [
    {"path": "runbooks/backup.md", "frontmatter": {"last_reviewed": "2026-05-12"}},
    {"path": "runbooks/deploy.md", "frontmatter": {"last_reviewed": "2026-06-30", "oncall": "platform"}}
  ],
  "totalApproximate": 2
}
```

Values come back as raw YAML, uncoerced, and a field a document does not have is simply **absent**
from its map — never a `null` standing in for "missing".

Two traps worth knowing:

- **The `frontmatter.` prefix is mandatory here**, unlike in `where`/`filter` where it is optional.
  In one and the same call, `where: "status = \"x\""` works but `include: ["status"]` is rejected:

  ```text
  INVALID_SCHEMA: cada entrada de «include» debe ser «frontmatter.<fieldPath>» (p. ej.
  «frontmatter.status» o «frontmatter.owner.name»); recibido ["status"]
  ```

- The key of each projected entry is the suffix **exactly as you asked for it**
  (`"frontmatter.owner.name"` comes back under `"owner.name"`), not re-nested.

## When a query fails

Outcomes are deliberately different:

| Situation | Result |
|---|---|
| A frontmatter field no document has | Matches nothing. Not an error. |
| An unknown property under `graph.` / `document.` | `INVALID_SCHEMA`, at compile time, naming the valid properties |
| An ordering comparison against an incompatible type | `INVALID_SCHEMA`, naming the field, the operator, both types and the document where they clashed |
| A list operator (`contains_any`, `contains_all`, or `contains` on a non-string scalar) on a field that is not a list | `INVALID_SCHEMA`, same shape |
| A text operator (`starts_with`, `ends_with`) on a value that is not a string | `INVALID_SCHEMA`, same shape |

The last three are the interesting ones — they are **type errors**, and they abort the query rather
than drop a document. `retention_days` is a number in the demo, so ordering it against a string
aborts instead of quietly dropping the document:

```json
knowledge_search
{"where": "retention_days >= \"30\""}
```

```text
INVALID_SCHEMA: la consulta no es respondible sobre estos datos: en «runbooks/backup.md» el campo
«retention_days» es de tipo number y la consulta lo compara con un literal de tipo string mediante el
operador de orden «greater_than_or_equal». El orden solo está definido entre dos number o entre dos
string (lexicográfico), y el lenguaje no coerce tipos (§20.8). Ajusta la consulta al tipo real del
campo: compara con un literal de ese tipo, o usa un operador definido para él («=»/«!=» nunca son
error — el cruce de tipos es false); metadata_inspect{"mode":"field"} enumera los tipos que ese campo
toma en el workspace
```

`starts_with` and `ends_with` are the same story with a different operator. They are **text**
operators: they need a string on **both** sides — the field *and* the literal — and there is no
coercion, so a number that merely looks like text is still a number:

```json
knowledge_search
{"where": "retention_days starts_with \"3\""}
```

```text
INVALID_SCHEMA: la consulta no es respondible sobre estos datos: en «runbooks/backup.md» la
comparación sobre el campo «retention_days» tiene un operando de tipo number y el operador de texto
«starts_with» exige un string a los DOS lados (el campo y el literal): lo que no es texto no tiene
prefijo ni sufijo que comprobar, y el lenguaje no coerce tipos (§20.8). Ajusta la consulta al tipo
real del campo: compara con un literal de ese tipo, o usa un operador definido para él («=»/«!=»
nunca son error — el cruce de tipos es false); metadata_inspect{"mode":"field"} enumera los tipos
que ese campo toma en el workspace
```

A list is not a string either. Asking for the prefix of `tags` does **not** silently test its first
element — that would be exactly the coercion the language refuses:

```json
knowledge_search
{"where": "tags starts_with \"arch\""}
```

```text
INVALID_SCHEMA: la consulta no es respondible sobre estos datos: en
«adr/0001-markdown-source-of-truth.md» la comparación sobre el campo «tags» tiene un operando de
tipo list y el operador de texto «starts_with» exige un string a los DOS lados (el campo y el
literal): lo que no es texto no tiene prefijo ni sufijo que comprobar, y el lenguaje no coerce tipos
(§20.8). …
```

Until v0.5.0 both of those returned `false` per document, so the answer was a **trimmed** result
list — the documents whose field happened to be text — with nothing to tell you the rest had been
dropped. Not matching is still not an error: `status starts_with "zzz"` over a string field is an
empty result list, and a field no document has stays an empty result list too.

The error is decided over the whole workspace and **before** `text`, `limit` and `cursor` are
applied, so it is deterministic: a narrower search or a smaller page will not hide it. The way out
is the one the message names — compare against the real type, or narrow the set with `has(...)`.
When you do not know the real type, `metadata_inspect` tells you:

```json
metadata_inspect
{"mode": "field", "field": "retention_days"}
```

```json
{"field": "retention_days", "presentIn": 1, "missingIn": 9, "inferredTypes": {"number": 1},
 "values": [{"value": 30, "count": 1}]}
```

## Declared limits

These are the places where the language cannot express what you might reasonably want. They are
listed here because a limit you know about is a limit you can work around.

### Dates are strings, and compare lexicographically

**There is no date type.** An unquoted `2026-07-23` in frontmatter is a *string* — the YAML library
in use does not type timestamps — and ordering strings is lexicographic. For ISO-8601 dates that are
well-formed and all the same length, lexicographic order agrees with chronological order, so the
common case works:

```json
knowledge_search
{"where": "last_reviewed > \"2026-06-01\"", "include": ["frontmatter.last_reviewed"]}
```

```json
{
  "results": [{"path": "runbooks/deploy.md", "frontmatter": {"last_reviewed": "2026-06-30"}}],
  "totalApproximate": 1
}
```

It stops agreeing as soon as the formats are mixed. The demo's two ADRs are dated `2025-11-04` and
`2026-01-09`; both are chronologically after September 2025, but only one comes back:

```json
knowledge_search
{"where": "date > \"2025-9-1\"", "include": ["frontmatter.date"]}
```

```json
{
  "results": [{"path": "adr/0002-event-bus.md", "frontmatter": {"date": "2026-01-09"}}],
  "totalApproximate": 1
}
```

`"2025-11-04"` sorts *before* `"2025-9-1"` because at the first character where the two differ,
`1` < `9`. The same
applies to mixed timezone offsets and to comparing a date against a datetime. What to do about it:

- keep one date format across the workspace, zero-padded (`2025-09-01`, not `2025-9-1`);
- quote dates in frontmatter if you want to be explicit that they are text;
- remember there is no `now()`, no relative dates, and no file timestamp (`mtime`/`ctime`) in
  `document.*` — the language only sees what is written in the documents.

### Three things the dot-path dialect cannot say

Field paths always split on `.` and the language has no quoting, which leaves three cases
unexpressible. The examples in this section run against a four-document scratch workspace built to
show them — not the demo. Its frontmatter, in full:

```console
$ head -3 service.md odd.md flat.md nested.md
==> service.md <==
---
sonar.projectKey: atlas-api
---

==> odd.md <==
---
frontmatter:
  x: 1

==> flat.md <==
---
env.region: eu-west
---

==> nested.md <==
---
env:
  region: us-east
```

**1. A key that contains a literal dot is not addressable.** `metadata_inspect` announces
`sonar.projectKey`, but any query for it descends into a nested `sonar` map that does not exist:

```json
knowledge_search
{"where": "sonar.projectKey = \"atlas-api\""}
```

```json
{"results": [], "totalApproximate": 0}
```

**2. A key literally named `frontmatter` is shadowed by the anchor.** `frontmatter.x` reads the
prefix as the anchor and looks for a top-level `x`:

```json
knowledge_search
{"where": "frontmatter.x = 1"}
```

```json
{"results": [], "totalApproximate": 0}
```

Its *value* is still readable, through the one surface whose suffix is literal:

```json
knowledge_search
{"include": ["frontmatter.frontmatter.x"]}
```

```json
{
  "results": [
    {"path": "flat.md",    "frontmatter": {}},
    {"path": "nested.md",  "frontmatter": {}},
    {"path": "odd.md",     "frontmatter": {"frontmatter.x": 1}},
    {"path": "service.md", "frontmatter": {}}
  ],
  "totalApproximate": 4
}
```

And `metadata_inspect` refuses the collision loudly rather than reporting a silent zero:

```json
metadata_inspect
{"mode": "field", "field": "frontmatter.x"}
```

```text
INVALID_SCHEMA: «frontmatter.x» no es inspeccionable: el prefijo «frontmatter.» es el ANCLAJE del
lenguaje de consulta (E24-H08), así que este texto se lee como una clave del frontmatter que no
aparece en ningún documento; el nombre que anuncia el catálogo viene de una clave de primer nivel
llamada literalmente «frontmatter», y el lenguaje no tiene comillas para distinguir las dos cosas. Su
VALOR sí se puede leer, con knowledge_search{include: ["frontmatter.frontmatter.x"]}, cuyo prefijo es
obligatorio y cuyo sufijo es literal; para inspeccionarla como campo habría que renombrar esa clave
```

**3. A literal `a.b` key and a nested `a: {b: …}` look identical in the catalog.** Both are
announced as `env.region` — two rows, one name, nothing to tell them apart — and only the nested one
is reachable from a query:

```json
metadata_inspect
{"mode": "catalog"}
```

```json
{
  "fields": [
    {"name": "env",              "presentIn": 1, "inferredTypes": {"mapping": 1}},
    {"name": "env.region",       "presentIn": 1, "inferredTypes": {"string": 1}},
    {"name": "env.region",       "presentIn": 1, "inferredTypes": {"string": 1}},
    {"name": "frontmatter",      "presentIn": 1, "inferredTypes": {"mapping": 1}},
    {"name": "frontmatter.x",    "presentIn": 1, "inferredTypes": {"number": 1}},
    {"name": "sonar.projectKey", "presentIn": 1, "inferredTypes": {"string": 1}}
  ],
  "nextCursor": null
}
```

```json
knowledge_search
{"where": "env.region contains \"e\"", "include": ["frontmatter.env.region"]}
```

```json
{"results": [{"path": "nested.md", "frontmatter": {"env.region": "us-east"}}], "totalApproximate": 1}
```

All three are avoided the same way: **do not put dots in frontmatter keys**, and do not name a
top-level key `frontmatter`. Nested maps are the intended way to group.

## Reference

The authority on parameters, defaults, limits and return shapes is
[`contracts/mcp.yml`](../../contracts/mcp.yml) — the entries for `knowledge_search`,
`metadata_inspect` and `change_plan`. It is written in Spanish, like the rest of the internal
material. For the tools that consume these queries, see
[mcp-clients.md](mcp-clients.md#a-tour-of-the-ten-tools); for using a query to drive a change, see
[safe-changes.md](safe-changes.md).
