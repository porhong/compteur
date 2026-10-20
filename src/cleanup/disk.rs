//! Finds and removes files that are safe to lose. Never follows links and never forces a delete:
//! a file that is in use, read-only or protected is simply kept.

use std::{
    collections::HashSet,
    fs,
    ops::AddAssign,
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

/// Temp files younger than this may belong to a running installer or app, so they stay.
pub const TEMP_MIN_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// Totals of one sweep.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tally {
    pub bytes: u64,
    pub files: u64,
    /// Files that can't be deleted: held open by an app, or protected by their permissions.
    pub kept: u64,
}

impl AddAssign for Tally {
    fn add_assign(&mut self, other: Self) {
        self.bytes += other.bytes;
        self.files += other.files;
        self.kept += other.kept;
    }
}

/// Adds up the files under `root` last changed at least `min_age` ago and, when `delete` is set,
/// removes them along with folders left empty. `root` itself is always kept.
pub fn sweep(root: &Path, min_age: Duration, delete: bool) -> Tally {
    let mut tally = Tally::default();
    if is_safe_root(root) {
        sweep_dir(root, min_age, delete, SystemTime::now(), &mut tally);
    }
    tally
}

fn sweep_dir(dir: &Path, min_age: Duration, delete: bool, now: SystemTime, tally: &mut Tally) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = fs::symlink_metadata(&path) else { continue };
        if is_link(&meta) {
            continue; // a junction or symlink can point anywhere, even outside the root
        }
        // Read before sweeping a folder, since deleting its contents updates its timestamp.
        let old_enough = meta.modified().is_ok_and(|at| now.duration_since(at).is_ok_and(|age| age >= min_age));
        if meta.is_dir() {
            sweep_dir(&path, min_age, delete, now, tally);
            if delete && old_enough {
                let _ = fs::remove_dir(&path); // only succeeds once the folder is empty
            }
        } else if old_enough {
            let removed = if delete { fs::remove_file(&path).is_ok() } else { can_delete(&path) };
            if removed {
                tally.bytes += meta.len();
                tally.files += 1;
            } else {
                tally.kept += 1;
            }
        }
    }
}

/// Whether deleting `path` would succeed, without deleting it: asks for delete access while
/// letting others keep the file open. Fails for files an app holds open without allowing deletion
/// and for files whose permissions forbid it, so a scan only counts what a clean can free.
fn can_delete(path: &Path) -> bool {
    const DELETE: u32 = 0x0001_0000;
    const SHARE_ALL: u32 = 0x7; // read, write and delete
    fs::OpenOptions::new().access_mode(DELETE).share_mode(SHARE_ALL).open(path).is_ok()
}

fn is_link(meta: &fs::Metadata) -> bool {
    meta.file_type().is_symlink() || meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

/// Refuses anything that isn't a real folder at least four levels deep (like
/// `C:\Users\name\AppData\Local\Temp`), so a wrong path can never sweep a drive or a whole profile.
pub fn is_safe_root(root: &Path) -> bool {
    root.is_absolute()
        && root.components().count() >= 5
        && fs::symlink_metadata(root).is_ok_and(|meta| meta.is_dir() && !is_link(&meta))
}

fn local_app_data() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
}

pub fn temp_roots() -> Vec<PathBuf> {
    vec![std::env::temp_dir()]
}

pub fn error_report_roots() -> Vec<PathBuf> {
    let Some(local) = local_app_data() else { return Vec::new() };
    vec![local.join(r"Microsoft\Windows\WER"), local.join("CrashDumps")]
}

/// DirectX and GPU-driver shader caches; drivers rebuild them as games and apps need them.
pub fn shader_cache_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(local) = local_app_data() {
        for dir in [
            "D3DSCache",
            r"AMD\DxCache",
            r"AMD\DxcCache",
            r"AMD\GLCache",
            r"AMD\VkCache",
            r"NVIDIA\DXCache",
            r"NVIDIA\GLCache",
        ] {
            roots.push(local.join(dir));
        }
    }
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        roots.push(PathBuf::from(profile).join(r"AppData\LocalLow\Intel\ShaderCache"));
    }
    roots
}

pub struct Browser {
    pub name: &'static str,
    /// Executable name, to tell whether the browser is running.
    process: &'static str,
    /// Folder under `%LOCALAPPDATA%` holding the profiles.
    profiles: &'static str,
    /// Cache folders inside each profile.
    caches: &'static [&'static str],
}

const CHROMIUM_CACHES: &[&str] = &["Cache", "Code Cache", "GPUCache"];

