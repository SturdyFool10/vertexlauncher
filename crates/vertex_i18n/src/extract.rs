//! Finds every translatable message in the launcher's source and writes the template translators start from.
//!
//! It reads Rust source text for the language macros (`tr!`, `trn!`, `tr_ctx!`, `src!`) and for the notification macros
//! (`notification::info!` and friends), whose message is translated the same way. The English text is each message's key.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// One message found in the source.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Message {
    Plain { context: Option<String>, text: String },
    /// The plural message, keyed by its `other` form, with the `one` form for translators' reference.
    Plural { one: String, other: String },
}

/// Everything found, and the files it came from.
#[derive(Default, Debug)]
pub struct Found {
    pub messages: BTreeMap<Message, BTreeSet<String>>,
}

struct Scanner<'a> {
    text: &'a str,
    bytes: &'a [u8],
}

impl<'a> Scanner<'a> {
    fn new(text: &'a str) -> Self {
        Self { text, bytes: text.as_bytes() }
    }

    /// The index after the string literal that starts at `start` and its unescaped value, or `None` if `start` is not a
    /// string literal.
    fn string_literal(&self, start: usize) -> Option<(usize, String)> {
        let b = self.bytes;
        if b.get(start) == Some(&b'r') {
            let mut hashes = 0;
            let mut i = start + 1;
            while b.get(i) == Some(&b'#') {
                hashes += 1;
                i += 1;
            }
            if b.get(i) != Some(&b'"') {
                return None;
            }
            let close = format!("\"{}", "#".repeat(hashes));
            let from = i + 1;
            let end = self.text[from..].find(&close)? + from;
            return Some((end + close.len(), self.text[from..end].to_owned()));
        }
        if b.get(start) != Some(&b'"') {
            return None;
        }
        let mut out = String::new();
        let mut i = start + 1;
        while i < b.len() {
            match b[i] {
                b'"' => return Some((i + 1, out)),
                b'\\' => {
                    let next = *b.get(i + 1)?;
                    match next {
                        b'n' => out.push('\n'),
                        b't' => out.push('\t'),
                        b'r' => out.push('\r'),
                        b'0' => out.push('\0'),
                        b'\\' => out.push('\\'),
                        b'"' => out.push('"'),
                        b'\'' => out.push('\''),
                        b'\n' | b'\r' => {
                            // A backslash at the end of a line skips the newline and the next line's indentation.
                            i += 2;
                            while i < b.len() && (b[i] as char).is_whitespace() {
                                i += 1;
                            }
                            continue;
                        }
                        b'u' => {
                            let open = self.text[i + 2..].find('{')? + i + 2;
                            let close = self.text[open..].find('}')? + open;
                            let code = u32::from_str_radix(&self.text[open + 1..close], 16).ok()?;
                            out.push(char::from_u32(code)?);
                            i = close + 1;
                            continue;
                        }
                        _ => return None,
                    }
                    i += 2;
                }
                _ => {
                    let c = self.text[i..].chars().next()?;
                    out.push(c);
                    i += c.len_utf8();
                }
            }
        }
        None
    }

    /// The spans of a call's top-level arguments, starting just after its `(`, and the index of the closing `)`.
    fn arguments(&self, start: usize) -> Option<(Vec<(usize, usize)>, usize)> {
        let b = self.bytes;
        let mut spans = Vec::new();
        let (mut depth, mut i, mut from) = (0usize, start, start);
        while i < b.len() {
            match b[i] {
                b'"' => {
                    i = self.string_literal(i)?.0;
                    continue;
                }
                b'r' if matches!(b.get(i + 1), Some(b'"') | Some(b'#')) && (i == 0 || !is_ident(b[i - 1])) => {
                    if let Some((end, _)) = self.string_literal(i) {
                        i = end;
                        continue;
                    }
                }
                b'/' if b.get(i + 1) == Some(&b'/') => {
                    i = self.text[i..].find('\n').map_or(b.len(), |n| i + n);
                    continue;
                }
                b'\'' => {
                    // A character literal (`'x'`, `'\n'`) must not be read as the start of something else.
                    if let Some(rest) = self.text[i + 1..].strip_prefix('\\') {
                        if let Some(n) = rest.find('\'') {
                            i += n + 3;
                            continue;
                        }
                    } else if self.text[i + 1..].chars().nth(1) == Some('\'') {
                        i += 1 + self.text[i + 1..].chars().next().map_or(1, char::len_utf8) + 1;
                        continue;
                    }
                }
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => {
                    if depth == 0 {
                        spans.push((from, i));
                        return Some((spans, i));
                    }
                    depth -= 1;
                }
                b',' if depth == 0 => {
                    spans.push((from, i));
                    from = i + 1;
                }
                _ => {}
            }
            i += 1;
        }
        None
    }

