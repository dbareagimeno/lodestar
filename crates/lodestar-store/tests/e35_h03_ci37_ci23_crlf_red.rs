//! E35-H03 CI38 — todos los consumidores estructurales deben localizar anchors en CRLF.
//!
//! Los tests estructurales incluyen los fuentes de producción y después delimitan secciones con
//! anchors que contienen LF. Este test autentica esos anchors contra cada consumidor real,
//! inventaría todos los consumidores E35-H03 del crate y exige que ninguna referencia cruda pueda
//! alcanzar `section`, `.contains`, `.find`, un alias u otro análisis posterior.

use std::collections::{BTreeMap, BTreeSet};

const CI19_SOURCE: &str = include_str!("e35_h03_ci19_repair2_red.rs");
const CI19_REPAIR3_SOURCE: &str = include_str!("e35_h03_ci19_repair3_red.rs");
const CI23_SOURCE: &str = include_str!("e35_h03_ci23_red.rs");
const CI23_RELATIVE_ROOT_SOURCE: &str = include_str!("e35_h03_ci23_relative_root_red.rs");
const CI23_REVIEW_SOURCE: &str = include_str!("e35_h03_ci23_review_repair_red.rs");
const CI26_SOURCE: &str = include_str!("e35_h03_ci26_placeholder_red.rs");
const CI27_SOURCE: &str = include_str!("e35_h03_ci27_identity_red.rs");
const CI33_SOURCE: &str = include_str!("e35_h03_ci33_windows_rename_red.rs");
const FINAL_RSS_SOURCE: &str = include_str!("e35_h03_final_review_rss_red.rs");
const REPAIR9_SOURCE: &str = include_str!("e35_h03_repair9_red.rs");

struct IncludeExpectation<'a> {
    binding: &'a str,
    path: &'a str,
    references: usize,
}

struct SuiteExpectation<'a> {
    file: &'a str,
    sensitive: bool,
    includes: &'a [IncludeExpectation<'a>],
    anchor: Option<(&'a str, usize)>,
}

const SUITE_EXPECTATIONS: &[SuiteExpectation<'static>] = &[
    SuiteExpectation {
        file: "e35_h03_ci19_repair2_red.rs",
        sensitive: true,
        includes: &[
            IncludeExpectation {
                binding: "STORE_SOURCE",
                path: "lib.rs",
                references: 2,
            },
            IncludeExpectation {
                binding: "WINDOWS_VFS_SOURCE",
                path: "windows_vfs.rs",
                references: 2,
            },
        ],
        anchor: Some((
            "fn rename_handle_to(\n    target: &Path,\n    handle: HANDLE,\n    extended_flags: Option<u32>,\n)",
            3,
        )),
    },
    SuiteExpectation {
        file: "e35_h03_ci19_repair3_red.rs",
        sensitive: false,
        includes: &[
            IncludeExpectation {
                binding: "STORE_SOURCE",
                path: "lib.rs",
                references: 3,
            },
            IncludeExpectation {
                binding: "SCHEMA_SOURCE",
                path: "schema.rs",
                references: 2,
            },
        ],
        anchor: None,
    },
    SuiteExpectation {
        file: "e35_h03_ci23_red.rs",
        sensitive: false,
        includes: &[IncludeExpectation {
            binding: "WINDOWS_VFS_SOURCE",
            path: "windows_vfs.rs",
            references: 2,
        }],
        anchor: None,
    },
    SuiteExpectation {
        file: "e35_h03_ci23_relative_root_red.rs",
        sensitive: false,
        includes: &[IncludeExpectation {
            binding: "WINDOWS_VFS_SOURCE",
            path: "windows_vfs.rs",
            references: 8,
        }],
        anchor: None,
    },
    SuiteExpectation {
        file: "e35_h03_ci23_review_repair_red.rs",
        sensitive: true,
        includes: &[
            IncludeExpectation {
                binding: "STORE_SOURCE",
                path: "lib.rs",
                references: 2,
            },
            IncludeExpectation {
                binding: "SCHEMA_SOURCE",
                path: "schema.rs",
                references: 2,
            },
            IncludeExpectation {
                binding: "WINDOWS_VFS_SOURCE",
                path: "windows_vfs.rs",
                references: 2,
            },
        ],
        anchor: Some(("ReOpenFile(\n            original,", 4)),
    },
    SuiteExpectation {
        file: "e35_h03_ci26_placeholder_red.rs",
        sensitive: true,
        includes: &[IncludeExpectation {
            binding: "STORE_SOURCE",
            path: "lib.rs",
            references: 2,
        }],
        anchor: Some((
            "\n    #[cfg(windows)]\n    fn verify_published_document_count(",
            1,
        )),
    },
    SuiteExpectation {
        file: "e35_h03_ci27_identity_red.rs",
        sensitive: true,
        includes: &[
            IncludeExpectation {
                binding: "STORE_SOURCE",
                path: "lib.rs",
                references: 2,
            },
            IncludeExpectation {
                binding: "WINDOWS_VFS_SOURCE",
                path: "windows_vfs.rs",
                references: 2,
            },
        ],
        anchor: Some((
            "\n    #[cfg(windows)]\n    fn verify_published_document_count(",
            2,
        )),
    },
    SuiteExpectation {
        file: "e35_h03_ci33_windows_rename_red.rs",
        sensitive: false,
        includes: &[IncludeExpectation {
            binding: "WINDOWS_VFS_SOURCE",
            path: "windows_vfs.rs",
            references: 5,
        }],
        anchor: None,
    },
    SuiteExpectation {
        file: "e35_h03_final_review_rss_red.rs",
        sensitive: false,
        includes: &[IncludeExpectation {
            binding: "STORE_SOURCE",
            path: "lib.rs",
            references: 4,
        }],
        anchor: None,
    },
    SuiteExpectation {
        file: "e35_h03_repair9_red.rs",
        sensitive: false,
        includes: &[IncludeExpectation {
            binding: "STORE_SOURCE",
            path: "lib.rs",
            references: 2,
        }],
        anchor: None,
    },
];