pub const BROWSERS: [Browser; 5] = [
    Browser { name: "Chrome", process: "chrome.exe", profiles: r"Google\Chrome\User Data", caches: CHROMIUM_CACHES },
    Browser { name: "Edge", process: "msedge.exe", profiles: r"Microsoft\Edge\User Data", caches: CHROMIUM_CACHES },
    Browser {
        name: "Brave",
        process: "brave.exe",
        profiles: r"BraveSoftware\Brave-Browser\User Data",
        caches: CHROMIUM_CACHES,
    },
    Browser { name: "Vivaldi", process: "vivaldi.exe", profiles: r"Vivaldi\User Data", caches: CHROMIUM_CACHES },
    Browser { name: "Firefox", process: "firefox.exe", profiles: r"Mozilla\Firefox\Profiles", caches: &["cache2"] },
];

impl Browser {
    /// The cache folders of every profile; only caches, never history, cookies or logins.
    pub fn cache_roots(&self) -> Vec<PathBuf> {
        let Some(profiles) = local_app_data().map(|local| local.join(self.profiles)) else { return Vec::new() };
        let Ok(entries) = fs::read_dir(profiles) else { return Vec::new() };
        entries
            .flatten()
            .map(|entry| entry.path())
            .flat_map(|profile| self.caches.iter().map(move |cache| profile.join(cache)))
            .filter(|cache| cache.is_dir())
            .collect()
    }

    pub fn is_running(&self, running: &HashSet<String>) -> bool {
        running.contains(self.process)
    }

    pub fn is_installed(&self) -> bool {
        local_app_data().is_some_and(|local| local.join(self.profiles).is_dir())
    }
}

/// Lower-case executable names of all running processes.
pub fn running_processes() -> HashSet<String> {
    let mut sys = System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    sys.processes().values().map(|p| p.name().to_string_lossy().to_lowercase()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE_SHARE_READ: u32 = 1;
    const DAY: Duration = Duration::from_secs(24 * 60 * 60);

    /// A fresh folder under the system temp folder, removed again on drop.
    struct TestDir(PathBuf);

    impl TestDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("compteur-test-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn file(&self, relative: &str, bytes: usize, age: Duration) -> PathBuf {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, vec![0u8; bytes]).unwrap();
            let file = fs::File::options().write(true).open(&path).unwrap();
            file.set_modified(SystemTime::now() - age).unwrap();
            path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn only_old_files_are_counted_and_deleted() {
        let dir = TestDir::new("age");
        let old = dir.file("old.tmp", 100, 2 * DAY);
        let nested = dir.file(r"sub\old.tmp", 50, 2 * DAY);
        let new = dir.file("new.tmp", 10, Duration::ZERO);

        let scan = sweep(&dir.0, DAY, false);
        assert_eq!(scan, Tally { bytes: 150, files: 2, kept: 0 });
        assert!(old.exists(), "a scan deletes nothing");

        assert_eq!(sweep(&dir.0, DAY, true), scan, "a clean frees what the scan found");
        assert!(!old.exists() && !nested.exists());
        assert!(new.exists(), "recent files are kept");
        assert!(dir.0.exists(), "the root itself is kept");
    }

    #[test]
    fn emptied_folders_are_removed() {
        let dir = TestDir::new("folders");
        dir.file(r"a\b\c.tmp", 1, Duration::ZERO);
        sweep(&dir.0, Duration::ZERO, true);
        assert!(!dir.0.join("a").exists());
        assert!(dir.0.exists());
    }

    #[test]
    fn files_in_use_are_kept_and_not_counted() {
        let dir = TestDir::new("in-use");
        let path = dir.file("locked.tmp", 5, 2 * DAY);
        // Windows refuses to delete a file opened without delete sharing, like an app holding it.
        let _held = fs::File::options().read(true).share_mode(FILE_SHARE_READ).open(&path).unwrap();
        let kept = Tally { bytes: 0, files: 0, kept: 1 };
        assert_eq!(sweep(&dir.0, DAY, false), kept, "the scan leaves it out of the estimate");
        assert_eq!(sweep(&dir.0, DAY, true), kept);
        assert!(path.exists());
    }

    #[test]
    fn links_are_not_followed() {
        let outside = TestDir::new("link-target");
        let target = outside.file("precious.txt", 5, 2 * DAY);
        let dir = TestDir::new("link");
        // Creating a symlink needs Developer Mode or admin; without it there is nothing to test.
        if std::os::windows::fs::symlink_dir(&outside.0, dir.0.join("link")).is_err() {
            return;
        }
        sweep(&dir.0, Duration::ZERO, true);
        assert!(target.exists());
    }

    #[test]
    fn unsafe_roots_are_refused() {
        assert!(!is_safe_root(Path::new("")));
        assert!(!is_safe_root(Path::new(r"C:\")));
        assert!(!is_safe_root(Path::new(r"C:\Users")));
        assert!(!is_safe_root(Path::new(r"relative\path\deep\enough\folder")));
        let dir = TestDir::new("root");
        assert!(is_safe_root(&dir.0));
    }
}