    /// The value of the argument at `span` if it is exactly one string literal.
    fn literal_argument(&self, span: (usize, usize)) -> Option<String> {
        let inner = self.text[span.0..span.1].trim();
        let offset = span.0 + (self.text[span.0..span.1].len() - self.text[span.0..span.1].trim_start().len());
        let (end, value) = self.string_literal(offset)?;
        (end >= offset + inner.len()).then_some(value)
    }
}

fn is_ident(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// The messages in one file's source.
pub fn scan_source(text: &str, file: &str, found: &mut Found) {
    let scanner = Scanner::new(text);
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        // Skip comments and strings so a macro name inside one is not read as a call.
        if b[i] == b'/' && b.get(i + 1) == Some(&b'/') {
            i = text[i..].find('\n').map_or(b.len(), |n| i + n);
            continue;
        }
        if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
            i = text[i..].find("*/").map_or(b.len(), |n| i + n + 2);
            continue;
        }
        if b[i] == b'"' {
            i = scanner.string_literal(i).map_or(i + 1, |(end, _)| end);
            continue;
        }
        if is_ident(b[i]) && (i == 0 || !is_ident(b[i - 1])) {
            let end = (i..b.len()).find(|&k| !is_ident(b[k])).unwrap_or(b.len());
            let name = &text[i..end];
            if b.get(end) == Some(&b'!') && b.get(end + 1) == Some(&b'(') {
                let line = text[..i].matches('\n').count() + 1;
                let origin = format!("{file}:{line}");
                if let Some((spans, _)) = scanner.arguments(end + 2) {
                    let literal = |index: usize| spans.get(index).and_then(|span| scanner.literal_argument(*span));
                    let preceded_by_notification = text[..i].trim_end_matches(':').ends_with("notification");
                    let mut add = |message: Message| {
                        // A message that is only a placeholder ("{err}") has no words to translate.
                        let only_placeholder = match &message {
                            Message::Plain { text, .. } => {
                                let trimmed = text.trim();
                                trimmed.starts_with('{') && trimmed.ends_with('}') && crate::template::placeholders(trimmed).len() == 1 && !trimmed[1..trimmed.len() - 1].contains(['{', '}'])
                            }
                            Message::Plural { .. } => false,
                        };
                        if !only_placeholder {
                            found.messages.entry(message).or_default().insert(origin.clone());
                        }
                    };
                    match name {
                        "tr" | "src" => {
                            if let Some(text) = literal(0) {
                                add(Message::Plain { context: None, text });
                            }
                        }
                        "tr_ctx" => {
                            if let (Some(context), Some(text)) = (literal(0), literal(1)) {
                                add(Message::Plain { context: Some(context), text });
                            }
                        }
                        "trn" => {
                            if let (Some(one), Some(other)) = (literal(1), literal(2)) {
                                add(Message::Plural { one, other });
                            }
                        }
                        "info" | "warn" | "error" | "log" | "progress" if preceded_by_notification => {
                            let index = if name == "progress" { 3 } else if literal(1).is_some() { 1 } else { 0 };
                            if let Some(text) = literal(index) {
                                add(Message::Plain { context: None, text });
                            }
                        }
                        _ => {}
                    }
                }
            }
            i = end;
            continue;
        }
        i += 1;
    }
}

/// Every `.rs` file under `dir`, skipping build output.
pub fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if !matches!(path.file_name().and_then(|n| n.to_str()), Some("target" | ".git" | "tests")) {
                    stack.push(path);
                }
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// Scans every Rust file under each of `roots`, naming files relative to `base`.
pub fn scan_dirs(roots: &[PathBuf], base: &Path) -> Found {
    let mut found = Found::default();
    for root in roots {
        for file in rust_files(root) {
            if let Ok(text) = std::fs::read_to_string(&file) {
                let name = file.strip_prefix(base).unwrap_or(&file).to_string_lossy().replace('\\', "/");
                // Unit tests are not interface text.
                let body = text.split("#[cfg(test)]").next().unwrap_or(&text);
                scan_source(body, &name, &mut found);
            }
        }
    }
    found
}