struct RawSource<'a> {
    constant: &'a str,
}

struct Consumer<'a> {
    file: &'a str,
    source: &'a str,
    raw_sources: &'a [RawSource<'a>],
}

struct SectionConsumer<'a> {
    name: &'a str,
    source: &'a str,
    expected_sections: usize,
    allowed_inputs: &'a [&'a str],
}

fn identifier_occurrences(source: &str, identifier: &str) -> usize {
    source
        .match_indices(identifier)
        .filter(|(at, _)| {
            let before = source[..*at].chars().next_back();
            let after = source[*at + identifier.len()..].chars().next();
            let is_identifier =
                |character: char| character == '_' || character.is_ascii_alphanumeric();
            !before.is_some_and(is_identifier) && !after.is_some_and(is_identifier)
        })
        .count()
}

fn raw_source_contract(consumer: &Consumer<'_>) -> Vec<String> {
    let mut errors = Vec::new();
    for raw in consumer.raw_sources {
        let declaration = format!("const {}: &str = include_str!(\"../src/", raw.constant);
        let declarations = consumer.source.matches(&declaration).count();
        if declarations != 1 {
            errors.push(format!(
                "{}: inventario de {} ambiguo; declaraciones={declarations}",
                consumer.file, raw.constant
            ));
        }

        let normalization = format!("{}.replace(\"\\r\\n\", \"\\n\")", raw.constant);
        let normalizations = consumer.source.matches(&normalization).count();
        if normalizations != 1 {
            errors.push(format!(
                "{}: {} debe normalizar CRLF→LF exactamente una vez; normalizaciones={normalizations}",
                consumer.file, raw.constant
            ));
        }

        let occurrences = identifier_occurrences(consumer.source, raw.constant);
        let allowed = declarations + normalizations;
        if occurrences != allowed || allowed != 2 {
            errors.push(format!(
                "{}: {} escapa crudo hacia un alias, section/find/contains u otro análisis; referencias={occurrences}, permitidas=2 (declaración+normalización)",
                consumer.file, raw.constant
            ));
        }
    }
    errors
}

fn section_inputs(source: &str) -> Vec<&str> {
    source
        .split("section(")
        .skip(1)
        .map(|after_call| {
            after_call
                .split(',')
                .next()
                .expect("anti-vacuidad: cada section debe tener primer argumento")
                .trim()
        })
        .collect()
}

