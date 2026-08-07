//! Rechazo de los argumentos que la superficie MCP **no declara** (E29-H08, `decisiones §15`).
//!
//! Los 10 `inputSchema` declaran `additionalProperties: false` desde siempre, pero el despachador
//! leía campo a campo con `params.get("…")` y nunca miraba las claves sobrantes: un `sort` retirado,
//! un `offset` inexistente o un typo como `wheres` se descartaban en silencio y el agente recibía la
//! respuesta **por defecto**, indistinguible de una legítima. `decisiones §15` decidió **ejecutar**
//! lo que el schema declara; este módulo es esa ejecución.
//!
//! # Una sola fuente
//!
//! Las claves legales no se escriben aquí a mano: se **derivan del propio `inputSchema`** que
//! [`crate::tools::list`] construye, recorriéndolo como JSON. Así el schema publicado y la lista
//! ejecutada no pueden divergir — que es la guarda anti-envejecimiento que pide la historia.
//!
//! # Dos niveles, con criterio distinto
//!
//! 1. **Nivel tool** (`arguments` de `tools/call`): partición **limpia**. Una clave que el
//!    `inputSchema` no declara → `INVALID_SCHEMA` nombrándola. Lo mismo dentro de los objetos
//!    anidados que declaran `additionalProperties: false` (`ref`, `to`, `proposedOperation`).
//! 2. **Nivel operación** (`operations[]` de `change_plan` y el `operation` de la selección masiva):
//!    validación por **UNIÓN**, no por partición. Se rechaza lo que no esté en la unión de los
//!    campos legales de las 7 ops; **no** se rechaza un campo legal para *otra* op. Un `body` en un
//!    `patch_frontmatter` se sigue ignorando; un `bodyy` se rechaza. Razón (`§15`): `path`/`ref` son
//!    intercambiables salvo en `create` y `body` pertenece a **dos** ops, así que una partición
//!    estricta rompería un lote válido en el que el agente reutiliza la misma plantilla de objeto.
//!    Cerrar la partición por op es **decisión posterior**, declarada, no un olvido.

use lodestar_core::types::ErrorCode;
use serde_json::Value;

/// Valida los `arguments` de `tools/call` contra el `inputSchema` de `tool`.
///
/// Recorre el nivel raíz y los objetos anidados que el schema declara cerrados, y devuelve
/// `Err(INVALID_SCHEMA: …)` nombrando la **primera** clave desconocida. Las tools sin `inputSchema`
/// en el catálogo (imposible hoy: el test `tools_list_lleva_input_schema` lo impide) no validan
/// nada.
pub fn valida_argumentos(tool: &str, args: &Value) -> Result<(), String> {
    let Some(schema) = schema_de(tool) else {
        return Ok(());
    };
    valida_objeto(args, &schema, tool)?;
    valida_operaciones_de_change_plan(tool, args)
}

/// Valida un objeto contra un (sub)schema: sus claves contra las `properties` declaradas y, en
/// cascada, los valores de las claves cuyo schema sea a su vez un objeto con `properties`.
///
/// `contexto` nombra dónde estamos (`«knowledge_get»`, `«knowledge_get.ref»`) para que el mensaje
/// diga qué corregir y **dónde**.
fn valida_objeto(valor: &Value, schema: &Value, contexto: &str) -> Result<(), String> {
    let Some(obj) = valor.as_object() else {
        // Un no-objeto no tiene claves que juzgar, así que este módulo no dice nada de él. Lo que
        // pase después NO es uniforme y conviene no fingir que lo es: donde el despachador
        // deserializa el parámetro (`ref`/`scope`/`policy`…) sale un error de FORMA con su propio
        // mensaje, pero las tools de lectura que leen con `params.get(…).and_then(…)` degradan a su
        // valor por defecto. Eso es PREEXISTENTE a E29-H08 —que solo juzga claves desconocidas, no
        // valores— y queda fuera de sus criterios; se anota aquí para que no se lea como resuelto.
        return Ok(());
    };
    let declaradas = schema["properties"].as_object();
    for clave in obj.keys() {
        let declarada = declaradas.is_some_and(|p| p.contains_key(clave));
        if !declarada {
            return Err(desconocida(clave, contexto, declaradas));
        }
    }
    // Cascada a los objetos anidados que el schema describe con sus propias `properties`.
    let Some(props) = declaradas else {
        return Ok(());
    };
    for (clave, valor_hijo) in obj {
        let sub = &props[clave];
        if sub["type"] == "object" && sub["properties"].is_object() {
            valida_objeto(valor_hijo, sub, &format!("{contexto}.{clave}"))?;
        }
    }
    Ok(())
}

/// El mensaje de rechazo: **nombra** la clave desconocida y lista las declaradas, para que el
/// agente sepa cuál quiso escribir. Nombrarla no es cosmética — es la diferencia entre «corrige
/// `wheres`» y «algo de tu llamada está mal», que es justo el silencio que E29-H08 cierra.
fn desconocida(
    clave: &str,
    contexto: &str,
    declaradas: Option<&serde_json::Map<String, Value>>,
) -> String {
    let legales: Vec<&str> = declaradas
        .map(|p| p.keys().map(String::as_str).collect())
        .unwrap_or_default();
    let cola = if legales.is_empty() {
        format!("«{contexto}» no declara ningún parámetro")
    } else {
        format!("«{contexto}» declara {legales:?}")
    };
    format!(
        "{}: «{clave}» no es un parámetro declarado; {cola}",
        ErrorCode::InvalidSchema.as_str()
    )
}

