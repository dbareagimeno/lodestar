use lodestar_core::model;
use lodestar_core::types::FrontmatterPatch;
use serde_yaml::Value as Yaml;

fn yaml(source: &str) -> Yaml {
    serde_yaml::from_str(source).expect("YAML de fixture válido")
}

fn patch(source: serde_json::Value) -> FrontmatterPatch {
    serde_json::from_value(source).expect("patch JSON interpretable como FrontmatterPatch")
}

fn frontmatter(raw: &str) -> Yaml {
    model::parse_file("doc.md", raw)
        .frontmatter
        .expect("la fixture debe tener frontmatter interpretable")
        .value
}

fn document(frontmatter: &Yaml, body: &str) -> String {
    format!(
        "---\n{}\n---\n\n{body}",
        serde_yaml::to_string(frontmatter)
            .expect("serializar fixture YAML")
            .trim_end()
    )
}

#[test]
fn patch_frontmatter_merge_recursivo_conserva_hermanas_y_cuerpo() {
    let raw = "---\nkeep: top-level\nnested:\n  a: old\n  b: survives\n  hondo:\n    keep: survives\n    a: old-deep\nexternal:\n  untouched: true\n---\n\n# Cuerpo original\n\ntexto fuera del frontmatter\n";
    let patched = model::patch_frontmatter(
        raw,
        &patch(serde_json::json!({
            "nested": {
                "a": "new",
                "hondo": {"a": "new-deep"}
            }
        })),
    )
    .expect("el patch RFC 7386 debe ser aplicable");

    assert_eq!(
        frontmatter(&patched.raw),
        yaml("keep: top-level\nnested:\n  a: new\n  b: survives\n  hondo:\n    keep: survives\n    a: new-deep\nexternal:\n  untouched: true\n"),
        "las claves ausentes deben sobrevivir a toda profundidad"
    );
    assert!(
        patched
            .raw
            .ends_with("# Cuerpo original\n\ntexto fuera del frontmatter\n"),
        "el cuerpo no pertenece al merge-patch y debe conservarse byte a byte: {:?}",
        patched.raw
    );
}

#[test]
fn patch_frontmatter_null_anidado_borra_solo_la_clave_nombrada() {
    let raw = "---\nnested:\n  a: survives\n  delete_me: remove\n  hondo:\n    keep: survives-too\nother: untouched\n---\n\nbody\n";
    let patched = model::patch_frontmatter(
        raw,
        &patch(serde_json::json!({
            "nested": {"delete_me": null}
        })),
    )
    .expect("null anidado es una operación de borrado válida");

    assert_eq!(
        frontmatter(&patched.raw),
        yaml("nested:\n  a: survives\n  hondo:\n    keep: survives-too\nother: untouched\n")
    );
    assert!(!patched.raw.contains("delete_me"));
    assert!(patched.raw.ends_with("---\n\nbody\n"));
}

#[test]
fn patch_frontmatter_null_raiz_borra_solo_linea_y_preserva_hermana_flow_y_cuerpo() {
    let raw = "---\nkeep: [uno, dos] # comentario de hermana\nremove_me: scalar\nother: untouched\n---\n\nbody con --- en el texto\n";
    assert!(raw.contains("keep: [uno, dos] # comentario de hermana\n"));
    assert_eq!(raw.matches("remove_me: scalar\n").count(), 1);
    assert!(raw.ends_with("body con --- en el texto\n"));

    let patched = model::patch_frontmatter(raw, &patch(serde_json::json!({"remove_me": null})))
        .expect("null raíz debe eliminar una clave escalar existente");

    assert_eq!(
        patched.raw,
        "---\nkeep: [uno, dos] # comentario de hermana\nother: untouched\n---\n\nbody con --- en el texto\n",
        "el patch debe borrar solo la línea objetivo y conservar la hermana flow con comentario"
    );
    assert!(!patched.raw.contains("remove_me:"));
    assert!(patched
        .raw
        .contains("keep: [uno, dos] # comentario de hermana\n"));
    assert!(patched.raw.ends_with("body con --- en el texto\n"));
    assert!(
        !patched.reserialized,
        "borrar una clave raíz escalar debe ser una edición quirúrgica"
    );
}

