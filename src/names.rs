//! Entry names: one spelling, checked in both directions.
//!
//! Zip names are `/`-separated by specification, whatever the platform that
//! wrote them. [`normalize`] is what a path becomes on the way into a bundle;
//! [`check`] is what an entry has to survive on the way back out, because the
//! name in an archive is a claim made by whoever built it rather than something
//! this crate wrote.

use std::path::{Component, Path};

use crate::error::{Error, Result};

/// Names Windows will not take as a file name, in any directory, with or
/// without an extension — `CON.txt` is refused just as `CON` is.
///
/// Checked on every platform, not just Windows: the point is that a bundle
/// written here opens there, and where it was written does not change that.
const WINDOWS_RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Characters Windows forbids in a file name. `/` is absent deliberately — it
/// is the separator here, and is split on before this is checked.
const WINDOWS_FORBIDDEN: [char; 8] = ['<', '>', ':', '"', '\\', '|', '?', '*'];

/// Turns a relative path into the name it will carry inside a bundle:
/// `/`-separated, no `.` or `..`, nothing a platform cannot represent.
///
/// # Errors
///
/// [`Error::BadName`] for anything absolute, empty, containing `.` or `..`, or
/// naming something a platform cannot store.
pub fn normalize(path: &str) -> Result<String> {
    if path.is_empty() {
        return Err(Error::bad_name(path, "empty"));
    }

    // Windows separators are accepted on the way in and converted, so a caller
    // that built a path with `\` is not punished for it — but the stored form is
    // always `/`.
    let unified = path.replace('\\', "/");

    if unified.starts_with('/') {
        return Err(Error::bad_name(path, "absolute"));
    }

    if unified
        .split('/')
        .next()
        .is_some_and(|first| first.len() == 2 && first.ends_with(':'))
    {
        return Err(Error::bad_name(path, "has a drive letter"));
    }

    let mut parts = Vec::new();
    for part in unified.split('/') {
        match part {
            // A trailing or doubled separator. Dropping it makes `a//b.txt` and
            // `a/b.txt` the same entry rather than two.
            "" => continue,
            // Refused rather than resolved: resolving would mean the name asked
            // for and the name stored could differ, and that gap is where a
            // traversal lives.
            ".." => return Err(Error::bad_name(path, "contains `..`")),
            "." => return Err(Error::bad_name(path, "contains `.`")),
            _ => parts.push(check_part(path, part)?),
        }
    }

    if parts.is_empty() {
        return Err(Error::bad_name(path, "names nothing"));
    }

    Ok(parts.join("/"))
}

fn check_part<'a>(whole: &str, part: &'a str) -> Result<&'a str> {
    if let Some(bad) = part.chars().find(|c| WINDOWS_FORBIDDEN.contains(c)) {
        return Err(Error::bad_name(whole, format!("contains `{bad}`")));
    }

    // Control characters are legal on Unix and legal in zip. They are still
    // never what was meant, and a newline in a name makes every tool that prints
    // one lie about it.
    if let Some(control) = part.chars().find(|c| c.is_control()) {
        return Err(Error::bad_name(
            whole,
            format!("contains a control character (U+{:04X})", control as u32),
        ));
    }

    // Windows strips both silently when creating a file, so `report.` becomes
    // `report` — two entries differing only in a trailing dot would collide on
    // one platform and not another.
    if part.ends_with('.') || part.ends_with(' ') {
        return Err(Error::bad_name(whole, "ends with a dot or a space"));
    }

    let stem = part
        .split_once('.')
        .map_or(part, |(stem, _)| stem)
        .to_ascii_uppercase();
    if WINDOWS_RESERVED.contains(&stem.as_str()) {
        return Err(Error::bad_name(
            whole,
            format!("`{stem}` is reserved on Windows"),
        ));
    }

    Ok(part)
}

/// Whether an entry name from a bundle is safe to join onto a directory.
///
/// Stricter than [`normalize`] in intent if not in rules: a name being written
/// is ours to fix, but a name being read is someone else's claim, so it is
/// refused rather than repaired.
///
/// Returns the reason it was refused, or `Ok(())`.
pub fn check(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(Error::bad_name(name, "empty"));
    }

    for component in Path::new(name).components() {
        match component {
            Component::Normal(part) => {
                let Some(text) = part.to_str() else {
                    return Err(Error::bad_name(name, "is not valid UTF-8"));
                };
                if text.chars().any(|c| c.is_control()) {
                    return Err(Error::bad_name(name, "contains a control character"));
                }
            }
            Component::ParentDir => return Err(Error::bad_name(name, "contains `..`")),
            Component::RootDir | Component::Prefix(_) => {
                return Err(Error::bad_name(name, "is absolute"));
            }
            Component::CurDir => continue,
        }
    }

    // `components()` on a Unix build does not treat `\` as a separator, so a
    // name like `..\..\evil` would pass the loop above and then be interpreted
    // as a path by Windows. Checked explicitly rather than per-platform.
    if name.contains('\\') {
        return Err(Error::bad_name(name, "contains a backslash"));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_a_plain_path() {
        assert_eq!(normalize("textures/wood.png").unwrap(), "textures/wood.png");
    }

    #[test]
    fn unifies_separators_and_drops_empty_components() {
        assert_eq!(
            normalize(r"textures\wood.png").unwrap(),
            "textures/wood.png"
        );
        assert_eq!(
            normalize("textures//wood.png").unwrap(),
            "textures/wood.png"
        );
        assert_eq!(
            normalize("textures/wood.png/").unwrap(),
            "textures/wood.png"
        );
    }

    #[test]
    fn refuses_traversal_and_absolutes() {
        for path in [
            "../secrets",
            "textures/../../secrets",
            "/etc/passwd",
            r"C:\Windows\System32",
            "./here",
            "",
            "/",
        ] {
            assert!(normalize(path).is_err(), "{path} should have been refused");
        }
    }

    #[test]
    fn refuses_what_windows_cannot_store() {
        for path in ["CON", "com1.txt", "a<b", "trailing.", "trailing ", "a\nb"] {
            assert!(normalize(path).is_err(), "{path} should have been refused");
        }
    }

    #[test]
    fn check_refuses_escapes() {
        for name in ["../out", "/abs", "a/../../b", r"..\..\evil", ""] {
            assert!(check(name).is_err(), "{name} should have been refused");
        }
    }

    #[test]
    fn check_accepts_ordinary_names() {
        for name in ["assets/ok.txt", "./assets/ok.txt", "a/b/c/d.bin"] {
            assert!(check(name).is_ok(), "{name} should have been allowed");
        }
    }
}
