//! Languages for the launcher's own text.
//!
//! # Writing messages
//!
//! English is the source language, and the English text *is* the message's identity: write the message where it is shown
//! and let translators find it by that text.
//!
//! ```ignore
//! kit.text(ui, role, &tr!("No instances yet"));
//! kit.text(ui, role, &tr!("{count} mods installed in {instance}", count = n, instance = name));
//! kit.text(ui, role, &trn!(n, "{count} mod", "{count} mods"));
//! ```
//!
//! Values go in through named placeholders, never by gluing strings together, so a translation can reorder, repeat or
//! drop them to suit its grammar. Counts that change a noun use [`trn!`], which picks the plural form the language's own
//! rules ask for. A word that means different things in different places takes a context: [`tr_ctx!`]. Text in a table or
//! constant is marked with [`src!`] and translated where it is shown, with [`t`].
//!
//! # Translating
//!
//! A language is one TOML file (see `lang/README.md`): `[language]` names it, an optional `[locale]` describes how it
//! counts, joins lists and writes numbers (see [`locale`]), and `[strings]` / `[plurals]` hold the translations, each
//! keyed by the English text. Anything not translated shows in English. Put the file in the launcher's `languages`
//! folder and the language setting lists it.
//!
//! Only text the user reads in the launcher's interface belongs here. Console and log output stays English.

mod catalog;
pub mod locale;
pub mod extract;
pub mod number;
pub mod plural;
pub mod template;

pub use catalog::{LanguageFile, LanguageInfo};
pub use locale::{ListKind, Locale, LocaleData, MAX_COUNT_WORD};
pub use plural::{PluralCategory, PluralForms, PluralRule};

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

use catalog::Catalog;

/// The tag of the source language, which needs no translation file.
pub const SOURCE_LANGUAGE: &str = "en";

/// Language files shipped inside the launcher. Add `include_str!("../lang/xx.toml")` here for each one.
const EMBEDDED_LANGUAGES: &[&str] = &[];

/// Where a language came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LanguageSource {
    /// English, written in the code.
    Source,
    /// Shipped inside the launcher.
    Embedded,
    /// A community file in the languages folder.
    File(PathBuf),
}

/// A language the launcher can show its interface in.
#[derive(Debug)]
pub struct Language {
    pub tag: String,
    /// The language's name in English ("Spanish").
    pub name: String,
    /// The language's name in itself ("Español"), which is what the language picker shows.
    pub native_name: String,
    pub right_to_left: bool,
    pub source: LanguageSource,
    locale: Locale,
    catalog: Catalog,
}

impl Language {
    fn english() -> Self {
        Self {
            tag: SOURCE_LANGUAGE.to_owned(),
            name: "English".to_owned(),
            native_name: "English".to_owned(),
            right_to_left: false,
            source: LanguageSource::Source,
            locale: Locale::english(),
            catalog: Catalog::default(),
        }
    }

    fn from_file(file: LanguageFile, source: LanguageSource) -> Result<Self, String> {
        let tag = file.language.tag.trim().to_owned();
        if tag.is_empty() {
            return Err("the [language] section needs a tag, such as tag = \"es\"".to_owned());
        }
        let locale = match file.locale {
            Some(mut data) => {
                if data.tag == LocaleData::default().tag && !tag.eq_ignore_ascii_case("en") {
                    data.tag = tag.clone();
                }
                Locale::from_data(data)
            }
            None => Locale::builtin(&tag).unwrap_or_else(|| {
                Locale::from_data(LocaleData { tag: tag.clone(), ..LocaleData::default() })
            }),
        };
        let name = if file.language.name.is_empty() { tag.clone() } else { file.language.name.clone() };
        let native_name = if file.language.native_name.is_empty() { name.clone() } else { file.language.native_name.clone() };
        Ok(Self {
            tag,
            name,
            native_name,
            right_to_left: file.language.right_to_left,
            source,
            locale,
            catalog: Catalog::from_file(&file.strings, &file.plurals, &file.contexts),
        })
    }

    pub fn locale(&self) -> &Locale {
        &self.locale
    }

    /// Whether this is the source language, whose text is returned as written.
    pub fn is_source(&self) -> bool {
        self.source == LanguageSource::Source
    }