#[test]
fn patch_frontmatter_objeto_sobre_escalar_array_y_null_crea_objeto_desde_vacio() {
    let mut failures = Vec::new();
    for target in [
        ("scalar", "slot: 7\n"),
        ("array", "slot:\n- old\n"),
        ("null", "slot: null\n"),
    ] {
        let raw = format!("---\n{}other: survives\n---\n\nbody\n", target.1);
        let patched = model::patch_frontmatter(
            &raw,
            &patch(serde_json::json!({
                "slot": {"created": true, "not_present": null}
            })),
        )
        .unwrap_or_else(|error| panic!("{}: patch rechazado: {error}", target.0));

        let expected = yaml("slot:\n  created: true\nother: survives\n");
        if frontmatter(&patched.raw) != expected || patched.raw.contains("not_present") {
            failures.push(format!(
                "{}: resultado {:?}",
                target.0,
                frontmatter(&patched.raw)
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "cada target no-objeto debe empezar como objeto vacío; fallos: {failures:?}"
    );
}

#[test]
fn patch_frontmatter_objeto_sobre_escalar_es_quirurgico_y_preserva_hermana_formateada() {
    let raw = "---\nstatus: draft\ntags: [uno, dos] # comentario distintivo\n---\n\nbody\n";
    let patched = model::patch_frontmatter(
        raw,
        &patch(serde_json::json!({
            "status": {
                "owner": "ana",
                "obsolete": null
            }
        })),
    )
    .expect("un objeto RFC 7386 sobre un escalar debe partir de un objeto vacío");

    assert_eq!(
        frontmatter(&patched.raw),
        yaml("status:\n  owner: ana\ntags: [uno, dos]\n"),
        "el escalar reemplazado debe convertirse en el objeto RFC 7386 y null no debe crear una clave"
    );
    assert_eq!(
        patched.raw,
        "---\nstatus:\n  owner: ana\ntags: [uno, dos] # comentario distintivo\n---\n\nbody\n",
        "la hermana no tocada debe conservar su formato y comentario byte a byte"
    );
    assert!(
        !patched.reserialized,
        "un objeto sobre un escalar de una sola línea es edición quirúrgica, no fallback de reserialización"
    );
}

#[test]
fn patch_frontmatter_arrays_y_escalares_son_sustituciones_atomicas() {
    let raw = "---\nsettings:\n  tags: [old-a, old-b]\n  limit: 3\n  keep: survives\nroot_scalar: old\n---\n\nbody\n";
    let patched = model::patch_frontmatter(
        raw,
        &patch(serde_json::json!({
            "settings": {
                "tags": ["new-only"],
                "limit": 4
            },
            "root_scalar": "new"
        })),
    )
    .expect("arrays y escalares son valores YAML válidos");

    assert_eq!(
        frontmatter(&patched.raw),
        yaml("settings:\n  tags: [new-only]\n  limit: 4\n  keep: survives\nroot_scalar: new\n"),
        "el objeto contenedor se fusiona, pero el array se sustituye entero y el escalar cambia"
    );
    assert!(!patched.raw.contains("old-a"));
    assert!(!patched.raw.contains("old-b"));
}

#[derive(Clone, Copy)]
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }
}

fn generated_value(rng: &mut Rng, depth: u8) -> Yaml {
    if depth == 0 {
        return match rng.next() % 4 {
            0 => Yaml::String(format!("s-{}", rng.next() % 1000)),
            1 => Yaml::Number((rng.next() % 100).into()),
            2 => Yaml::Bool(rng.next() % 2 == 0),
            _ => Yaml::Null,
        };
    }

    match rng.next() % 5 {
        0 => Yaml::String(format!("s-{}", rng.next() % 1000)),
        1 => Yaml::Number((rng.next() % 100).into()),
        2 => Yaml::Bool(rng.next() % 2 == 0),
        3 => Yaml::Sequence(vec![
            generated_value(rng, depth - 1),
            generated_value(rng, depth - 1),
        ]),
        _ => {
            let mut object = serde_yaml::Mapping::new();
            object.insert(Yaml::String("left".into()), generated_value(rng, depth - 1));
            object.insert(
                Yaml::String("right".into()),
                generated_value(rng, depth - 1),
            );
            Yaml::Mapping(object)
        }
    }
}

fn generated_object(rng: &mut Rng, depth: u8) -> Yaml {
    let mut object = serde_yaml::Mapping::new();
    object.insert(
        Yaml::String("left".into()),
        generated_value(rng, depth.saturating_sub(1)),
    );
    object.insert(
        Yaml::String("right".into()),
        generated_value(rng, depth.saturating_sub(1)),
    );
    Yaml::Mapping(object)
}

fn generated_target(rng: &mut Rng) -> Yaml {
    let mut target = serde_yaml::Mapping::new();
    target.insert(Yaml::String("stable".into()), generated_value(rng, 2));
    target.insert(Yaml::String("nested".into()), generated_object(rng, 3));
    target.insert(
        Yaml::String("array".into()),
        Yaml::Sequence(vec![generated_value(rng, 1), generated_value(rng, 1)]),
    );
    target.insert(Yaml::String("scalar".into()), generated_value(rng, 0));
    target.insert(Yaml::String("nullable".into()), Yaml::Null);
    Yaml::Mapping(target)
}

fn generated_patch(rng: &mut Rng) -> Yaml {
    let mut nested = serde_yaml::Mapping::new();
    nested.insert(Yaml::String("left".into()), generated_value(rng, 2));
    nested.insert(Yaml::String("right".into()), Yaml::Null);
    nested.insert(Yaml::String("new_deep".into()), generated_object(rng, 2));

    let mut absent = serde_yaml::Mapping::new();
    absent.insert(Yaml::String("created".into()), generated_value(rng, 1));
    absent.insert(Yaml::String("not_there".into()), Yaml::Null);

    let mut patch = serde_yaml::Mapping::new();
    patch.insert(Yaml::String("nested".into()), Yaml::Mapping(nested));
    patch.insert(
        Yaml::String("array".into()),
        Yaml::Sequence(vec![generated_value(rng, 1)]),
    );
    patch.insert(Yaml::String("scalar".into()), generated_value(rng, 1));
    patch.insert(Yaml::String("nullable".into()), generated_object(rng, 1));
    patch.insert(Yaml::String("absent".into()), Yaml::Mapping(absent));
    Yaml::Mapping(patch)
}

/// Oráculo independiente de RFC 7386: no llama a `lodestar_core` ni usa su representación de
/// `FrontmatterPatch`; opera solamente sobre `serde_yaml::Value` y la regla del RFC.
fn rfc7386(target: Yaml, patch: &Yaml) -> Yaml {
    let Yaml::Mapping(patch_map) = patch else {
        return patch.clone();
    };
    let mut result = match target {
        Yaml::Mapping(map) => map,
        _ => serde_yaml::Mapping::new(),
    };
    for (key, patch_value) in patch_map {
        if *patch_value == Yaml::Null {
            result.remove(key);
            continue;
        }
        let previous = result.remove(key).unwrap_or(Yaml::Null);
        result.insert(key.clone(), rfc7386(previous, patch_value));
    }
    Yaml::Mapping(result)
}

#[test]
fn property_patch_frontmatter_contra_oraculo_independiente_rfc7386() {
    let mut rng = Rng(0x46_7386);
    let mut cases = Vec::new();
    let mut saw_array = false;
    let mut saw_scalar = false;
    let mut saw_null = false;
    for _ in 0..64 {
        let target = generated_target(&mut rng);
        let patch = generated_patch(&mut rng);
        saw_array |= matches!(target, Yaml::Mapping(ref map) if matches!(map.get(Yaml::String("array".into())), Some(Yaml::Sequence(_))));
        saw_scalar |= matches!(patch, Yaml::Mapping(ref map) if !matches!(map.get(Yaml::String("scalar".into())), Some(Yaml::Mapping(_))));
        saw_null |= matches!(patch, Yaml::Mapping(ref map) if matches!(map.get(Yaml::String("nested".into())), Some(Yaml::Mapping(nested)) if nested.values().any(|value| *value == Yaml::Null)));
        cases.push((target, patch));
    }
    assert!(
        saw_array && saw_scalar && saw_null,
        "el generador debe cubrir arrays, escalares y null"
    );

    for (case, (target, patch_value)) in cases.into_iter().enumerate() {
        let raw = document(&target, &format!("# body case {case}\n"));
        let expected = rfc7386(target, &patch_value);
        let patch_json =
            serde_json::to_value(&patch_value).expect("patch YAML compatible con JSON");
        let actual = model::patch_frontmatter(&raw, &patch(patch_json))
            .unwrap_or_else(|error| panic!("caso {case}: patch rechazado: {error}"));
        assert_eq!(
            frontmatter(&actual.raw),
            expected,
            "caso {case}: semántica distinta a RFC 7386; patch={patch_value:?}"
        );
        assert_eq!(
            model::parse_file("doc.md", &actual.raw).body,
            format!("\n# body case {case}\n")
        );
    }
}
