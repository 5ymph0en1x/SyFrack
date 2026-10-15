//! English/French UI messages. Scene data, paths and shader keys are never translated.
//! The catalog is compiled into the binary; no runtime downloads or locale files.
use std::{
    cell::Cell,
    io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU8, Ordering},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum Language {
    #[default]
    English = 0,
    French = 1,
}
impl Language {
    pub const ALL: [Self; 2] = [Self::English, Self::French];
    pub fn code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::French => "fr",
        }
    }
    pub fn native_name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::French => "Français",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "en" => Some(Self::English),
            "fr" => Some(Self::French),
            _ => None,
        }
    }
}
static LANGUAGE: AtomicU8 = AtomicU8::new(0);
thread_local! { static OVERRIDE: Cell<Option<Language>> = const { Cell::new(None) }; }

pub fn language() -> Language {
    OVERRIDE.with(|value| value.get()).unwrap_or_else(|| {
        if LANGUAGE.load(Ordering::Relaxed) == 1 {
            Language::French
        } else {
            Language::English
        }
    })
}
pub fn set_language(language: Language) {
    LANGUAGE.store(language as u8, Ordering::Relaxed);
}

/// Scoped override for tests and deterministic offscreen captures; other threads are unaffected.
pub fn with_language<T>(language: Language, f: impl FnOnce() -> T) -> T {
    struct Restore(Option<Language>);
    impl Drop for Restore {
        fn drop(&mut self) {
            OVERRIDE.with(|v| v.set(self.0));
        }
    }
    let _restore = Restore(OVERRIDE.with(|v| v.replace(Some(language))));
    f()
}

pub struct Message {
    pub key: &'static str,
    pub en: &'static str,
    pub fr: &'static str,
}
include!(concat!(env!("OUT_DIR"), "/catalog.rs"));
pub fn text(key: &str) -> &str {
    text_in(language(), key)
}

/// Original message keys are stable identifiers, including for detached window IDs.
pub fn canonical_key(text: &str) -> &str {
    MESSAGES
        .iter()
        .find(|m| m.key == text || m.en == text || m.fr == text)
        .map_or(text, |m| m.key)
}

#[macro_export]
macro_rules! tr {
    ($text:expr) => {
        $crate::text($text)
    };
}

pub fn preference_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base =
        std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Library/Application Support"));
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")));
    base.map(|p| p.join("SyFrack/language.txt"))
}
pub fn read_preference(path: &Path) -> Option<Language> {
    Language::parse(&std::fs::read_to_string(path).ok()?)
}
pub fn save_preference(path: &Path, language: Language) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, format!("{}\n", language.code()))
}
/// Environment override, then the saved preference, then English on first launch.
pub fn initialize() {
    let value = std::env::var("SYFRACK_LANG")
        .ok()
        .and_then(|s| Language::parse(&s))
        .or_else(|| preference_path().and_then(|p| read_preference(&p)))
        .unwrap_or_default();
    set_language(value);
}
pub fn select(language: Language) -> io::Result<()> {
    set_language(language);
    if let Some(path) = preference_path() {
        save_preference(&path, language)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_messages_have_both_languages_and_unique_keys() {
        let mut keys = std::collections::HashSet::new();
        for m in MESSAGES {
            assert!(!m.en.is_empty() && !m.fr.is_empty(), "{}", m.key);
            assert!(keys.insert(m.key), "duplicate key: {}", m.key);
            assert_eq!(text_in(Language::English, m.key), m.en);
            assert_eq!(text_in(Language::French, m.key), m.fr);
        }
    }
    #[test]
    fn language_scopes_nest_restore_and_do_not_leak_to_other_threads() {
        with_language(Language::French, || {
            assert_eq!(text("Fichier"), "Fichier");
            with_language(Language::English, || assert_eq!(text("Fichier"), "File"));
            assert_eq!(language(), Language::French);
            std::thread::spawn(|| assert_eq!(language(), Language::English))
                .join()
                .unwrap();
        });
        assert_eq!(language(), Language::English);
        assert_eq!(text("user-provided name"), "user-provided name");
    }
    #[test]
    fn preference_roundtrip_and_invalid_file() {
        let path =
            std::env::temp_dir().join(format!("syfrack-language-{}.txt", std::process::id()));
        save_preference(&path, Language::French).unwrap();
        assert_eq!(read_preference(&path), Some(Language::French));
        std::fs::write(&path, "invalid").unwrap();
        assert_eq!(read_preference(&path), None);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn detached_window_identity_is_language_independent() {
        assert_eq!(
            canonical_key("Render / Export"),
            canonical_key("Rendre / Exporter")
        );
    }
}
