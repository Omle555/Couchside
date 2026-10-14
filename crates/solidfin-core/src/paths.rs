//! Path safety helpers.

use std::path::{Component, Path, PathBuf};

/// Compares two path components; case-insensitive on Windows.
fn component_eq(a: Component<'_>, b: Component<'_>) -> bool {
    if cfg!(windows) {
        a.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
    } else {
        a == b
    }
}

/// Returns true if `candidate` is `root` or lies beneath it.
/// Both paths must already be canonical (absolute, no `..`).
pub fn starts_with_path(candidate: &Path, root: &Path) -> bool {
    let mut cand = candidate.components();
    for r in root.components() {
        match cand.next() {
            Some(c) if component_eq(c, r) => {}
            _ => return false,
        }
    }
    true
}

/// Resolves `candidate` on disk and returns it only if it lies inside one of `roots`.
///
/// Protects against path traversal (`..`), symlinks pointing outside a library
/// and stale database rows. Returns `None` if the file does not exist or is outside.
pub fn resolve_within<'a, I>(candidate: &Path, roots: I) -> Option<PathBuf>
where
    I: IntoIterator<Item = &'a Path>,
{
    let resolved = candidate.canonicalize().ok()?;
    roots
        .into_iter()
        .filter_map(|root| root.canonicalize().ok())
        .any(|root| starts_with_path(&resolved, &root))
        .then_some(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_matching_is_component_based() {
        assert!(starts_with_path(
            Path::new("/media/movies/a.mkv"),
            Path::new("/media/movies")
        ));
        assert!(!starts_with_path(
            Path::new("/media/movies2/a.mkv"),
            Path::new("/media/movies")
        ));
        assert!(!starts_with_path(
            Path::new("/media"),
            Path::new("/media/movies")
        ));
    }

    #[test]
    fn resolve_rejects_traversal() {
        let base = std::env::temp_dir().join(format!("solidfin-paths-{}", std::process::id()));
        let lib = base.join("lib");
        std::fs::create_dir_all(&lib).expect("create lib dir");
        let inside = lib.join("ok.mkv");
        let outside = base.join("secret.txt");
        std::fs::write(&inside, b"x").expect("write inside");
        std::fs::write(&outside, b"x").expect("write outside");

        assert!(resolve_within(&inside, [lib.as_path()]).is_some());
        assert!(resolve_within(&lib.join("..").join("secret.txt"), [lib.as_path()]).is_none());
        assert!(resolve_within(&lib.join("missing.mkv"), [lib.as_path()]).is_none());

        std::fs::remove_dir_all(&base).ok();
    }
}