fn section_contract(consumer: &SectionConsumer<'_>) -> Vec<String> {
    let mut errors = Vec::new();
    let inputs = section_inputs(consumer.source);
    if inputs.len() != consumer.expected_sections {
        errors.push(format!(
            "{}: inventario section desactualizado; esperadas={}, observadas={} ({inputs:?})",
            consumer.name,
            consumer.expected_sections,
            inputs.len()
        ));
    }
    for (ordinal, input) in inputs.iter().enumerate() {
        if !consumer.allowed_inputs.contains(input) {
            errors.push(format!(
                "{}: section #{} recibe `{input}`; solo se admiten fuentes normalizadas exactas {:?}",
                consumer.name,
                ordinal + 1,
                consumer.allowed_inputs
            ));
        }
    }
    for allowed in consumer.allowed_inputs {
        if !inputs.contains(allowed) {
            errors.push(format!(
                "{}: ninguna entrada section consume la fuente normalizada {allowed}",
                consumer.name
            ));
        }
    }
    errors
}

fn encoded_anchor(decoded: &str) -> String {
    decoded.replace('\n', "\\n")
}

fn real_suite() -> [(&'static str, &'static str, bool); 10] {
    [
        ("e35_h03_ci19_repair2_red.rs", CI19_SOURCE, true),
        ("e35_h03_ci19_repair3_red.rs", CI19_REPAIR3_SOURCE, false),
        ("e35_h03_ci23_red.rs", CI23_SOURCE, false),
        (
            "e35_h03_ci23_relative_root_red.rs",
            CI23_RELATIVE_ROOT_SOURCE,
            false,
        ),
        (
            "e35_h03_ci23_review_repair_red.rs",
            CI23_REVIEW_SOURCE,
            true,
        ),
        ("e35_h03_ci26_placeholder_red.rs", CI26_SOURCE, true),
        ("e35_h03_ci27_identity_red.rs", CI27_SOURCE, true),
        ("e35_h03_ci33_windows_rename_red.rs", CI33_SOURCE, false),
        ("e35_h03_final_review_rss_red.rs", FINAL_RSS_SOURCE, false),
        ("e35_h03_repair9_red.rs", REPAIR9_SOURCE, false),
    ]
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RustTokenKind {
    Identifier(String),
    String(String),
    Punctuation(char),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RustToken {
    kind: RustTokenKind,
    at: usize,
    line: usize,
}

impl RustToken {
    fn identifier(&self) -> Option<&str> {
        match &self.kind {
            RustTokenKind::Identifier(identifier) => Some(identifier),
            RustTokenKind::String(_) | RustTokenKind::Punctuation(_) => None,
        }
    }

    fn string(&self) -> Option<&str> {
        match &self.kind {
            RustTokenKind::String(string) => Some(string),
            RustTokenKind::Identifier(_) | RustTokenKind::Punctuation(_) => None,
        }
    }

    fn punctuation(&self) -> Option<char> {
        match self.kind {
            RustTokenKind::Punctuation(punctuation) => Some(punctuation),
            RustTokenKind::Identifier(_) | RustTokenKind::String(_) => None,
        }
    }
}

fn decode_escape(bytes: &[u8], cursor: &mut usize, decoded: &mut String) {
    if *cursor >= bytes.len() {
        return;
    }
    let escaped = bytes[*cursor];
    *cursor += 1;
    match escaped {
        b'n' => decoded.push('\n'),
        b'r' => decoded.push('\r'),
        b't' => decoded.push('\t'),
        b'0' => decoded.push('\0'),
        b'\\' => decoded.push('\\'),
        b'\'' => decoded.push('\''),
        b'"' => decoded.push('"'),
        b'\n' => {
            while *cursor < bytes.len() && matches!(bytes[*cursor], b' ' | b'\t' | b'\r' | b'\n') {
                *cursor += 1;
            }
        }
        other => decoded.push(char::from(other)),
    }
}

fn normal_string_token(source: &str, start: usize) -> Option<(RustTokenKind, usize)> {
    let bytes = source.as_bytes();
    let mut cursor = start + 1;
    let mut decoded = String::new();
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'"' => return Some((RustTokenKind::String(decoded), cursor + 1)),
            b'\\' => {
                cursor += 1;
                decode_escape(bytes, &mut cursor, &mut decoded);
            }
            byte if byte.is_ascii() => {
                decoded.push(char::from(byte));
                cursor += 1;
            }
            _ => {
                let character = source[cursor..].chars().next()?;
                decoded.push(character);
                cursor += character.len_utf8();
            }
        }
    }
    None
}

