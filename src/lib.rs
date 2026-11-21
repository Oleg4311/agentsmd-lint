//! agentsmd-lint: сверяет файлы-инструкции для ИИ-агентов (AGENTS.md, CLAUDE.md и др.) с реальным репозиторием.

pub mod exec;
pub mod json;
pub mod md;
pub mod repo;
pub mod report;
pub mod rules;
pub mod shell;

use repo::Repo;
use std::fs;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub rule: &'static str,
    pub severity: Severity,
    /// Путь к файлу-инструкции относительно корня репозитория.
    pub file: String,
    pub line: usize,
    pub col: usize,
    pub message: String,
    pub hint: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Options {
    /// Предел размера файла (с учётом импортов) в оценочных токенах (байты / 4).
    pub max_tokens: usize,
    pub ignore_rules: Vec<String>,
    pub ignore_paths: Vec<String>,
    /// Выполнять разрешённые команды из инструкций (проверка, что они действительно работают).
    pub exec: bool,
    /// Префиксы команд, которые разрешено выполнять; пусто — набор по умолчанию.
    pub exec_allow: Vec<String>,
    pub exec_timeout_secs: u64,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            max_tokens: 6000,
            ignore_rules: Vec::new(),
            ignore_paths: Vec::new(),
            exec: false,
            exec_allow: Vec::new(),
            exec_timeout_secs: 120,
        }
    }
}

/// Файл-инструкция, прочитанный и разобранный.
pub struct Instr {
    pub rel: String,
    pub dir: String,
    pub text: String,
    pub doc: md::Doc,
}

impl Instr {
    pub fn load(repo: &Repo, rel: &str) -> Option<Instr> {
        let bytes = fs::read(repo.abs(rel)).ok()?;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        Some(Instr {
            rel: rel.to_string(),
            dir: repo::parent_dir(rel).to_string(),
            doc: md::parse(&text),
            text,
        })
    }
}

const AGENT_NAMES: &[&str] = &[
    "agents.md",
    "claude.md",
    "gemini.md",
    "agent.md",
    "claude.local.md",
];

/// Находит файлы-инструкции: AGENTS.md, CLAUDE.md, GEMINI.md (в любом каталоге), правила Cursor, Copilot, Windsurf.
pub fn discover(repo: &Repo) -> Vec<String> {
    let mut out = Vec::new();
    for f in &repo.files {
        let name = f.rsplit('/').next().unwrap_or(f).to_lowercase();
        let hit = AGENT_NAMES.contains(&name.as_str())
            || f == ".github/copilot-instructions.md"
            || f == ".cursorrules"
            || f == ".windsurfrules"
            || (f.starts_with(".cursor/rules/") && (f.ends_with(".mdc") || f.ends_with(".md")))
            || (f.starts_with(".github/instructions/") && f.ends_with(".instructions.md"));
        if !hit {
            continue;
        }
        // символические ссылки (CLAUDE.md → AGENTS.md) не дублируем
        if fs::symlink_metadata(repo.abs(f))
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            continue;
        }
        out.push(f.clone());
    }
    out
}

/// Разбор комментариев `<!-- agentsmd-lint-disable[-next-line] правило, ... -->`.
fn suppressed(instr: &Instr, d: &Diagnostic) -> bool {
    let matches_rule = |list: &str| {
        let list = list.trim();
        list.is_empty() || list.split(',').any(|r| r.trim() == d.rule)
    };
    for (i, line) in instr.doc.lines.iter().enumerate() {
        let Some(start) = line.find("<!--") else {
            continue;
        };
        let Some(end) = line[start..].find("-->") else {
            continue;
        };
        let body = line[start + 4..start + end].trim();
        if let Some(rest) = body.strip_prefix("agentsmd-lint-disable-next-line") {
            if i + 2 == d.line && matches_rule(rest) {
                return true;
            }
        } else if let Some(rest) = body.strip_prefix("agentsmd-lint-disable") {
            if matches_rule(rest) {
                return true;
            }
        }
    }
    false
}

fn path_ignored(rel: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|p| {
        let p = p.trim_start_matches("./");
        if let Some(dir) = p.strip_suffix('/') {
            rel == dir || rel.starts_with(p)
        } else {
            rel == p
        }
    })
}

/// Оставляет файлы, не попавшие под `ignore_paths`.
pub fn select_files(files: Vec<String>, opts: &Options) -> Vec<String> {
    files
        .into_iter()
        .filter(|f| !path_ignored(f, &opts.ignore_paths))
        .collect()
}

pub fn lint(repo: &Repo, files: &[String], opts: &Options) -> Vec<Diagnostic> {
    let instrs: Vec<Instr> = files
        .iter()
        .filter(|f| !path_ignored(f, &opts.ignore_paths))
        .filter_map(|f| Instr::load(repo, f))
        .collect();
    let mut all = Vec::new();
    rules::run(repo, &instrs, opts, &mut all);
    if opts.exec {
        exec::run(repo, &instrs, opts, &mut all);
    }
    all.retain(|d| !opts.ignore_rules.iter().any(|r| r == d.rule));
    all.retain(|d| {
        !instrs
            .iter()
            .find(|i| i.rel == d.file)
            .is_some_and(|i| suppressed(i, d))
    });
    all.sort_by(|a, b| (&a.file, a.line, a.col, a.rule).cmp(&(&b.file, b.line, b.col, b.rule)));
    all.dedup();
    all
}