    /// How many of the message strings in `template` this language has translated; for translators' progress.
    pub fn translated_count(&self) -> usize {
        self.catalog.len()
    }
}

/// A language as the language setting lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanguageSummary {
    pub tag: String,
    pub name: String,
    pub native_name: String,
}

struct State {
    languages: Vec<Arc<Language>>,
    current: Arc<Language>,
}

fn state() -> &'static RwLock<State> {
    static STATE: OnceLock<RwLock<State>> = OnceLock::new();
    STATE.get_or_init(|| {
        let english = Arc::new(Language::english());
        let mut languages = vec![english.clone()];
        for source in EMBEDDED_LANGUAGES {
            match parse(source, LanguageSource::Embedded) {
                Ok(language) => languages.push(Arc::new(language)),
                Err(err) => tracing::error!(target: "vertex_i18n", "An embedded language file is invalid: {err}"),
            }
        }
        RwLock::new(State { languages, current: english })
    })
}

fn parse(source: &str, origin: LanguageSource) -> Result<Language, String> {
    let file: LanguageFile = toml::from_str(source).map_err(|err| err.to_string())?;
    Language::from_file(file, origin)
}

/// A language file that was skipped, and why.
#[derive(Clone, Debug)]
pub struct LoadProblem {
    pub path: PathBuf,
    pub message: String,
}

/// Reads every `*.toml` language file in `dir` (replacing the languages loaded from files before) and keeps the current
/// language if it is still there. A missing folder is fine. Returns the files that could not be used.
pub fn load_language_dir(dir: &Path) -> Vec<LoadProblem> {
    let mut problems = Vec::new();
    let mut loaded: Vec<Arc<Language>> = Vec::new();
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| entries.filter_map(Result::ok).map(|entry| entry.path()).collect())
        .unwrap_or_default();
    paths.sort();
    for path in paths.into_iter().filter(|path| path.extension().is_some_and(|ext| ext == "toml")) {
        match std::fs::read_to_string(&path)
            .map_err(|err| err.to_string())
            .and_then(|text| parse(&text, LanguageSource::File(path.clone())))
        {
            Ok(language) => loaded.push(Arc::new(language)),
            Err(message) => problems.push(LoadProblem { path, message }),
        }
    }
    let mut state = state().write().unwrap_or_else(|poisoned| poisoned.into_inner());
    state.languages.retain(|language| !matches!(language.source, LanguageSource::File(_)));
    for language in loaded {
        // A file replaces an embedded language with the same tag, so a newer community translation wins.
        state.languages.retain(|known| !known.tag.eq_ignore_ascii_case(&language.tag));
        state.languages.push(language);
    }
    let current_tag = state.current.tag.clone();
    state.current = state
        .languages
        .iter()
        .find(|language| language.tag.eq_ignore_ascii_case(&current_tag))
        .cloned()
        .unwrap_or_else(|| state.languages[0].clone());
    problems
}

/// [`load_language_dir`] on the launcher's own `languages` folder.
pub fn load_user_languages() -> Vec<LoadProblem> {
    load_language_dir(&app_paths::languages_dir())
}

/// The languages the launcher can show, English first, then the rest by name.
pub fn available_languages() -> Vec<LanguageSummary> {
    let state = state().read().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut rest: Vec<&Arc<Language>> = state.languages.iter().filter(|language| !language.is_source()).collect();
    rest.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    std::iter::once(&state.languages[0])
        .chain(rest)
        .map(|language| LanguageSummary {
            tag: language.tag.clone(),
            name: language.name.clone(),
            native_name: language.native_name.clone(),
        })
        .collect()
}

/// Switches the interface language to `tag` ("es", "pt-BR"; case and `_` or `-` do not matter). An empty tag or one that
/// is not available selects English. Returns whether the language was found.
pub fn set_language(tag: &str) -> bool {
    let wanted = tag.trim().replace('_', "-");
    let mut state = state().write().unwrap_or_else(|poisoned| poisoned.into_inner());
    let found = state.languages.iter().find(|language| language.tag.eq_ignore_ascii_case(&wanted)).cloned();
    let was_found = found.is_some() || wanted.is_empty();
    state.current = found.unwrap_or_else(|| state.languages[0].clone());
    was_found
}