fn raw_string_token(source: &str, start: usize) -> Option<(RustTokenKind, usize)> {
    let bytes = source.as_bytes();
    if bytes.get(start) != Some(&b'r') {
        return None;
    }
    let mut quote = start + 1;
    while bytes.get(quote) == Some(&b'#') {
        quote += 1;
    }
    if bytes.get(quote) != Some(&b'"') {
        return None;
    }
    let hashes = quote - start - 1;
    let content_start = quote + 1;
    let mut cursor = content_start;
    while cursor < bytes.len() {
        if bytes[cursor] == b'"'
            && bytes.get(cursor + 1..cursor + 1 + hashes) == Some(&bytes[start + 1..quote])
        {
            let end = cursor + 1 + hashes;
            return Some((
                RustTokenKind::String(source[content_start..cursor].to_owned()),
                end,
            ));
        }
        cursor += 1;
    }
    None
}

fn rust_tokens(source: &str) -> Vec<RustToken> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut cursor = 0;
    let mut line = 1;
    while cursor < bytes.len() {
        if bytes[cursor] == b'\n' {
            line += 1;
            cursor += 1;
            continue;
        }
        if bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
            continue;
        }
        if bytes.get(cursor..cursor + 2) == Some(b"//") {
            cursor += 2;
            while cursor < bytes.len() && bytes[cursor] != b'\n' {
                cursor += 1;
            }
            continue;
        }
        if bytes.get(cursor..cursor + 2) == Some(b"/*") {
            cursor += 2;
            let mut depth = 1_usize;
            while cursor < bytes.len() && depth > 0 {
                if bytes.get(cursor..cursor + 2) == Some(b"/*") {
                    depth += 1;
                    cursor += 2;
                } else if bytes.get(cursor..cursor + 2) == Some(b"*/") {
                    depth -= 1;
                    cursor += 2;
                } else {
                    if bytes[cursor] == b'\n' {
                        line += 1;
                    }
                    cursor += 1;
                }
            }
            continue;
        }
        let token_line = line;
        let token_at = cursor;
        if let Some((kind, end)) = raw_string_token(source, cursor) {
            line += source[cursor..end]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count();
            tokens.push(RustToken {
                kind,
                at: token_at,
                line: token_line,
            });
            cursor = end;
            continue;
        }
        if bytes[cursor] == b'"' {
            if let Some((kind, end)) = normal_string_token(source, cursor) {
                line += source[cursor..end]
                    .bytes()
                    .filter(|byte| *byte == b'\n')
                    .count();
                tokens.push(RustToken {
                    kind,
                    at: token_at,
                    line: token_line,
                });
                cursor = end;
                continue;
            }
        }
        if bytes[cursor] == b'\'' {
            let char_end = if bytes.get(cursor + 1) == Some(&b'\\') {
                cursor + 3
            } else {
                cursor + 2
            };
            if bytes.get(char_end) == Some(&b'\'') {
                cursor = char_end + 1;
                continue;
            }
        }
        if bytes[cursor] == b'_' || bytes[cursor].is_ascii_alphabetic() {
            cursor += 1;
            while cursor < bytes.len()
                && (bytes[cursor] == b'_' || bytes[cursor].is_ascii_alphanumeric())
            {
                cursor += 1;
            }
            tokens.push(RustToken {
                kind: RustTokenKind::Identifier(source[token_at..cursor].to_owned()),
                at: token_at,
                line: token_line,
            });
            continue;
        }
        tokens.push(RustToken {
            kind: RustTokenKind::Punctuation(char::from(bytes[cursor])),
            at: token_at,
            line: token_line,
        });
        cursor += 1;
    }
    tokens
}

fn production_relative_path(path: &str) -> Option<String> {
    let normalized = path.replace('\\', "/");
    let mut components = normalized.split('/').filter(|component| *component != ".");
    (components.next() == Some("..") && components.next() == Some("src"))
        .then(|| components.collect::<Vec<_>>().join("/"))
        .filter(|relative| !relative.is_empty())
}

fn declaration_binding(tokens: &[RustToken], include_at: usize) -> String {
    let statement_start = tokens[..include_at]
        .iter()
        .rposition(|token| matches!(token.punctuation(), Some(';' | '{' | '}')))
        .map_or(0, |at| at + 1);
    let statement = &tokens[statement_start..include_at];
    let Some(declaration_at) = statement
        .iter()
        .position(|token| matches!(token.identifier(), Some("const" | "let" | "static")))
    else {
        return "<expresión-sin-binding>".to_owned();
    };
    let mut binding_at = declaration_at + 1;
    if statement.get(binding_at).and_then(RustToken::identifier) == Some("mut") {
        binding_at += 1;
    }
    let Some(binding) = statement.get(binding_at).and_then(RustToken::identifier) else {
        return "<expresión-sin-binding>".to_owned();
    };
    binding.to_owned()
}

