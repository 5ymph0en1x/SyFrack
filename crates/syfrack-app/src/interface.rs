//! A local presentation preference, independent of documents and undo history.
use std::{io, path::Path};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum InterfaceMode {
    Simple,
    #[default]
    Advanced,
}

impl InterfaceMode {
    pub const ALL: [Self; 2] = [Self::Simple, Self::Advanced];

    pub fn label(self) -> &'static str {
        match self {
            Self::Simple => syfrack_i18n::tr!("Simple"),
            Self::Advanced => syfrack_i18n::tr!("Avancée"),
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Simple => {
                syfrack_i18n::tr!("Créer avec les réglages essentiels, étape par étape.")
            }
            Self::Advanced => {
                syfrack_i18n::tr!("Composer librement avec le graphe et tous les outils.")
            }
        }
    }

    pub fn read(path: &Path) -> Option<Self> {
        match std::fs::read_to_string(path).ok()?.trim() {
            "simple" => Some(Self::Simple),
            "advanced" => Some(Self::Advanced),
            _ => None,
        }
    }

    pub fn write(self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(
            path,
            match self {
                Self::Simple => "simple\n",
                Self::Advanced => "advanced\n",
            },
        )
    }

    pub fn load() -> Self {
        syfrack_i18n::preference_path()
            .and_then(|path| Self::read(&path.with_file_name("interface.txt")))
            .unwrap_or_default()
    }

    pub fn save(self) -> io::Result<()> {
        if let Some(path) = syfrack_i18n::preference_path() {
            self.write(&path.with_file_name("interface.txt"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preference_roundtrip_and_invalid_value() {
        let path =
            std::env::temp_dir().join(format!("syfrack-interface-{}.txt", std::process::id()));
        for mode in InterfaceMode::ALL {
            mode.write(&path).unwrap();
            assert_eq!(InterfaceMode::read(&path), Some(mode));
        }
        std::fs::write(&path, "unknown").unwrap();
        assert_eq!(InterfaceMode::read(&path), None);
        std::fs::remove_file(path).unwrap();
        assert_eq!(InterfaceMode::default(), InterfaceMode::Advanced);
    }
}