/// Nivel operación de `change_plan`: valida cada elemento de `operations[]` contra la **unión** de
/// los campos legales de las 7 ops, y el `operation` de la selección masiva contra los parámetros
/// sueltos de la op que codifica.
///
/// Solo aplica a `change_plan`; las demás tools no tienen nivel operación.
fn valida_operaciones_de_change_plan(tool: &str, args: &Value) -> Result<(), String> {
    if tool != "change_plan" {
        return Ok(());
    }
    // (1) La forma de ops sueltas. Se valida **elemento a elemento**, nunca el objeto de argumentos
    //     entero: con `selection`, `tools.rs` pasa `params.clone()` como `raw_ops`, así que el
    //     objeto de la tool y el de la selección son EL MISMO — validar el nivel operación sobre él
    //     vería `selection`/`operation`/`policy` como campos de op desconocidos y rompería la forma
    //     masiva completa.
    if let Some(ops) = args.get("operations").and_then(Value::as_array) {
        let union = union_de_campos_de_operacion();
        for (i, op) in ops.iter().enumerate() {
            let Some(obj) = op.as_object() else { continue };
            for clave in obj.keys() {
                if !union.contains(&clave.as_str()) {
                    return Err(format!(
                        "{}: «{clave}» no es un campo de operación declarado (operations[{i}]); \
                         los campos legales son {union:?}. La validación es por UNIÓN de las 7 \
                         operaciones: un campo legal de OTRA op se ignora, pero uno que no existe \
                         en ninguna se rechaza.",
                        ErrorCode::InvalidSchema.as_str()
                    ));
                }
            }
            // Y la cascada a los sub-objetos que el `items` declara con sus propias `properties`
            // (`ref`, y `to` cuando viaja como objeto). Sin esto había una asimetría contra el
            // principio de la épica: el mismo `ref: {path, parametroQueNoExiste}` se rechazaba por
            // `knowledge_get` y pasaba en silencio dentro de una operación. `patch`/`frontmatter`
            // quedan FUERA por construcción: el schema los declara `type: object` SIN `properties`
            // —son claves arbitrarias del usuario (`§20.2` invariante 3)—, así que `valida_objeto`
            // no desciende a ellos.
            //
            // Se desciende a los HIJOS directamente, sin pasar el elemento entero por
            // `valida_objeto`: esa función valida el nivel que recibe por PARTICIÓN, y el nivel
            // operación es por UNIÓN (ya juzgado arriba, con su propio mensaje). Aquí solo se
            // reutiliza su cascada.
            let item = item_schema_de_operacion();
            if let Some(props) = item["properties"].as_object() {
                for (clave, hijo) in obj {
                    let sub = &props[clave];
                    if sub["type"] == "object" && sub["properties"].is_object() {
                        valida_objeto(hijo, sub, &format!("operations[{i}].{clave}"))?;
                    }
                }
            }
        }
    }
    // (2) La forma de selección masiva. `operation` es un objeto de UNA clave (el tipo de op) cuyo
    //     valor lleva los parámetros SUELTOS de esa op — salvo `patch_frontmatter`, cuyo valor ES
    //     el merge-patch de frontmatter, o sea claves ARBITRARIAS del usuario que este módulo no
    //     puede juzgar sin inventarse un vocabulario que `§20.2` prohíbe imponer.
    if let Some(operation) = args.get("operation").and_then(Value::as_object) {
        let union = union_de_campos_de_operacion();
        for (kind, params) in operation {
            if kind == "patch_frontmatter" {
                continue;
            }
            let Some(sueltos) = params.as_object() else {
                continue;
            };
            for clave in sueltos.keys() {
                if !union.contains(&clave.as_str()) {
                    return Err(format!(
                        "{}: «{clave}» no es un campo de operación declarado (operation.{kind}); \
                         los campos legales son {union:?}.",
                        ErrorCode::InvalidSchema.as_str()
                    ));
                }
            }
        }
    }
    Ok(())
}

/// El schema de UN elemento de `operations[]`, tal cual lo publica el `inputSchema` de
/// `change_plan` (o sea, `tools::operacion_item_schema`). Misma fuente única que
/// [`union_de_campos_de_operacion`]: de aquí salen tanto los campos legales como los sub-objetos a
/// los que hay que descender.
fn item_schema_de_operacion() -> Value {
    schema_de("change_plan")
        .map(|s| s["properties"]["operations"]["items"].clone())
        .unwrap_or(Value::Null)
}

/// La **unión** de los campos legales de las 7 operaciones, derivada de las `properties` que
/// declara el `items` de `operations` en el `inputSchema` de `change_plan` (o sea, de
/// `tools::operacion_item_schema`): una sola fuente, consultada por el rechazo y publicada por el
/// schema.
fn union_de_campos_de_operacion() -> Vec<&'static str> {
    // El schema es un `json!` construido con literales `'static`, pero `serde_json` los entrega como
    // `String` prestadas del `Value` temporal; se materializa una vez y se filtra a `&'static str`
    // con la lista que el propio schema publica.
    static UNION: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    UNION
        .get_or_init(|| {
            schema_de("change_plan")
                .and_then(|s| {
                    s["properties"]["operations"]["items"]["properties"]
                        .as_object()
                        .cloned()
                })
                .map(|p| p.keys().cloned().collect())
                .unwrap_or_default()
        })
        .iter()
        .map(String::as_str)
        .collect()
}

/// El `inputSchema` publicado para `tool`, tal cual lo sirve `tools/list`.
fn schema_de(tool: &str) -> Option<Value> {
    crate::tools::list()
        .as_array()?
        .iter()
        .find(|t| t["name"] == tool)
        .map(|t| t["inputSchema"].clone())
}
