//! Test helpers.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A temporary directory tree removed on drop.
pub(crate) struct Fixture {
    pub(crate) root: PathBuf,
}

impl Fixture {
    pub(crate) fn new(files: &[(&str, &str)]) -> Self {
        let id = NEXT.fetch_add(1, Ordering::SeqCst);
        let root =
            std::env::temp_dir().join(format!("nativeboat-project-{}-{id}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let root = nativeboat_bundler::resolve::realpath(&root);
        let fixture = Fixture { root };
        for (path, content) in files {
            fixture.write(path, content.as_bytes());
        }
        fixture
    }

    pub(crate) fn write(&self, path: &str, content: &[u8]) {
        let full = self.path(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, content).unwrap();
    }

    pub(crate) fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn junction(&self, link: &str, target: &str) {
        let output = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .raw_arg(format!("\"{}\"", self.path(link).display()))
            .raw_arg(format!("\"{}\"", self.path(target).display()))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
