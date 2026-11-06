//! Shared pieces of the embedded file servers (TFTP P5-T02, FTP and SCP P5-T03): one
//! root folder, nothing outside it (no `..`, no absolute paths, no symlink escapes),
//! uploads optional and never overwriting unless allowed, transfer events for the UI.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;

pub struct Config {
    pub root: PathBuf,
    pub allow_upload: bool,
    pub overwrite: bool,
}

/// Login for the FTP / SCP servers. Lives only in memory while the server runs.
pub struct Credentials {
    pub user: String,
    pub password: String,
}

impl Credentials {
    /// Constant-time comparison, so a wrong guess does not reveal how much matched.
    pub fn check(&self, user: &str, password: &str) -> bool {
        let eq = |a: &[u8], b: &[u8]| {
            a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
        };
        // Evaluate both so timing does not show which one was wrong.
        let u = eq(user.as_bytes(), self.user.as_bytes());
        let p = eq(password.as_bytes(), self.password.as_bytes());
        u & p
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Event {
    Started {
        id: u64,
        peer: String,
        file: String,
        upload: bool,
        size: Option<u64>,
    },
    Progress {
        id: u64,
        bytes: u64,
    },
    Done {
        id: u64,
        bytes: u64,
    },
    Failed {
        id: u64,
        error: String,
    },
}

pub type Events = Arc<dyn Fn(Event) + Send + Sync>;

/// Transfer ids for one server.
#[derive(Default)]
pub struct Ids(std::sync::atomic::AtomicU64);

impl Ids {
    pub fn next(&self) -> u64 {
        self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1
    }
}

/// `root` + a client file name, or None when it would leave the root or names the
/// root itself.
pub fn resolve(root: &Path, name: &str) -> Option<PathBuf> {
    let rel = normalize("/", name)?;
    (rel != "/").then(|| join(root, &rel))
}

/// A client path (absolute from the root, or relative to `cwd`) as a clean virtual path
/// (`/a/b`, `/` for the root). `..` is followed but never above the root.
pub fn normalize(cwd: &str, name: &str) -> Option<String> {
    let name = name.replace('\\', "/");
    let mut parts: Vec<String> = if name.starts_with('/') {
        Vec::new()
    } else {
        cwd.split('/')
            .filter(|p| !p.is_empty())
            .map(str::to_owned)
            .collect()
    };
    for c in Path::new(&name).components() {
        match c {
            Component::Normal(p) => parts.push(p.to_str()?.to_owned()),
            Component::CurDir | Component::RootDir => {}
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::Prefix(_) => return None,
        }
    }
    Some(format!("/{}", parts.join("/")))
}

/// The file system path of a virtual path from [`normalize`].
pub fn join(root: &Path, virt: &str) -> PathBuf {
    let mut out = root.to_path_buf();
    for p in virt.split('/').filter(|p| !p.is_empty()) {
        out.push(p);
    }
    out
}

/// True when `path` (or, for a file not yet created, its nearest existing ancestor)
/// really lies inside `root` once symlinks are followed.
pub fn confined(root: &Path, path: &Path) -> bool {
    let Ok(root) = root.canonicalize() else {
        return false;
    };
    let mut probe = path;
    loop {
        if let Ok(real) = probe.canonicalize() {
            return real.starts_with(&root);
        }
        match probe.parent() {
            Some(p) => probe = p,
            None => return false,
        }
    }
}

/// Uploads are written here and renamed into place once complete.
pub fn part_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

/// Why an upload is refused, if it is.
pub async fn upload_refusal(cfg: &Config, path: &Path) -> Option<&'static str> {
    if !cfg.allow_upload {
        return Some("uploads are off");
    }
    if !confined(&cfg.root, path) {
        return Some("outside the shared folder");
    }
    if !cfg.overwrite && tokio::fs::metadata(path).await.is_ok() {
        return Some("file exists (overwrite is off)");
    }
    None
}

/// A random alphanumeric password for a new server.
pub fn random_password(len: usize) -> String {
    use ring::rand::SecureRandom;
    const CHARS: &[u8] = b"abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut buf = vec![0u8; len];
    if ring::rand::SystemRandom::new().fill(&mut buf).is_err() {
        // Practically unreachable; fall back to clock bits rather than an empty password.
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        for (i, b) in buf.iter_mut().enumerate() {
            *b = (t >> ((i % 16) * 8)) as u8;
        }
    }
    buf.iter()
        .map(|b| CHARS[usize::from(*b) % CHARS.len()] as char)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_stay_inside_the_root() {
        let root = Path::new("/srv");
        assert_eq!(resolve(root, "a.bin"), Some(PathBuf::from("/srv/a.bin")));
        assert_eq!(resolve(root, "/x/../b"), Some(PathBuf::from("/srv/b")));
        assert_eq!(resolve(root, "../etc/passwd"), None);
        assert_eq!(resolve(root, "/"), None);
        assert_eq!(normalize("/a/b", "..").as_deref(), Some("/a"));
        assert_eq!(normalize("/a", "c/./d").as_deref(), Some("/a/c/d"));
        assert_eq!(normalize("/a", "/z").as_deref(), Some("/z"));
        assert_eq!(normalize("/", "..").as_deref(), None);
        assert_eq!(normalize("/", "dir\\f").as_deref(), Some("/dir/f"));
    }

    #[test]
    fn credentials_and_passwords() {
        let c = Credentials {
            user: "kenshell".into(),
            password: "s3cret".into(),
        };
        assert!(c.check("kenshell", "s3cret"));
        assert!(!c.check("kenshell", "s3cre"));
        assert!(!c.check("root", "s3cret"));
        let p = random_password(12);
        assert_eq!(p.len(), 12);
        assert!(p.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_ne!(p, random_password(12));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_is_caught() {
        let dir = std::env::temp_dir().join(format!("ks-served-{}", std::process::id()));
        let root = dir.join("root");
        std::fs::create_dir_all(&root).unwrap();
        std::os::unix::fs::symlink("/", root.join("out")).unwrap();
        assert!(confined(&root, &root.join("new.bin")));
        assert!(!confined(&root, &root.join("out/etc")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
