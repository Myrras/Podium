//! Path translation between Podium's Windows view of the world and the WSL
//! distro that runs its processes in WSL-backend mode.
//!
//! Podium keeps every path as a Windows `PathBuf` (a project opened as
//! `\\wsl.localhost\Ubuntu\home\me\app`, the app data dir under `C:\Users`);
//! whatever it hands to a process inside WSL (a `--cd`, a `git -C`, a config
//! path) must be the Linux spelling, and whatever comes back (e.g. `git
//! worktree list`) must be mapped back.
//!
//! Pure string work — no IO and no `cfg` — so it behaves (and is tested) the
//! same on every platform, which is why it parses the Windows syntax itself
//! instead of relying on `std::path` (which only understands it on Windows).

use std::path::{Path, PathBuf};

use crate::error::{CoreError, CoreResult};

/// UNC hosts under which Windows exposes a WSL distro's filesystem.
const WSL_HOSTS: [&str; 2] = ["wsl.localhost", "wsl$"];
/// Host used when building a UNC path (the modern spelling).
const WSL_HOST: &str = "wsl.localhost";

/// The Linux path (inside `distro`) for a Windows `path`.
///
/// - `\\wsl.localhost\<distro>\a\b`, `\\wsl$\<distro>\a\b` and the verbatim
///   `\\?\UNC\wsl.localhost\<distro>\a\b` (what `canonicalize` returns) →
///   `/a/b`;
/// - `C:\x\y` (and verbatim `\\?\C:\x\y`) → `/mnt/c/x/y`.
///
/// Host and distro match case-insensitively; `/` and `\` are both accepted
/// as separators; empty and `.` segments (trailing or doubled separators)
/// are dropped. A path under another distro, another UNC host, or a
/// relative path is an error.
pub fn to_linux(distro: &str, path: &Path) -> CoreResult<String> {
    let raw = path
        .to_str()
        .ok_or_else(|| invalid(path, "is not valid Unicode"))?;
    let normalized = raw.replace('/', "\\");

    // Verbatim prefixes: `\\?\UNC\host\share\…` is `\\host\share\…`, and
    // `\\?\C:\…` is `C:\…`.
    let rest = if let Some(unc) = strip_prefix_ci(&normalized, r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else if let Some(local) = normalized.strip_prefix(r"\\?\") {
        local.to_string()
    } else {
        normalized
    };

    if let Some(unc) = rest.strip_prefix(r"\\") {
        let mut parts = unc.split('\\');
        let host = parts.next().unwrap_or_default();
        if !WSL_HOSTS.iter().any(|h| h.eq_ignore_ascii_case(host)) {
            return Err(invalid(path, "is a network path, not a WSL path"));
        }
        let share = parts.next().unwrap_or_default();
        if share.is_empty() {
            return Err(invalid(path, "names no WSL distro"));
        }
        if !share.eq_ignore_ascii_case(distro) {
            return Err(invalid(
                path,
                &format!("is in WSL distro `{share}`, but Podium's backend is `{distro}`"),
            ));
        }
        return Ok(join_linux("", parts));
    }

    let bytes = rest.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        let after = &rest[2..];
        if !(after.is_empty() || after.starts_with('\\')) {
            return Err(invalid(path, "is drive-relative, not absolute"));
        }
        let drive = (bytes[0] as char).to_ascii_lowercase();
        return Ok(join_linux(&format!("/mnt/{drive}"), after.split('\\')));
    }

    Err(invalid(path, "is not an absolute Windows path"))
}

/// The Windows path for a Linux `path` inside `distro`: `/mnt/c/x` →
/// `C:\x`, anything else absolute → `\\wsl.localhost\<distro>\…`. Empty and
/// `.` segments are dropped. A relative path only has its separators
/// flipped (there is nothing to anchor it to).
pub fn to_windows(distro: &str, path: &str) -> PathBuf {
    let Some(abs) = path.strip_prefix('/') else {
        return PathBuf::from(path.replace('/', "\\"));
    };
    let segments: Vec<&str> = abs
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect();

    if let [mnt, drive, rest @ ..] = segments.as_slice() {
        if *mnt == "mnt" && drive.len() == 1 && drive.as_bytes()[0].is_ascii_alphabetic() {
            let drive = drive.to_ascii_uppercase();
            return PathBuf::from(format!(r"{drive}:\{}", rest.join("\\")));
        }
    }

    PathBuf::from(format!(r"\\{WSL_HOST}\{distro}\{}", segments.join("\\")))
}

/// `prefix` + `/segment` for every non-empty, non-`.` segment; `/` if none.
fn join_linux<'a>(prefix: &str, segments: impl Iterator<Item = &'a str>) -> String {
    let mut out = prefix.to_string();
    for segment in segments.filter(|s| !s.is_empty() && *s != ".") {
        out.push('/');
        out.push_str(segment);
    }
    if out.is_empty() {
        out.push('/');
    }
    out
}

/// `s` without `prefix`, matched ASCII-case-insensitively.
fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &s[prefix.len()..])
}