fn production_includes(source: &str) -> Vec<String> {
    let tokens = rust_tokens(source);
    let mut includes = Vec::new();
    for (at, token) in tokens.iter().enumerate() {
        if token.identifier() != Some("include_str")
            || tokens.get(at + 1).and_then(RustToken::punctuation) != Some('!')
            || tokens.get(at + 2).and_then(RustToken::punctuation) != Some('(')
        {
            continue;
        }
        let Some(path) = tokens.get(at + 3).and_then(RustToken::string) else {
            continue;
        };
        let Some(relative) = production_relative_path(path) else {
            continue;
        };
        let binding = declaration_binding(&tokens, at);
        includes.push(format!("{binding}=>{relative}@{}", token.line));
    }
    includes
}

fn expected_production_includes(expectation: &SuiteExpectation<'_>) -> Vec<String> {
    expectation
        .includes
        .iter()
        .map(|include| format!("{}=>{}", include.binding, include.path))
        .collect()
}

fn include_identity(include: &str) -> &str {
    include
        .split_once('@')
        .map_or(include, |(identity, _)| identity)
}

fn has_internal_lf_anchor(literal: &str) -> bool {
    let mut positions: Vec<_> = literal.match_indices('\n').map(|(at, _)| at).collect();
    if positions.is_empty() {
        positions = literal.match_indices("\\n").map(|(at, _)| at).collect();
    }
    positions
        .first()
        .is_some_and(|first| *first > 0 || positions.len() > 1)
}

fn string_bindings(tokens: &[RustToken]) -> BTreeMap<String, String> {
    let mut unresolved = Vec::new();
    for (at, token) in tokens.iter().enumerate() {
        if !matches!(token.identifier(), Some("const" | "let" | "static")) {
            continue;
        }
        let mut name_at = at + 1;
        if tokens.get(name_at).and_then(RustToken::identifier) == Some("mut") {
            name_at += 1;
        }
        let Some(name) = tokens.get(name_at).and_then(RustToken::identifier) else {
            continue;
        };
        let Some(statement_end) = tokens[name_at + 1..]
            .iter()
            .position(|candidate| candidate.punctuation() == Some(';'))
            .map(|relative| name_at + 1 + relative)
        else {
            continue;
        };
        let Some(value_at) = tokens[name_at + 1..statement_end]
            .iter()
            .position(|candidate| candidate.punctuation() == Some('='))
            .map(|relative| name_at + 2 + relative)
        else {
            continue;
        };
        if let Some(value) = tokens.get(value_at).and_then(RustToken::string) {
            unresolved.push((name.to_owned(), None, Some(value.to_owned())));
        } else if let Some(alias) = tokens.get(value_at).and_then(RustToken::identifier) {
            unresolved.push((name.to_owned(), Some(alias.to_owned()), None));
        }
    }

    let mut bindings = BTreeMap::new();
    loop {
        let mut progress = false;
        for (name, alias, literal) in &unresolved {
            if bindings.contains_key(name) {
                continue;
            }
            let value = literal.clone().or_else(|| {
                alias
                    .as_ref()
                    .and_then(|alias| bindings.get(alias).cloned())
            });
            if let Some(value) = value {
                bindings.insert(name.clone(), value);
                progress = true;
            }
        }
        if !progress {
            break;
        }
    }
    bindings
}

fn call_arguments(tokens: &[RustToken], open: usize) -> Vec<(usize, usize)> {
    let mut arguments = Vec::new();
    let mut depth = 0_usize;
    let mut start = open + 1;
    for (at, token) in tokens.iter().enumerate().skip(open + 1) {
        match token.punctuation() {
            Some('(' | '[' | '{') => depth += 1,
            Some(')' | ']' | '}') if depth > 0 => depth -= 1,
            Some(')') => {
                if start < at {
                    arguments.push((start, at));
                }
                break;
            }
            Some(',') if depth == 0 => {
                arguments.push((start, at));
                start = at + 1;
            }
            _ => {}
        }
    }
    arguments
}

struct AnchorCall {
    line: usize,
    name: &'static str,
    arguments: Vec<(usize, usize)>,
}

