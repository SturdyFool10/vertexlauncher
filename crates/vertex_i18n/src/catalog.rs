//! A language file and the translations in it.

use std::collections::HashMap;

use serde::Deserialize;

use crate::locale::LocaleData;
use crate::plural::PluralForms;

/// The `[language]` section.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct LanguageInfo {
    /// The language code: "es", "pt-BR", "zh-Hans".
    pub tag: String,
    /// The language's name in English.
    pub name: String,
    /// The language's name in itself, which the language picker shows.
    pub native_name: String,
    pub right_to_left: bool,
    /// Who made the translation; shown nowhere yet, kept for credit.
    pub translators: Vec<String>,
}

/// One language file, as written.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct LanguageFile {
    pub language: LanguageInfo,
    /// How the language counts, joins lists and writes numbers; anything left out is English's.
    pub locale: Option<LocaleData>,
    /// English text to its translation.
    pub strings: HashMap<String, String>,
    /// English text (the plural form) to its forms.
    pub plurals: HashMap<String, PluralForms>,
    /// Context to English text to translation, for words whose meaning depends on where they are used.
    pub contexts: HashMap<String, HashMap<String, String>>,
}

/// The separator between a context and the English text in a catalog key.
const CONTEXT_SEPARATOR: char = '\u{4}';

#[derive(Debug, Default)]
pub(crate) struct Catalog {
    strings: HashMap<String, String>,
    plurals: HashMap<String, PluralForms>,
}

fn key(context: Option<&str>, source: &str) -> String {
    match context {
        Some(context) => format!("{context}{CONTEXT_SEPARATOR}{source}"),
        None => source.to_owned(),
    }
}

impl Catalog {
    pub(crate) fn from_file(
        strings: &HashMap<String, String>,
        plurals: &HashMap<String, PluralForms>,
        contexts: &HashMap<String, HashMap<String, String>>,
    ) -> Self {
        let mut all: HashMap<String, String> = strings.iter().filter(|(_, text)| !text.is_empty()).map(|(k, v)| (k.clone(), v.clone())).collect();
        for (context, entries) in contexts {
            for (source, text) in entries.iter().filter(|(_, text)| !text.is_empty()) {
                all.insert(key(Some(context), source), text.clone());
            }
        }
        Self { strings: all, plurals: plurals.iter().filter(|(_, forms)| !forms.other.is_empty()).map(|(k, v)| (k.clone(), v.clone())).collect() }
    }

    pub(crate) fn string(&self, context: Option<&str>, source: &str) -> Option<&str> {
        match context {
            None => self.strings.get(source).map(String::as_str),
            Some(_) => self.strings.get(&key(context, source)).map(String::as_str),
        }
    }

    pub(crate) fn plural(&self, other: &str) -> Option<&PluralForms> {
        self.plurals.get(other)
    }

    pub(crate) fn len(&self) -> usize {
        self.strings.len() + self.plurals.len()
    }
}