fn toml_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The template language file: every message with an empty translation, ready to fill in.
pub fn render_template(found: &Found) -> String {
    let mut plain: BTreeSet<&String> = BTreeSet::new();
    let mut contexts: BTreeMap<&String, BTreeSet<&String>> = BTreeMap::new();
    let mut plurals: BTreeMap<&String, &String> = BTreeMap::new();
    for message in found.messages.keys() {
        match message {
            Message::Plain { context: None, text } => {
                plain.insert(text);
            }
            Message::Plain { context: Some(context), text } => {
                contexts.entry(context).or_default().insert(text);
            }
            Message::Plural { one, other } => {
                plurals.insert(other, one);
            }
        }
    }
    let mut out = String::new();
    out.push_str("# Vertex Launcher language template.\n");
    out.push_str("# Copy this file to `<tag>.toml` (for example `es.toml`), fill in [language], and translate the messages below.\n");
    out.push_str("# Leave a message empty to keep showing it in English. Keep every {placeholder} exactly as written (you may move\n");
    out.push_str("# or repeat them); see README.md in this folder. This file is generated: `cargo run -p vertex_i18n --bin extract -- --write`.\n\n");
    out.push_str("[language]\ntag = \"\"\nname = \"\"\nnative_name = \"\"\nright_to_left = false\ntranslators = []\n\n");
    out.push_str("# [locale] is optional: it describes how the language counts, joins lists and writes numbers. See README.md.\n\n");
    out.push_str("[strings]\n");
    for text in &plain {
        let placeholders = crate::template::placeholders(text);
        if !placeholders.is_empty() {
            out.push_str(&format!("# placeholders: {}\n", placeholders.iter().map(|p| format!("{{{p}}}")).collect::<Vec<_>>().join(" ")));
        }
        out.push_str(&format!("{} = \"\"\n", toml_string(text)));
    }
    for (context, texts) in &contexts {
        out.push_str(&format!("\n[contexts.{}]\n", toml_string(context)));
        for text in texts {
            out.push_str(&format!("{} = \"\"\n", toml_string(text)));
        }
    }
    for (other, one) in &plurals {
        out.push_str(&format!("\n[plurals.{}]\n# English one: {}\none = \"\"\nother = \"\"\n", toml_string(other), toml_string(one)));
    }
    out
}

/// The messages a language file translates that no longer exist in the source, and the template messages it lacks, with
/// placeholders it uses that the English message does not have.
#[derive(Debug, Default)]
pub struct Audit {
    pub untranslated: usize,
    pub unknown: Vec<String>,
    pub bad_placeholders: Vec<String>,
}

pub fn audit(found: &Found, file: &crate::LanguageFile) -> Audit {
    let mut audit = Audit::default();
    let mut known: BTreeSet<&str> = BTreeSet::new();
    for message in found.messages.keys() {
        match message {
            Message::Plain { context: None, text } => {
                known.insert(text);
                match file.strings.get(text).filter(|translation| !translation.is_empty()) {
                    None => audit.untranslated += 1,
                    Some(translation) => check_placeholders(text, translation, &mut audit),
                }
            }
            Message::Plain { context: Some(context), text } => {
                known.insert(text);
                match file.contexts.get(context).and_then(|entries| entries.get(text)).filter(|translation| !translation.is_empty()) {
                    None => audit.untranslated += 1,
                    Some(translation) => check_placeholders(text, translation, &mut audit),
                }
            }
            Message::Plural { other, .. } => {
                known.insert(other);
                if file.plurals.get(other).is_none_or(|forms| forms.other.is_empty()) {
                    audit.untranslated += 1;
                }
            }
        }
    }
    for key in file.strings.keys() {
        if !known.contains(key.as_str()) {
            audit.unknown.push(key.clone());
        }
    }
    audit
}

fn check_placeholders(source: &str, translation: &str, audit: &mut Audit) {
    let wanted = crate::template::placeholders(source);
    for used in crate::template::placeholders(translation) {
        if !wanted.contains(&used) {
            audit.bad_placeholders.push(format!("{source:?} uses {{{used}}}, which the English text does not have"));
        }
    }
}