fn argument_identifier(tokens: &[RustToken], argument: (usize, usize)) -> Option<&str> {
    let (mut start, end) = argument;
    while start < end && matches!(tokens[start].punctuation(), Some('&' | '*')) {
        start += 1;
    }
    (start + 1 == end)
        .then(|| tokens[start].identifier())
        .flatten()
}

fn argument_string(
    tokens: &[RustToken],
    argument: (usize, usize),
    bindings: &BTreeMap<String, String>,
) -> Option<String> {
    let (mut start, end) = argument;
    while start < end && matches!(tokens[start].punctuation(), Some('&' | '*')) {
        start += 1;
    }
    if start + 1 != end {
        return None;
    }
    tokens[start].string().map(str::to_owned).or_else(|| {
        tokens[start]
            .identifier()
            .and_then(|identifier| bindings.get(identifier).cloned())
    })
}

fn anchor_calls(tokens: &[RustToken], raw_bindings: &BTreeSet<&str>) -> Vec<AnchorCall> {
    let mut calls = Vec::new();
    for (at, token) in tokens.iter().enumerate() {
        if token.identifier() == Some("section")
            && tokens
                .get(at.saturating_sub(1))
                .and_then(RustToken::identifier)
                != Some("fn")
            && tokens.get(at + 1).and_then(RustToken::punctuation) == Some('(')
        {
            let arguments = call_arguments(tokens, at + 1);
            if arguments
                .first()
                .and_then(|argument| argument_identifier(tokens, *argument))
                .is_some_and(|source| raw_bindings.contains(source))
            {
                calls.push(AnchorCall {
                    line: token.line,
                    name: "section",
                    arguments,
                });
            }
        }
        if !matches!(token.identifier(), Some("find" | "contains"))
            || tokens
                .get(at.saturating_sub(1))
                .and_then(RustToken::punctuation)
                != Some('.')
            || tokens.get(at + 1).and_then(RustToken::punctuation) != Some('(')
        {
            continue;
        }
        let statement_start = tokens[..at.saturating_sub(1)]
            .iter()
            .rposition(|candidate| matches!(candidate.punctuation(), Some(';' | '{' | '}')))
            .map_or(0, |boundary| boundary + 1);
        if tokens[statement_start..at.saturating_sub(1)]
            .iter()
            .any(|candidate| {
                candidate
                    .identifier()
                    .is_some_and(|identifier| raw_bindings.contains(identifier))
            })
        {
            calls.push(AnchorCall {
                line: token.line,
                name: if token.identifier() == Some("find") {
                    "find"
                } else {
                    "contains"
                },
                arguments: call_arguments(tokens, at + 1),
            });
        }
    }
    calls
}

