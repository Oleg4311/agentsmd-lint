//! Факты о репозитории, с которыми сверяются инструкции: какие файлы есть, какие скрипты, цели, версии.

use crate::json::{self, Json};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    ".venv",
    "venv",
    "__pycache__",
    ".tox",
    ".gradle",
    ".idea",
    ".vscode",
];
/// Каталоги и файлы, которые обычно порождает сборка или не лежат в git: упоминания о них не считаются ошибкой.
const GENERATED: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    "out",
    "coverage",
    ".next",
    ".nuxt",
    ".cache",
    "__pycache__",
    ".venv",
    "venv",
    ".turbo",
    ".parcel-cache",
];
const MAX_FILES: usize = 300_000;

#[derive(Debug, Default, Clone)]
pub struct Pkg {
    pub scripts: Vec<String>,
    pub package_manager: Option<String>,
    pub engines_node: Option<String>,
    pub has_workspaces: bool,
}

#[derive(Debug, Default)]
pub struct MakeFacts {
    pub targets: Vec<String>,
    pub has_pattern_rules: bool,
    pub has_include: bool,
}

pub struct Repo {
    pub root: PathBuf,
    /// Пути файлов относительно корня, с '/' в качестве разделителя.
    pub files: Vec<String>,
    names: HashMap<String, usize>,
    gitignore: Vec<String>,
}

/// Склеивает каталог и относительный путь, убирая `.` и `..`. None, если путь выходит за корень.
pub fn join_norm(base: &str, rel: &str) -> Option<String> {
    let mut parts: Vec<&str> = if rel.starts_with('/') {
        Vec::new()
    } else {
        base.split('/').filter(|s| !s.is_empty()).collect()
    };
    for c in rel.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            c => parts.push(c),
        }
    }
    Some(parts.join("/"))
}

pub fn parent_dir(path: &str) -> &str {
    path.rfind('/').map_or("", |i| &path[..i])
}

fn walk(dir: &Path, rel: &str, out: &mut Vec<String>, depth: usize) {
    if depth > 14 || out.len() >= MAX_FILES {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().to_string();
        let Ok(ft) = e.file_type() else { continue };
        let child = if rel.is_empty() {
            name.clone()
        } else {
            format!("{rel}/{name}")
        };
        if ft.is_dir() {
            if SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            walk(&e.path(), &child, out, depth + 1);
        } else if ft.is_file() {
            out.push(child);
        }
    }
}

