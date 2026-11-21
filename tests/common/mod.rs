#![allow(dead_code)]
use agentsmd_lint::repo::Repo;
use agentsmd_lint::{discover, lint, Diagnostic, Options};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static N: AtomicUsize = AtomicUsize::new(0);

/// Временный репозиторий для теста; удаляется при уничтожении.
pub struct Fx {
    pub root: PathBuf,
}

impl Fx {
    pub fn new() -> Fx {
        let root = std::env::temp_dir().join(format!(
            "agentsmd-lint-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Fx { root }
    }

    pub fn write(&self, rel: &str, content: &str) -> &Fx {
        let p = self.root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, content).unwrap();
        self
    }

    pub fn lint(&self) -> Vec<Diagnostic> {
        self.lint_with(&Options::default())
    }

    pub fn lint_with(&self, opts: &Options) -> Vec<Diagnostic> {
        let repo = Repo::scan(&self.root).unwrap();
        let files = discover(&repo);
        lint(&repo, &files, opts)
    }
}

impl Drop for Fx {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub fn rules(d: &[Diagnostic]) -> Vec<&'static str> {
    d.iter().map(|x| x.rule).collect()
}

pub fn find<'a>(d: &'a [Diagnostic], rule: &str) -> Vec<&'a Diagnostic> {
    d.iter().filter(|x| x.rule == rule).collect()
}

pub fn count(d: &[Diagnostic], rule: &str) -> usize {
    find(d, rule).len()
}
