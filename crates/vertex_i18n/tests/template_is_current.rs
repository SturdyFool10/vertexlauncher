//! The translators' template is generated from the source; this fails when a message was added or changed without
//! regenerating it (`cargo run -p vertex_i18n --bin extract -- --write`).

use std::path::PathBuf;

#[test]
fn the_template_lists_every_message_in_the_source() {
    let crates = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
    let base = crates.parent().unwrap().to_path_buf();
    let found = vertex_i18n::extract::scan_dirs(&[crates], &base);
    let expected = vertex_i18n::extract::render_template(&found);
    let current = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("lang/template.toml")).unwrap_or_default();
    assert!(
        expected == current,
        "lang/template.toml is out of date: run `cargo run -p vertex_i18n --bin extract -- --write`"
    );
}

#[test]
fn the_template_is_a_valid_language_file() {
    let text = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("lang/template.toml")).unwrap();
    let file: vertex_i18n::LanguageFile = toml::from_str(&text).expect("the template parses");
    assert!(file.strings.len() > 100);
    assert!(file.strings.values().all(String::is_empty));
}
