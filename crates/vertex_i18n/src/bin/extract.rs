//! `cargo run -p vertex_i18n --bin extract -- [--write] [--check <language.toml>]`
//!
//! Lists the launcher's translatable messages. With no flags it prints the template; `--write` updates
//! `crates/vertex_i18n/lang/template.toml`; `--check file.toml` reports what a translation file is missing or gets wrong.

use std::path::PathBuf;

fn main() {
    let crates = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().expect("crates directory").to_path_buf();
    let base = crates.parent().expect("workspace root").to_path_buf();
    let found = vertex_i18n::extract::scan_dirs(&[crates], &base);
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(position) = args.iter().position(|arg| arg == "--check") {
        let Some(path) = args.get(position + 1) else {
            eprintln!("--check needs a language file");
            std::process::exit(2);
        };
        let text = std::fs::read_to_string(path).unwrap_or_else(|err| {
            eprintln!("cannot read {path}: {err}");
            std::process::exit(2);
        });
        let file: vertex_i18n::LanguageFile = toml::from_str(&text).unwrap_or_else(|err| {
            eprintln!("{path} is not a valid language file: {err}");
            std::process::exit(2);
        });
        let audit = vertex_i18n::extract::audit(&found, &file);
        println!("{} messages not translated yet", audit.untranslated);
        for key in &audit.unknown {
            println!("no longer in the launcher (can be removed): {key}");
        }
        for problem in &audit.bad_placeholders {
            println!("placeholder problem: {problem}");
        }
        std::process::exit(i32::from(!audit.bad_placeholders.is_empty()));
    }
    let template = vertex_i18n::extract::render_template(&found);
    if args.iter().any(|arg| arg == "--write") {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("lang/template.toml");
        std::fs::write(&path, template).expect("write the template");
        println!("wrote {} ({} messages)", path.display(), found.messages.len());
    } else {
        print!("{template}");
    }
}
