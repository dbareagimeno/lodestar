//! Render de markdown a HTML para el preview (feature `render`). Port de `mdRender`/`miniMd`.
//!
//! **El saneado vive en el frontend (DOMPurify, `§12`)**: este módulo solo produce el HTML;
//! nunca lo inyecta. No usar el HTML sin sanear.

use pulldown_cmark::{html, Options, Parser};

/// Convierte markdown a HTML (sin sanear). El cuerpo es el del `.md` (ya separado del frontmatter).
pub fn render_markdown(body: &str) -> String {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    let parser = Parser::new_ext(body, opts);
    let mut out = String::new();
    html::push_html(&mut out, parser);
    out
}