fn tolerant_anchor_contract(
    file: &str,
    source: &str,
    includes: &[IncludeExpectation<'_>],
) -> Vec<String> {
    let mut errors = Vec::new();
    let tokens = rust_tokens(source);
    let bindings = string_bindings(&tokens);
    let raw_bindings: BTreeSet<_> = includes.iter().map(|include| include.binding).collect();
    for include in includes {
        let references = identifier_occurrences(source, include.binding);
        if references != include.references {
            errors.push(format!(
                "{file}: consumidor tolerante {} cambió su inventario de usos crudos/aliases; referencias={}, esperadas={}",
                include.binding,
                references,
                include.references
            ));
        }
    }
    for call in anchor_calls(&tokens, &raw_bindings) {
        let first_anchor = usize::from(call.name == "section");
        for argument in call.arguments.iter().skip(first_anchor) {
            let Some(anchor) = argument_string(&tokens, *argument, &bindings) else {
                continue;
            };
            if has_internal_lf_anchor(&anchor) {
                errors.push(format!(
                    "{file}:{}: consumidor tolerante adquirió un anchor con salto LF interno en {}: {anchor:?}",
                    call.line, call.name
                ));
            }
        }
    }
    errors
}

fn audit_suite_fixture_impl(suite: &[(&str, &str, bool)]) -> Result<(), String> {
    let mut errors = Vec::new();
    let observed_names: BTreeSet<_> = suite.iter().map(|(file, _, _)| *file).collect();
    let expected_names: BTreeSet<_> = SUITE_EXPECTATIONS
        .iter()
        .map(|expectation| expectation.file)
        .collect();
    if suite.len() != SUITE_EXPECTATIONS.len() || observed_names != expected_names {
        errors.push(format!(
            "inventario de suite ambiguo: entradas={}, nombres={observed_names:?}, esperados={expected_names:?}",
            suite.len()
        ));
    }

    for expectation in SUITE_EXPECTATIONS {
        let matches: Vec<_> = suite
            .iter()
            .filter(|(file, _, _)| *file == expectation.file)
            .collect();
        if matches.len() != 1 {
            errors.push(format!(
                "{}: inventario requiere una entrada exacta; observadas={}",
                expectation.file,
                matches.len()
            ));
            continue;
        }
        let (_, source, sensitive) = *matches[0];
        if sensitive != expectation.sensitive {
            errors.push(format!(
                "{}: partición CRLF incorrecta; sensible={sensitive}, esperado={}",
                expectation.file, expectation.sensitive
            ));
        }

        let observed_includes = production_includes(source);
        let observed_identities: Vec<_> = observed_includes
            .iter()
            .map(|include| include_identity(include).to_owned())
            .collect();
        let expected_includes = expected_production_includes(expectation);
        if observed_identities != expected_includes {
            errors.push(format!(
                "{}: cada include productivo y binding debe estar inventariado exactamente; observados={observed_includes:?}, esperados={expected_includes:?}",
                expectation.file
            ));
        }

        if expectation.sensitive {
            let raw_sources: Vec<_> = expectation
                .includes
                .iter()
                .map(|include| RawSource {
                    constant: include.binding,
                })
                .collect();
            errors.extend(raw_source_contract(&Consumer {
                file: expectation.file,
                source,
                raw_sources: &raw_sources,
            }));
            if let Some((decoded, expected_occurrences)) = expectation.anchor {
                let encoded = encoded_anchor(decoded);
                let occurrences = source.matches(&encoded).count();
                if occurrences != expected_occurrences {
                    errors.push(format!(
                        "{}: anchor sensible no autenticado; anchor={encoded:?}, ocurrencias={occurrences}, esperadas={expected_occurrences}",
                        expectation.file
                    ));
                }
                let lf = format!("prefix{decoded}suffix");
                let crlf = lf.replace('\n', "\r\n");
                if !has_internal_lf_anchor(&encoded)
                    || crlf.contains(decoded)
                    || !crlf.replace("\r\n", "\n").contains(decoded)
                {
                    errors.push(format!(
                        "{}: el anchor sensible no demuestra el contrafactual CRLF→LF",
                        expectation.file
                    ));
                }
            }
        } else {
            errors.extend(tolerant_anchor_contract(
                expectation.file,
                source,
                expectation.includes,
            ));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n- "))
    }
}

mod suite_audit {
    /// Audita una suite inyectada para que el inventario, la partición CRLF y las guardas compartan
    /// una única verdad ejecutable. Los callers contrafactuales pueden sustituir un fuente sin
    /// tocar el checkout ni depender del descubrimiento del filesystem.
    pub(super) fn audit_suite_fixture(suite: &[(&str, &str, bool)]) -> Result<(), String> {
        super::audit_suite_fixture_impl(suite)
    }
}

pub(crate) fn audit_suite_fixture(suite: &[(&str, &str, bool)]) -> Result<(), String> {
    suite_audit::audit_suite_fixture(suite)
}

#[test]
fn ci38_inventaria_exhaustivamente_consumidores_e35_h03_de_fuentes_productivos() {
    let tests = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let discovered: BTreeSet<_> = std::fs::read_dir(&tests)
        .expect("anti-vacuity: debe existir el directorio de tests del crate")
        .map(|entry| {
            entry
                .expect("anti-vacuity: entrada de tests legible")
                .path()
        })
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("e35_h03_") && name.ends_with(".rs"))
        })
        .filter(|path| {
            let source =
                std::fs::read_to_string(path).expect("anti-vacuity: consumidor E35-H03 legible");
            !production_includes(&source).is_empty()
        })
        .map(|path| path.file_name().unwrap().to_str().unwrap().to_owned())
        .collect();

    let expected: BTreeSet<_> = SUITE_EXPECTATIONS
        .iter()
        .map(|expectation| expectation.file.to_owned())
        .collect();
    assert_eq!(
        discovered, expected,
        "CI38: apareció o desapareció un consumidor estructural; clasificar explícitamente su sensibilidad CRLF"
    );
    audit_suite_fixture(&real_suite())
        .expect("CI38: el inventario descubierto debe satisfacer el auditor compartido");
}