fn invalid(path: &Path, why: &str) -> CoreError {
    CoreError::InvalidInput(format!("path `{}` {why}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linux(path: &str) -> CoreResult<String> {
        to_linux("Ubuntu", Path::new(path))
    }

    fn windows(path: &str) -> String {
        to_windows("Ubuntu", path).to_string_lossy().into_owned()
    }

    #[test]
    fn unc_paths_map_into_the_distro() {
        assert_eq!(
            linux(r"\\wsl.localhost\Ubuntu\home\me\app").unwrap(),
            "/home/me/app"
        );
        assert_eq!(linux(r"\\wsl$\Ubuntu\home\me\app").unwrap(), "/home/me/app");
        assert_eq!(
            linux(r"\\?\UNC\wsl.localhost\Ubuntu\home\me\app").unwrap(),
            "/home/me/app"
        );
        assert_eq!(linux(r"\\wsl.localhost\Ubuntu").unwrap(), "/");
        assert_eq!(linux(r"\\wsl.localhost\Ubuntu\").unwrap(), "/");
    }

    #[test]
    fn host_and_distro_match_case_insensitively() {
        assert_eq!(linux(r"\\WSL.LOCALHOST\ubuntu\etc").unwrap(), "/etc");
        assert_eq!(linux(r"\\WSL$\UBUNTU\etc").unwrap(), "/etc");
        assert_eq!(linux(r"\\?\unc\Wsl.Localhost\Ubuntu\etc").unwrap(), "/etc");
    }

    #[test]
    fn forward_slashes_are_accepted() {
        assert_eq!(linux("//wsl.localhost/Ubuntu/home/me").unwrap(), "/home/me");
        assert_eq!(linux("C:/Users/me").unwrap(), "/mnt/c/Users/me");
    }

    #[test]
    fn drive_paths_map_under_mnt_with_a_lowercase_drive() {
        assert_eq!(
            linux(r"C:\Users\me\AppData").unwrap(),
            "/mnt/c/Users/me/AppData"
        );
        assert_eq!(linux(r"d:\work").unwrap(), "/mnt/d/work");
        assert_eq!(linux(r"\\?\C:\Users\me").unwrap(), "/mnt/c/Users/me");
        assert_eq!(linux(r"C:\").unwrap(), "/mnt/c");
        assert_eq!(linux("C:").unwrap(), "/mnt/c");
    }

    #[test]
    fn trailing_and_doubled_separators_are_dropped() {
        assert_eq!(
            linux(r"\\wsl.localhost\Ubuntu\home\me\\app\").unwrap(),
            "/home/me/app"
        );
        assert_eq!(linux(r"C:\Users\.\me\").unwrap(), "/mnt/c/Users/me");
    }

    #[test]
    fn spaces_and_unicode_survive() {
        assert_eq!(
            linux(r"\\wsl.localhost\Ubuntu\home\me\mijn project\café ☕").unwrap(),
            "/home/me/mijn project/café ☕"
        );
        assert_eq!(
            linux(r"C:\Program Files\Pödium").unwrap(),
            "/mnt/c/Program Files/Pödium"
        );
    }

    #[test]
    fn another_distro_is_a_clear_error() {
        let err = linux(r"\\wsl.localhost\Debian\home\me").unwrap_err();
        assert!(matches!(err, CoreError::InvalidInput(_)));
        let msg = err.to_string();
        assert!(msg.contains("Debian") && msg.contains("Ubuntu"), "{msg}");
    }

    #[test]
    fn non_wsl_and_relative_paths_are_errors() {
        for path in [
            r"\\fileserver\share\x",
            r"\\wsl.localhost",
            r"relative\path",
            r"\rooted\no\drive",
            r"C:relative",
            "/home/me",
        ] {
            assert!(
                matches!(linux(path), Err(CoreError::InvalidInput(_))),
                "{path} should be rejected"
            );
        }
    }

    #[test]
    fn linux_paths_map_back_to_windows() {
        assert_eq!(
            windows("/home/me/app"),
            r"\\wsl.localhost\Ubuntu\home\me\app"
        );
        assert_eq!(windows("/"), r"\\wsl.localhost\Ubuntu\");
        assert_eq!(windows("/mnt/c/Users/me"), r"C:\Users\me");
        assert_eq!(windows("/mnt/d"), r"D:\");
        assert_eq!(windows("/mnt/c/"), r"C:\");
        // `/mnt/<not a drive letter>` is an ordinary directory in the distro.
        assert_eq!(windows("/mnt/wsl/x"), r"\\wsl.localhost\Ubuntu\mnt\wsl\x");
        assert_eq!(
            windows("/home/me//app/"),
            r"\\wsl.localhost\Ubuntu\home\me\app"
        );
        assert_eq!(
            windows("/home/me/mijn project/café"),
            r"\\wsl.localhost\Ubuntu\home\me\mijn project\café"
        );
        assert_eq!(windows("relative/path"), r"relative\path");
    }

    #[test]
    fn round_trips() {
        for path in [
            r"\\wsl.localhost\Ubuntu\home\me\mijn project",
            r"C:\Users\me\AppData\Roaming\com.podium.app",
        ] {
            let there = linux(path).unwrap();
            assert_eq!(windows(&there), path);
        }
        for path in ["/home/me/app", "/mnt/c/Users/me"] {
            let back = to_windows("Ubuntu", path);
            assert_eq!(to_linux("Ubuntu", &back).unwrap(), path);
        }
    }
}
