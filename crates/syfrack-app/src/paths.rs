//! Locate the Studio content independently of how Windows launched the GUI.
use std::path::{Path, PathBuf};

pub fn content_root() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    find_root(&cwd, std::env::current_exe().ok().as_deref())
}

fn find_root(cwd: &Path, exe: Option<&Path>) -> PathBuf {
    let is_content = |p: &&Path| p.join("scenes").is_dir() && p.join("graphs").is_dir();
    // Respect an explicitly selected workspace, then a portable installation
    // or a checkout above target/debug. Never change the process CWD: relative
    // paths supplied on the command line keep their original meaning.
    std::iter::once(cwd)
        .chain(
            exe.and_then(Path::parent)
                .into_iter()
                .flat_map(Path::ancestors),
        )
        .chain(cwd.ancestors().skip(1))
        .find(is_content)
        .unwrap_or(cwd)
        .to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_is_found_from_build_folder_portable_install_and_explicit_workspace() {
        let temp = std::env::temp_dir().join(format!("syfrack-paths-{}", std::process::id()));
        let checkout = temp.join("checkout");
        let portable = temp.join("portable");
        let unrelated = temp.join("unrelated");
        for root in [&checkout, &portable] {
            std::fs::create_dir_all(root.join("scenes")).unwrap();
            std::fs::create_dir_all(root.join("graphs")).unwrap();
        }
        std::fs::create_dir_all(&unrelated).unwrap();
        let build = checkout.join("target/debug");
        std::fs::create_dir_all(&build).unwrap();
        let exe = build.join("syfrack-studio.exe");
        assert_eq!(find_root(&build, Some(&exe)), checkout);
        assert_eq!(find_root(&unrelated, Some(&exe)), checkout);
        assert_eq!(find_root(&checkout.join("scenes"), None), checkout);
        let exe = portable.join("syfrack-studio.exe");
        assert_eq!(find_root(&unrelated, Some(&exe)), portable);
        assert_eq!(find_root(&checkout, Some(&exe)), checkout);
        assert_eq!(find_root(&unrelated, None), unrelated);
        std::fs::remove_dir_all(temp).unwrap();
    }
}