impl Repo {
    pub fn scan(root: &Path) -> std::io::Result<Repo> {
        let root = root.canonicalize()?;
        let mut files = Vec::new();
        walk(&root, "", &mut files, 0);
        let mut names = HashMap::new();
        for f in &files {
            *names
                .entry(f.rsplit('/').next().unwrap_or(f).to_string())
                .or_insert(0) += 1;
        }
        let gitignore = fs::read_to_string(root.join(".gitignore"))
            .map(|s| {
                s.lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with('!'))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        Ok(Repo {
            root,
            files,
            names,
            gitignore,
        })
    }

    pub fn abs(&self, rel: &str) -> PathBuf {
        let mut p = self.root.clone();
        for c in rel.split('/').filter(|c| !c.is_empty()) {
            p.push(c);
        }
        p
    }

    pub fn exists(&self, rel: &str) -> bool {
        fs::metadata(self.abs(rel)).is_ok()
    }

    pub fn is_file(&self, rel: &str) -> bool {
        fs::metadata(self.abs(rel))
            .map(|m| m.is_file())
            .unwrap_or(false)
    }

    pub fn read(&self, rel: &str) -> Option<String> {
        fs::read_to_string(self.abs(rel)).ok()
    }

    pub fn name_exists(&self, name: &str) -> bool {
        self.names.contains_key(name)
    }

    /// Есть ли в дереве файл или каталог, оканчивающийся этим путём (для упоминаний вида `src/lib.rs` в монорепозитории).
    pub fn has_suffix(&self, path: &str) -> bool {
        let p = path.trim_end_matches('/');
        let last = p.rsplit('/').next().unwrap_or(p);
        if !path.ends_with('/') && !self.names.contains_key(last) {
            // путь к файлу: файла с таким именем нет нигде в дереве
            return false;
        }
        let dir_pre = format!("{p}/");
        let mid = format!("/{p}");
        let mid_dir = format!("/{p}/");
        self.files
            .iter()
            .any(|f| f == p || f.ends_with(&mid) || f.starts_with(&dir_pre) || f.contains(&mid_dir))
    }

    /// Упрощённая проверка .gitignore (имена, каталоги, `*.ext`, пути от корня) и каталогов сборки.
    pub fn ignored(&self, rel: &str) -> bool {
        let first = rel.split('/').next().unwrap_or("");
        if GENERATED.contains(&first) || rel.split('/').any(|c| c == "node_modules") {
            return true;
        }
        let last = rel.rsplit('/').next().unwrap_or(rel);
        if last == ".env"
            || (last.starts_with(".env.")
                && !last.ends_with(".example")
                && !last.ends_with(".sample")
                && !last.ends_with(".template"))
        {
            return true;
        }
        self.gitignore.iter().any(|pat| {
            if let Some(ext) = pat.strip_prefix("*.") {
                return rel.ends_with(&format!(".{ext}"));
            }
            if pat.contains('*') || pat.contains('?') || pat.contains('[') {
                return false;
            }
            let anchored = pat.starts_with('/') || pat.trim_end_matches('/').contains('/');
            let p = pat.trim_start_matches('/').trim_end_matches('/');
            if anchored {
                rel == p || rel.starts_with(&format!("{p}/"))
            } else {
                rel.split('/').any(|c| c == p)
            }
        })
    }

    /// Ближайший каталог (от `dir` вверх до корня), где лежит файл `name`.
    pub fn nearest(&self, dir: &str, name: &str) -> Option<String> {
        let mut d = dir.to_string();
        loop {
            let candidate = if d.is_empty() {
                name.to_string()
            } else {
                format!("{d}/{name}")
            };
            if self.is_file(&candidate) {
                return Some(d);
            }
            if d.is_empty() {
                return None;
            }
            d = parent_dir(&d).to_string();
        }
    }

    pub fn package_json(&self, dir: &str) -> Option<Pkg> {
        let path = if dir.is_empty() {
            "package.json".to_string()
        } else {
            format!("{dir}/package.json")
        };
        let j = json::parse(&self.read(&path)?).ok()?;
        let scripts = j
            .get("scripts")
            .and_then(Json::as_obj)
            .map(|o| o.iter().map(|(k, _)| k.clone()).collect())
            .unwrap_or_default();
        Some(Pkg {
            scripts,
            package_manager: j
                .get("packageManager")
                .and_then(Json::as_str)
                .map(str::to_string),
            engines_node: j
                .get("engines")
                .and_then(|e| e.get("node"))
                .and_then(Json::as_str)
                .map(str::to_string),
            has_workspaces: j.get("workspaces").is_some(),
        })
    }

    pub fn makefile(&self, dir: &str) -> Option<MakeFacts> {
        let name = ["Makefile", "makefile", "GNUmakefile"]
            .into_iter()
            .find(|n| self.is_file(&join_norm(dir, n).unwrap_or_default()))?;
        let text = self.read(&join_norm(dir, name)?)?;
        let mut f = MakeFacts::default();
        for line in text.lines() {
            if line.starts_with('\t') || line.starts_with(' ') || line.starts_with('#') {
                continue;
            }
            let t = line.trim();
            if t.starts_with("include ") || t.starts_with("-include ") || t.starts_with("sinclude ")
            {
                f.has_include = true;
                continue;
            }
            let Some(colon) = t.find(':') else { continue };
            if t[colon + 1..].starts_with('=') || t[colon + 1..].starts_with(":=") {
                continue; // присваивание переменной
            }
            let left = &t[..colon];
            if left.contains('=') {
                continue;
            }
            for w in left.split_whitespace() {
                if w.contains('%') {
                    f.has_pattern_rules = true;
                } else if !w.starts_with('.') && !w.contains('$') {
                    f.targets.push(w.to_string());
                }
            }
        }
        Some(f)
    }

    /// Рецепты justfile и признак того, что часть рецептов подключается через `mod`/`import` (тогда список неполон).
    pub fn justfile_recipes(&self, dir: &str) -> Option<(Vec<String>, bool)> {
        let name = ["justfile", "Justfile", ".justfile"]
            .into_iter()
            .find(|n| self.is_file(&join_norm(dir, n).unwrap_or_default()))?;
        let text = self.read(&join_norm(dir, name)?)?;
        let mut out = Vec::new();
        let mut partial = false;
        for line in text.lines() {
            if line.starts_with(' ')
                || line.starts_with('\t')
                || line.starts_with('#')
                || line.trim().is_empty()
            {
                continue;
            }
            let t = line.trim_start_matches('@');
            let word: String = t
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .collect();
            if word == "alias" {
                if let Some(name) = t["alias".len()..].split_whitespace().next() {
                    out.push(name.to_string());
                }
                continue;
            }
            if matches!(word.as_str(), "import" | "mod") {
                partial = true;
                continue;
            }
            if word.is_empty() || matches!(word.as_str(), "set" | "export" | "alias" | "default") {
                continue;
            }
            let rest = t[word.len()..].trim_start();
            // «name := value» — переменная; «name args: deps» — рецепт
            if rest.starts_with(":=") {
                continue;
            }
            if rest.contains(':') {
                out.push(word);
            }
        }
        Some((out, partial))
    }
}

/// Старший номер версии из `.nvmrc`/`.node-version`: «v20.11.0» → 20, «lts/*» → None.
pub fn leading_major(s: &str) -> Option<u32> {
    let t = s.trim().trim_start_matches('v');
    let digits: String = t.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// Минимальный старший номер среди частей диапазона (">=18.17 || ^20" → 18).
pub fn min_major_in_range(s: &str) -> Option<u32> {
    s.split(|c: char| c.is_whitespace() || c == ',' || c == '|')
        .filter_map(|tok| {
            let t = tok.trim_start_matches(|c: char| !c.is_ascii_digit());
            let d: String = t.chars().take_while(char::is_ascii_digit).collect();
            d.parse::<u32>().ok()
        })
        .min()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_norm_resolves_dots_and_escapes() {
        assert_eq!(join_norm("a/b", "../c.md").as_deref(), Some("a/c.md"));
        assert_eq!(join_norm("a", "./x/./y").as_deref(), Some("a/x/y"));
        assert_eq!(join_norm("a", "/root.md").as_deref(), Some("root.md"));
        assert_eq!(join_norm("a", "../../x"), None);
    }

    #[test]
    fn majors() {
        assert_eq!(leading_major("v20.11.0"), Some(20));
        assert_eq!(leading_major("lts/*"), None);
        assert_eq!(min_major_in_range(">=18.17 || ^20"), Some(18));
        assert_eq!(min_major_in_range(">=20"), Some(20));
        assert_eq!(min_major_in_range("*"), None);
    }
}
