# Translating Vertex Launcher

A language is one TOML file. Put it in the launcher's `languages` folder (next to the `themes` folder in the launcher's
settings directory) and restart the launcher: the language appears in **Settings → Language**. Anything you leave
untranslated is shown in English, so a translation can be finished a little at a time.

## Start

1. Copy `template.toml` to `<tag>.toml`, for example `es.toml` or `pt-BR.toml`.
2. Fill in `[language]`: the `tag`, the language's name in English (`name`) and in itself (`native_name`, which is what
   the picker shows), and `right_to_left = true` for right-to-left scripts.
3. Translate the messages. The English text is the key; put your translation after the `=`.

## Messages and placeholders

```toml
[strings]
"Play" = "Jugar"
# placeholders: {user} {server}
"{user} joined {server}" = "{user} se unió a {server}"
```

* Keep every `{placeholder}` spelled exactly as in the English text. You may **move** them (word order differs between
  languages), **repeat** them, or leave one out if your grammar does not need it.
* `{{` and `}}` are a literal `{` and `}`.
* A placeholder can carry a number format: `{speed:.1}` writes one decimal place the way your language does (`1,5` in
  German) and `{count:n}` groups digits (`12.345`).
* Leave a message empty (`""`) to keep it in English.

### Plurals

Messages whose wording depends on a number live under `[plurals]`, keyed by the English plural text. Give each form
your language needs (`zero`, `one`, `two`, `few`, `many`, `other`); `other` is required and is used for any form you leave out.

```toml
[plurals."{count} mods"]
one = "{count} mod"
other = "{count} mods"
```

Which numbers use which form comes from the language's plural rule, see `plural_rule` below.

### Context

When the same English word is used in different places and needs different translations, the messages sit under
`[contexts.<name>]`:

```toml
[contexts.menu]
"Open" = "Abrir"
```

## How your language counts, lists and writes numbers: `[locale]`

This section is optional. Anything you leave out uses English's value, so add only what differs. Launcher text such as
"2 minutes and 5 seconds", "A, B and C", "12,345", "3rd" and month names come from here.

```toml
[locale]
tag = "ru"
plural_rule = "east_slavic"      # see the list below; the usual rule for the language code is used when omitted

[locale.number]
decimal = ","
group = " "                       # a no-break space
group_from = 4                    # the fewest digits that get grouped
grouping = "western"              # "western" (1,234,567), "indian" (12,34,567) or "none"
digits = ""                       # ten digits for non-ASCII numerals, such as "٠١٢٣٤٥٦٧٨٩"

[locale.words]
count = ["ноль", "один", "два", "три", "четыре", "пять", "шесть", "семь", "восемь", "девять", "десять"]
several = "несколько"
just_now = "только что"
ago = "{span} назад"
from_now = "через {span}"
about = "около {amount}"
remainder = "ещё {count}"
less_than_a_second = "меньше секунды"

[locale.units.day]                # also hour, minute, second and byte
one = "день"
few = "дня"
many = "дней"
other = "дня"

[locale.lists.and]                # also [locale.lists.or]
two = "{0} и {1}"
start = "{0}, {1}"
middle = "{0}, {1}"
end = "{0} и {1}"

[locale.ordinal]
rule = "single"                   # "single", "english" (1st 2nd 3rd) or "first_only" (1er, 2e)
other = "{n}-й"

[locale.calendar]
months_long = [...]               # January to December
months_short = [...]
weekdays_short = [...]            # Monday to Sunday
```

`plural_rule` is one of `none` (no plural forms: Japanese, Chinese, Korean, Vietnamese, Thai, Indonesian),
`one_other` (English, German, Dutch, Swedish, ...), `french` (also Portuguese), `spanish` (also Italian, Catalan),
`hindi` (also Bengali, Persian), `icelandic`, `east_slavic` (Russian, Ukrainian, Belarusian), `south_slavic` (Croatian,
Serbian, Bosnian), `polish`, `czech` (also Slovak), `slovenian`, `latvian`, `lithuanian`, `romanian`, `arabic`, `hebrew`,
`welsh`, `irish`, `maltese`, `filipino`.

A list pattern can switch a conjunction before certain words (Spanish `y` becomes `e` before an *i*-sound), with
`swaps = [{ from = " y ", to = " e ", when_next_starts_with = ["i", "hi"], except = ["hia", "hie"] }]`.

## Checking your work

```
cargo run -p vertex_i18n --bin extract -- --check path/to/es.toml
```

lists how many messages are still untranslated, messages that no longer exist in the launcher, and placeholders your
translation uses that the English text does not have.

## For developers

Interface text is written with the macros in `vertex_i18n`: `tr!("Hello {name}", name = n)`, `trn!(count, "{count} mod",
"{count} mods")`, `tr_ctx!("menu", "Open")`, and `src!("...")` for text kept in a table (shown with `vertex_i18n::t`).
The English text is the message's identity. `template.toml` is generated from the source with
`cargo run -p vertex_i18n --bin extract -- --write`; a test fails when it is out of date. Only text users read in the
interface belongs here: console and log output stays English.
