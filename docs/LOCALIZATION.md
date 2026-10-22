# Localization

Studio ships English and French, with English on first launch. No external
translation service or locale download is used. The `syfrack-i18n` crate compiles
the catalog into the binaries.

## Catalog format

`crates/syfrack-i18n/messages.tsv` has three tab-separated columns: stable key,
English message, French message. Each field is a quoted Rust string literal
(also readable as a JSON string). Keep literal tabs out of message text.
Use `\n` for line breaks. Existing French keys are stable identifiers retained
from the original UI; new keys may use English.

```text
"Language"\t"Language"\t"Langue"
```

The `\t` above represents an actual column separator in the file.

Use `syfrack_i18n::tr!("Language")` for static text. For formatted messages, use
`tr_format!` with a catalog key; pass named captures explicitly, for example
`tr_format!("Language preference could not be saved: {}", error)`.
The build script generates both language branches with Rust's `format!`, so
argument types are checked by the compiler. Translate before interpolation;
never translate filenames or an arbitrary rendered output string.

Run:

```text
python scripts/i18n/audit.py
cargo test -p syfrack-i18n
cargo test -p syfrack-app --lib
```

The audit verifies unique keys, both translations, matching format fields and
unlocalized French literals in Studio. A small explicit allowlist covers
constant tables translated at draw time and stable dialog keys. It is not a
substitute for visual review or detecting every possible ASCII-only sentence.

## State and threading

The selected language is shared across UI windows and worker diagnostics.
Tests use a scoped thread-local override, avoiding global-language races.
Language does not belong in `Scene`, undo history, parameter paths, RON variant
names, graph topology or checkpoint hashes.

Detached windows use canonical message keys for identity and translated text
for captions. Cached library/texture tables retain keys and translate on draw.
User-assigned names and paths are preserved.

## Preferences

The Windows preference is `%APPDATA%/SyFrack/language.txt`, containing `en` or
`fr`. Invalid or missing preferences fall back to English. `SYFRACK_LANG=en|fr`
overrides the preference at startup. Saving a language selection changes only
this preference, never the project. A write failure is shown in Studio.

Simple/Advanced is a separate presentation preference in
`%APPDATA%/SyFrack/interface.txt` (`simple` or `advanced`). Missing/invalid values
use Advanced. This choice also stays outside scene files and undo/redo history.

The CLI keeps English command syntax/help and offers `--lang en|fr` for
localized graph diagnostics. Forme source exports have a stable English syntax
independent of the UI language, and legacy French syntax remains readable.