/// The language the interface is in.
pub fn current_language() -> Arc<Language> {
    state().read().unwrap_or_else(|poisoned| poisoned.into_inner()).current.clone()
}

/// Whether the interface language is written right to left.
pub fn is_right_to_left() -> bool {
    current_language().right_to_left
}

pub(crate) fn current_locale() -> Locale {
    current_language().locale.clone()
}

pub(crate) fn locale_for_tag(tag: &str) -> Option<Locale> {
    let language = tag.split(['-', '_', '.', '@']).next()?.to_ascii_lowercase();
    let state = state().read().unwrap_or_else(|poisoned| poisoned.into_inner());
    state
        .languages
        .iter()
        .find(|known| known.tag.eq_ignore_ascii_case(tag.split(['.', '@']).next().unwrap_or(tag)) || known.locale.tag() == language)
        .map(|known| known.locale.clone())
}

// ---------------------------------------------------------------------------------------------------------------------
// Looking messages up
// ---------------------------------------------------------------------------------------------------------------------

fn finish(template: &str, args: &[(&str, String)]) -> String {
    template::render_with(template, args, current_language().locale.number_format())
}

/// The text for the English message `source`, with its placeholders filled in. Prefer [`tr!`].
pub fn translate(source: &'static str, args: &[(&str, String)]) -> Cow<'static, str> {
    let language = current_language();
    let template = if language.is_source() { None } else { language.catalog.string(None, source) };
    match template {
        Some(translated) => Cow::Owned(finish(translated, args)),
        None if args.is_empty() && !source.contains(['{', '}']) => Cow::Borrowed(source),
        None => Cow::Owned(finish(source, args)),
    }
}

/// [`translate`] for a message with a context. Prefer [`tr_ctx!`].
pub fn translate_ctx(context: &str, source: &'static str, args: &[(&str, String)]) -> Cow<'static, str> {
    let language = current_language();
    let template = if language.is_source() { None } else { language.catalog.string(Some(context), source) };
    match template {
        Some(translated) => Cow::Owned(finish(translated, args)),
        None if args.is_empty() && !source.contains(['{', '}']) => Cow::Borrowed(source),
        None => Cow::Owned(finish(source, args)),
    }
}

/// The plural form of a message for `count`, with `{count}` (written the way the language writes numbers) and any other
/// placeholders filled in. `other` is the message's identity in the catalog. Prefer [`trn!`].
pub fn translate_plural(count: u64, one: &'static str, other: &'static str, args: &[(&str, String)]) -> String {
    let language = current_language();
    let mut all: Vec<(&str, String)> = Vec::with_capacity(args.len() + 1);
    all.push(("count", language.locale.thousands(count)));
    all.extend(args.iter().map(|(name, value)| (*name, value.clone())));
    let template = match language.catalog.plural(other) {
        Some(forms) if !language.is_source() => forms.select(language.locale.plural_rule(), count).to_owned(),
        _ => PluralForms::new(one, other).select(PluralRule::OneOther, count).to_owned(),
    };
    finish(&template, &all)
}

/// The translation of text that is only known as a string at the point it is shown, such as a label in a table of
/// settings (mark the literal with [`src!`]). No placeholders. Untranslated text comes back as it was.
pub fn t(source: &str) -> Cow<'_, str> {
    let language = current_language();
    if language.is_source() {
        return Cow::Borrowed(source);
    }
    match language.catalog.string(None, source) {
        Some(translated) => Cow::Owned(translated.to_owned()),
        None => Cow::Borrowed(source),
    }
}

/// `source` in every language the launcher has (the source language's own text included), for finding something by a
/// word in a language other than the one the interface is in. Languages that do not translate it add nothing new.
pub fn translations_everywhere(source: &str) -> Vec<String> {
    let state = state().read().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut found = vec![source.to_owned()];
    for language in &state.languages {
        if let Some(translated) = language.catalog.string(None, source) {
            if !found.iter().any(|known| known == translated) {
                found.push(translated.to_owned());
            }
        }
    }
    found
}

/// A value for a placeholder, as text.
pub fn arg(value: &dyn std::fmt::Display) -> String {
    value.to_string()
}

/// A whole number written the way the interface language writes numbers ("12,345").
pub fn number(value: u64) -> String {
    current_language().locale.thousands(value)
}

/// Marks an English string in a table or constant for translation. It is the string itself; show it with [`t`].
#[macro_export]
macro_rules! src {
    ($text:literal) => {
        $text
    };
}

/// The interface text for a message, with named placeholders: `tr!("Hello {name}", name = user)`.
#[macro_export]
macro_rules! tr {
    ($text:literal $(,)?) => {
        $crate::translate($text, &[])
    };
    ($text:literal, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::translate($text, &[$((stringify!($name), $crate::arg(&$value))),+])
    };
}

/// [`tr!`] for a word or phrase whose meaning depends on where it is used: `tr_ctx!("menu", "Open")`.
#[macro_export]
macro_rules! tr_ctx {
    ($context:literal, $text:literal $(,)?) => {
        $crate::translate_ctx($context, $text, &[])
    };
    ($context:literal, $text:literal, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::translate_ctx($context, $text, &[$((stringify!($name), $crate::arg(&$value))),+])
    };
}

/// A message whose wording depends on a count: `trn!(n, "{count} mod", "{count} mods")`. `{count}` is filled in.
#[macro_export]
macro_rules! trn {
    ($count:expr, $one:literal, $other:literal $(,)?) => {
        $crate::translate_plural(($count) as u64, $one, $other, &[])
    };
    ($count:expr, $one:literal, $other:literal, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::translate_plural(($count) as u64, $one, $other, &[$((stringify!($name), $crate::arg(&$value))),+])
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests share the process-wide language, so each one that changes it holds this lock.
    pub(crate) static LANGUAGE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn write_language(dir: &Path, name: &str, text: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(name), text).unwrap();
    }

    const SPANISH: &str = r#"
[language]
tag = "es"
name = "Spanish"
native_name = "Español"

[strings]
"Play" = "Jugar"
"{user} joined {server}" = "{user} se unió a {server}"
"Taken at {time}" = "Tomada a las {time}"

[contexts.menu]
"Open" = "Abrir"

[contexts.door]
"Open" = "Abierta"

[plurals."{count} mods"]
one = "{count} mod"
other = "{count} mods"
"#;

    #[test]
    fn english_is_returned_as_written_with_placeholders_filled() {
        let _guard = LANGUAGE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        set_language("en");
        assert_eq!(tr!("Play"), "Play");
        assert_eq!(tr!("{a} and {b}", a = 1, b = "x"), "1 and x");
        assert_eq!(tr!("Literal {{braces}}"), "Literal {braces}");
        assert_eq!(trn!(1, "{count} mod", "{count} mods"), "1 mod");
        assert_eq!(trn!(2_500, "{count} mod", "{count} mods"), "2,500 mods");
        assert_eq!(t("Anything"), "Anything");
    }

    #[test]
    fn a_language_file_translates_by_english_text_and_falls_back_to_english() {
        let _guard = LANGUAGE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join(format!("vertex-i18n-test-{}", std::process::id()));
        write_language(&dir, "es.toml", SPANISH);
        write_language(&dir, "broken.toml", "this is not toml = = =");
        let problems = load_language_dir(&dir);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].path.ends_with("broken.toml"));

        let tags: Vec<String> = available_languages().into_iter().map(|language| language.tag).collect();
        assert_eq!(tags, vec!["en", "es"]);
        assert!(set_language("es"));
        assert_eq!(tr!("Play"), "Jugar");
        assert_eq!(tr!("{user} joined {server}", server = "Home", user = "Ada"), "Ada se unió a Home");
        assert_eq!(tr!("Not translated yet"), "Not translated yet");
        assert_eq!(tr_ctx!("menu", "Open"), "Abrir");
        assert_eq!(tr_ctx!("door", "Open"), "Abierta");
        assert_eq!(trn!(1, "{count} mod", "{count} mods"), "1 mod");
        assert_eq!(t("Play"), "Jugar");
        // The language's mechanics come from the built-in Spanish data.
        assert_eq!(Locale::current().thousands(12_345), "12.345");

        assert!(set_language("ES"));
        assert!(!set_language("xx"));
        assert_eq!(current_language().tag, "en");
        load_language_dir(&dir.join("missing"));
        assert_eq!(available_languages().len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }
}
