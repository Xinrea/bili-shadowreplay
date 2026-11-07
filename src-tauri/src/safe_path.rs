//! Paths taken from requests, kept inside the directory they are served from.

use std::path::{Component, Path, PathBuf};

/// `path`, `/`-separated and relative, joined onto `base`.
///
/// Every segment must be a single plain file name: `..`, `.`, empty segments,
/// absolute paths and Windows prefixes or `\` separators are rejected, so the
/// result can never escape `base`. Names made only of dots and spaces (`.. `,
/// `...`) are rejected too, since Windows trims trailing dots and spaces and
/// could read them as `..`.
pub(crate) fn join_relative(base: &Path, path: &str) -> Option<PathBuf> {
    let mut resolved = base.to_path_buf();
    for segment in path.split('/') {
        let mut components = Path::new(segment).components();
        match (components.next(), components.next()) {
            (Some(Component::Normal(name)), None)
                if !name
                    .to_string_lossy()
                    .trim_end_matches([' ', '.'])
                    .is_empty() =>
            {
                resolved.push(name)
            }
            _ => return None,
        }
    }
    Some(resolved)
}