#[test]
fn ci38_autentica_anchors_multilinea_reales_y_su_fallo_crlf() {
    let suite = real_suite();
    audit_suite_fixture(&suite).expect("la suite real debe superar el auditor compartido");
    for expectation in SUITE_EXPECTATIONS {
        let Some((decoded, expected_source_occurrences)) = expectation.anchor else {
            continue;
        };
        let source = suite
            .iter()
            .find(|(file, _, _)| *file == expectation.file)
            .map(|(_, source, _)| *source)
            .expect("anti-vacuidad: todo anchor pertenece a un consumidor inventariado");
        let encoded = encoded_anchor(decoded);
        assert_eq!(
            source.matches(&encoded).count(),
            expected_source_occurrences,
            "{}: el anchor debe autenticarse contra el consumidor real; codificado={encoded:?}",
            expectation.file
        );
        assert!(
            decoded
                .find('\n')
                .is_some_and(|at| at > 0 || decoded.matches('\n').count() >= 2),
            "{}: el anchor debe contener un salto interno, no solo un prefijo LF tolerante a CRLF",
            expectation.file
        );
        let lf = format!("prefijo{decoded}sufijo");
        assert_eq!(lf.matches(decoded).count(), 1);
        let crlf = lf.replace('\n', "\r\n");
        assert!(crlf.contains("\r\n"), "contrafactual CRLF no vacío");
        assert_eq!(
            crlf.matches(decoded).count(),
            0,
            "{}: un checkout CRLF debe refutar el anchor LF crudo",
            expectation.file
        );
        assert_eq!(
            crlf.replace("\r\n", "\n").matches(decoded).count(),
            1,
            "{}: una normalización CRLF→LF restaura el anchor exacto",
            expectation.file
        );
    }
}

#[test]
fn ci38_guardas_rechazan_alias_crudo_contains_directo_y_alias_de_section() {
    let baseline = format!(
        "const STORE_SOURCE: &str = include_str!(\"..{}src/lib.rs\");\n\
         let sources = STORE_SOURCE.replace(\"\\r\\n\", \"\\n\");\n\
         section(&sources.store, \"inicio\", \"final\");",
        "/"
    );
    let baseline_consumer = Consumer {
        file: "contrafactual.rs",
        source: &baseline,
        raw_sources: &[RawSource {
            constant: "STORE_SOURCE",
        }],
    };
    assert!(
        raw_source_contract(&baseline_consumer).is_empty(),
        "anti-vacuidad: el control con una única normalización debe ser admitido"
    );

    for forbidden in [
        "\nlet raw_alias = STORE_SOURCE;",
        "\nlet direct = STORE_SOURCE.contains(\"marker\");",
    ] {
        let mutated = format!("{baseline}{forbidden}");
        let consumer = Consumer {
            file: "contrafactual.rs",
            source: &mutated,
            raw_sources: &[RawSource {
                constant: "STORE_SOURCE",
            }],
        };
        assert!(
            !raw_source_contract(&consumer).is_empty(),
            "CI38 debe rechazar el escape crudo contrafactual {forbidden:?}"
        );
    }

    let section_alias = baseline.replace(
        "section(&sources.store,",
        "let normalized_alias = &sources.store; section(normalized_alias,",
    );
    let errors = section_contract(&SectionConsumer {
        name: "contrafactual",
        source: &section_alias,
        expected_sections: 1,
        allowed_inputs: &["&sources.store"],
    });
    assert!(
        errors.iter().any(|error| error.contains("normalized_alias")),
        "CI38 debe rechazar también aliases aunque apunten a una fuente ya normalizada; errores={errors:?}"
    );
}

#[test]
fn ci38_normaliza_una_vez_y_bloquea_todo_uso_crudo_o_alias() {
    let mut errors = audit_suite_fixture(&real_suite())
        .err()
        .into_iter()
        .collect::<Vec<_>>();

    for section_consumer in [
        SectionConsumer {
            name: "CI19",
            source: CI19_SOURCE,
            expected_sections: 12,
            allowed_inputs: &["&sources.store", "&sources.windows_vfs"],
        },
        SectionConsumer {
            name: "CI23/review",
            source: CI23_REVIEW_SOURCE,
            expected_sections: 9,
            allowed_inputs: &["&sources.store", "&sources.schema", "&sources.windows_vfs"],
        },
    ] {
        errors.extend(section_contract(&section_consumer));
    }

    assert!(
        errors.is_empty(),
        "rojo causal CI38: consumidores estructurales no portátiles:\n- {}",
        errors.join("\n- ")
    );
}
