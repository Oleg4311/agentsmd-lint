//! Правила. Каждое проверяет утверждение из инструкции фактом из репозитория (там, где это возможно)
//! либо ловит очевидный вред (секреты, противоречия, лишний вес).

use crate::repo::{join_norm, leading_major, min_major_in_range, parent_dir, Repo};
use crate::shell::{self, Cmd};
use crate::{Diagnostic, Instr, Options, Severity};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

pub struct RuleInfo {
    pub id: &'static str,
    pub severity: Severity,
    pub summary: &'static str,
}

pub const RULES: &[RuleInfo] = &[
    RuleInfo {
        id: "dead-link",
        severity: Severity::Error,
        summary: "ссылка на локальный файл, которого нет",
    },
    RuleInfo {
        id: "link-root-relative",
        severity: Severity::Warning,
        summary: "ссылка записана от корня репозитория, но считается от каталога файла и на GitHub сломана",
    },
    RuleInfo {
        id: "import-missing",
        severity: Severity::Error,
        summary: "импорт @файл указывает на несуществующий файл",
    },
    RuleInfo {
        id: "import-cycle",
        severity: Severity::Error,
        summary: "файлы подключают друг друга по кругу",
    },
    RuleInfo {
        id: "import-depth",
        severity: Severity::Warning,
        summary: "цепочка импортов глубже 5 уровней: агент её не раскроет",
    },
    RuleInfo {
        id: "broken-path",
        severity: Severity::Warning,
        summary: "упомянутого пути или файла нет в репозитории",
    },
    RuleInfo {
        id: "missing-script",
        severity: Severity::Warning,
        summary: "команда ссылается на скрипт или цель, которых нет (npm, make, just)",
    },
    RuleInfo {
        id: "no-manifest",
        severity: Severity::Warning,
        summary: "команда требует package.json, Makefile, Cargo.toml или go.mod, а их нет",
    },
    RuleInfo {
        id: "pm-lockfile-mismatch",
        severity: Severity::Warning,
        summary: "менеджер пакетов в инструкции не совпадает с lockfile репозитория",
    },
    RuleInfo {
        id: "pm-field-mismatch",
        severity: Severity::Warning,
        summary: "менеджер пакетов не совпадает с полем packageManager в package.json",
    },
    RuleInfo {
        id: "pm-inconsistent",
        severity: Severity::Warning,
        summary: "в одном файле используются разные менеджеры пакетов",
    },
    RuleInfo {
        id: "stale-version",
        severity: Severity::Warning,
        summary: "версия Node, Go или Python в тексте ниже требуемой проектом",
    },
    RuleInfo {
        id: "too-long",
        severity: Severity::Warning,
        summary: "файл (с импортами) слишком велик для постоянного контекста агента",
    },
    RuleInfo {
        id: "secret",
        severity: Severity::Error,
        summary: "в файле похоже на секрет (ключ, токен, приватный ключ)",
    },
    RuleInfo {
        id: "contradiction",
        severity: Severity::Warning,
        summary: "возможное противоречие между указаниями",
    },
    RuleInfo {
        id: "vague-instruction",
        severity: Severity::Info,
        summary: "расплывчатое указание без проверяемого действия",
    },
    RuleInfo {
        id: "duplicate-line",
        severity: Severity::Info,
        summary: "повтор строки в файле",
    },
    RuleInfo {
        id: "duplicate-files",
        severity: Severity::Info,
        summary: "несколько файлов-инструкций дублируют друг друга",
    },
    RuleInfo {
        id: "no-commands",
        severity: Severity::Info,
        summary: "в инструкциях нет команд сборки и проверки",
    },
    RuleInfo {
        id: "command-failed",
        severity: Severity::Error,
        summary: "команда из инструкции завершилась ошибкой (режим --exec)",
    },
    RuleInfo {
        id: "command-timeout",
        severity: Severity::Warning,
        summary: "команда из инструкции не уложилась во время (режим --exec)",
    },
];

pub fn info(id: &str) -> Option<&'static RuleInfo> {
    RULES.iter().find(|r| r.id == id)
}

struct Sink<'a> {
    out: &'a mut Vec<Diagnostic>,
    file: String,
}

impl Sink<'_> {
    fn add(
        &mut self,
        rule: &'static str,
        line: usize,
        col: usize,
        message: String,
        hint: Option<&str>,
    ) {
        let severity = info(rule).map_or(Severity::Warning, |r| r.severity);
        self.out.push(Diagnostic {
            rule,
            severity,
            file: self.file.clone(),
            line,
            col,
            message,
            hint: hint.map(str::to_string),
        });
    }
}

pub fn run(repo: &Repo, instrs: &[Instr], opts: &Options, out: &mut Vec<Diagnostic>) {
    let by_rel: HashMap<&str, &Instr> = instrs.iter().map(|i| (i.rel.as_str(), i)).collect();
    for f in instrs {
        let mut s = Sink {
            out,
            file: f.rel.clone(),
        };
        links(repo, f, &mut s);
        paths(repo, f, &mut s);
        secrets(f, &mut s);
        vague(f, &mut s);
        duplicates(f, &mut s);
        contradictions(f, &mut s);
        let cmds = shell::extract(&f.doc, &f.dir);
        commands(repo, f, &cmds, &mut s);
        package_managers(repo, f, &cmds, &mut s);
        versions(repo, f, &mut s);
        no_commands(f, &cmds, &mut s);
        imports(repo, f, &by_rel, opts, &mut s);
    }
    duplicate_files(instrs, out);
}

// ---------- ссылки ----------

fn is_external(t: &str) -> bool {
    t.contains("://")
        || t.starts_with("mailto:")
        || t.starts_with("tel:")
        || t.starts_with("data:")
        || t.starts_with('#')
        || t.starts_with("//")
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn links(repo: &Repo, f: &Instr, s: &mut Sink) {
    for l in &f.doc.links {
        if is_external(&l.target) {
            continue;
        }
        let t = l.target.split(['#', '?']).next().unwrap_or("");
        if t.is_empty() {
            continue;
        }
        let t = percent_decode(t);
        let Some(resolved) = join_norm(&f.dir, &t) else {
            continue;
        };
        if repo.exists(&resolved) || repo.ignored(&resolved) {
            continue;
        }
        if !t.starts_with('/') && !f.dir.is_empty() {
            if let Some(from_root) = join_norm("", &t) {
                if repo.exists(&from_root) {
                    s.add("link-root-relative", l.line, l.col, format!("ссылка `{t}` считается от каталога файла (`{resolved}`), а такой файл лежит от корня"), Some("на GitHub ссылка сломана; чтобы указать путь от корня, начните его со слэша: `/{t}`"));
                    continue;
                }
            }
        }
        s.add(
            "dead-link",
            l.line,
            l.col,
            format!("ссылка ведёт на `{}`, но такого файла нет", resolved),
            Some("поправьте путь или удалите ссылку: агент перейдёт по ней и ничего не найдёт"),
        );
    }
}

// ---------- пути ----------

const FILE_EXTS: &[&str] = &[
    "json",
    "toml",
    "yaml",
    "yml",
    "md",
    "mdx",
    "rs",
    "go",
    "ts",
    "tsx",
    "js",
    "jsx",
    "mjs",
    "cjs",
    "py",
    "sh",
    "lock",
    "mod",
    "sum",
    "sql",
    "css",
    "scss",
    "html",
    "vue",
    "svelte",
    "java",
    "kt",
    "swift",
    "rb",
    "php",
    "cs",
    "cpp",
    "c",
    "h",
    "txt",
    "cfg",
    "ini",
    "gradle",
    "xml",
    "proto",
    "mdc",
    "dockerfile",
];

fn looks_like_path(tok: &str) -> bool {
    if tok.is_empty() || tok.len() > 200 || tok.contains(char::is_whitespace) {
        return false;
    }
    if tok.contains("://")
        || tok.starts_with('-')
        || tok.starts_with('@')
        || tok.starts_with('~')
        || tok.starts_with('$')
        || tok.starts_with('/')
    {
        return false;
    }
    if tok.chars().any(|c| {
        matches!(
            c,
            '*' | '?'
                | '{'
                | '}'
                | '<'
                | '>'
                | '$'
                | '|'
                | '='
                | '('
                | ')'
                | '['
                | ']'
                | ','
                | ';'
                | '!'
                | '"'
                | '\''
                | '\\'
        )
    }) || tok.contains("...")
        || tok.contains('…')
    {
        return false;
    }
    let last = tok.trim_end_matches('/').rsplit('/').next().unwrap_or(tok);
    let ext = last.rsplit_once('.').map(|(_, e)| e.to_lowercase());
    let has_ext = last.contains('.')
        && !last.starts_with('.')
        && ext.as_deref().is_some_and(|e| FILE_EXTS.contains(&e));
    let dot_file = last.starts_with('.')
        && last.len() > 1
        && last[1..]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
        && tok.contains('/');
    if tok.contains('/') {
        return has_ext
            || tok.ends_with('/')
            || tok.starts_with("./")
            || tok.starts_with("../")
            || dot_file;
    }
    has_ext
}

/// Расширения, для которых проверяется и голое имя без каталога.
const BARE_CHECKED: &[&str] = &[
    "json", "toml", "yaml", "yml", "lock", "mod", "sum", "md", "cfg", "ini", "gradle", "xml",
];

/// Слова, по которым строка описывает создаваемый файл или пример, а не существующий: путь в ней не проверяем.
const EXAMPLE_WORDS: &[&str] = &[
    "create",
    "creates",
    "creating",
    "new file",
    "such as",
    "for example",
    "e.g.",
    "i.e.",
    "example",
    "will be generated",
    "generated",
    "writes",
    "produces",
    "emits",
    "outputs",
    "output file",
    "generates",
    "stores",
    "persists",
    "saves",
    "создай",
    "создать",
    "создаётся",
    "например",
    "пример",
];

fn stripped_prefix(mut t: &str) -> &str {
    loop {
        if let Some(r) = t.strip_prefix("./").or_else(|| t.strip_prefix("../")) {
            t = r;
        } else {
            return t;
        }
    }
}

/// Скомпилированные файлы (.js) в инструкциях обычно упомянуты по исходнику (.ts), который есть в репозитории.
fn ts_counterpart(repo: &Repo, path: &str) -> bool {
    let p = stripped_prefix(path);
    let Some((stem, ext)) = p.rsplit_once('.') else {
        return false;
    };
    let alts: &[&str] = match ext {
        "js" => &["ts", "tsx"],
        "jsx" => &["tsx"],
        "mjs" => &["mts", "ts"],
        "cjs" => &["cts", "ts"],
        _ => return false,
    };
    alts.iter().any(|a| repo.has_suffix(&format!("{stem}.{a}")))
}

fn paths(repo: &Repo, f: &Instr, s: &mut Sink) {
    let mut seen = HashSet::new();
    for sp in &f.doc.spans {
        let line_lc = f
            .doc
            .lines
            .get(sp.line - 1)
            .map(|l| l.to_lowercase())
            .unwrap_or_default();
        if EXAMPLE_WORDS.iter().any(|w| line_lc.contains(w)) {
            continue;
        }
        let tok = sp.text.trim();
        // команды вроде `npm run x` и пути с пробелами не трогаем; «foo.js:12» — ссылка на строку
        let tok = tok
            .rsplit_once(':')
            .filter(|(_, n)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
            .map_or(tok, |(p, _)| p);
        if !looks_like_path(tok) {
            continue;
        }
        let is_dir = tok.ends_with('/');
        let rel_path = tok.trim_end_matches('/');
        // кандидаты: от каталога файла, от корня
        let mut cands: Vec<String> = Vec::new();
        if let Some(p) = join_norm(&f.dir, rel_path) {
            cands.push(p);
        }
        if let Some(p) = join_norm("", rel_path) {
            if !cands.contains(&p) {
                cands.push(p);
            }
        }
        if cands.is_empty() || cands.iter().any(|p| repo.exists(p) || repo.ignored(p)) {
            continue;
        }
        if !tok.contains('/') {
            // голое имя файла: достаточно, чтобы такой файл был где-нибудь; а имена исходников без пути («foo.py»,
            // «_layout.tsx», «stdarg.h») почти всегда примеры, поэтому их не проверяем
            if repo.name_exists(tok)
                || !BARE_CHECKED
                    .contains(&tok.rsplit('.').next().unwrap_or("").to_lowercase().as_str())
            {
                continue;
            }
        } else if repo.has_suffix(stripped_prefix(tok)) || ts_counterpart(repo, rel_path) {
            continue;
        }
        if !seen.insert((sp.line, tok.to_string())) {
            continue;
        }
        let what = if is_dir {
            "каталога"
        } else {
            "файла"
        };
        s.add(
            "broken-path",
            sp.line,
            sp.col,
            format!("упомянут путь `{tok}`, но {what} с таким путём нет в репозитории"),
            Some("если файл создаётся сборкой, добавьте его в .gitignore или уберите упоминание"),
        );
    }
}

// ---------- импорты ----------

fn import_tokens(line: &str) -> Vec<(usize, String)> {
    let ch: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < ch.len() {
        if ch[i] == '@' && (i == 0 || ch[i - 1].is_whitespace() || ch[i - 1] == '(') {
            let mut j = i + 1;
            while j < ch.len()
                && (ch[j].is_alphanumeric() || matches!(ch[j], '_' | '.' | '/' | '-' | '~'))
            {
                j += 1;
            }
            let tok: String = ch[i + 1..j].iter().collect();
            let tok = tok.trim_end_matches(['.', ',', ';', ':']).to_string();
            if !tok.is_empty() {
                out.push((i + 1, tok));
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

fn has_file_ext(t: &str) -> bool {
    t.rsplit('/')
        .next()
        .and_then(|l| l.rsplit_once('.'))
        .is_some_and(|(n, e)| {
            !n.is_empty()
                && !e.is_empty()
                && e.len() <= 6
                && e.chars().all(|c| c.is_ascii_alphanumeric())
        })
}

fn resolve_imports(repo: &Repo, f: &Instr) -> Vec<(usize, usize, String, bool)> {
    // (строка, столбец, путь от корня, существует)
    let mut out = Vec::new();
    for (no, line) in &f.doc.prose {
        for (col, tok) in import_tokens(line) {
            if tok.starts_with('~') || tok.starts_with('/') {
                continue; // пользовательские файлы вне репозитория проверить нечем
            }
            let Some(p) = join_norm(&f.dir, &tok) else {
                continue;
            };
            let exists = repo.is_file(&p);
            if exists || has_file_ext(&tok) {
                out.push((*no, col, p, exists));
            }
        }
    }
    out
}

struct Walk<'a> {
    repo: &'a Repo,
    all: &'a HashMap<&'a str, &'a Instr>,
    stack: Vec<String>,
    visited: BTreeSet<String>,
    total: usize,
    cycle: Option<(usize, usize, String)>,
    deep: bool,
}

impl Walk<'_> {
    /// `hop` — позиция первого импорта в проверяемом файле: к ней привязываются найденные ниже проблемы.
    fn visit(&mut self, cur: &Instr, hop: Option<(usize, usize)>) {
        for (line, col, p, exists) in resolve_imports(self.repo, cur) {
            if !exists {
                continue;
            }
            let at = hop.unwrap_or((line, col));
            if self.stack.contains(&p) {
                if self.cycle.is_none() {
                    self.cycle = Some((at.0, at.1, p.clone()));
                }
                continue;
            }
            if self.stack.len() >= 5 {
                self.deep = true;
                continue;
            }
            if self.visited.insert(p.clone()) {
                self.total += match self.all.get(p.as_str()) {
                    Some(i) => i.text.len(),
                    None => self.repo.read(&p).map_or(0, |t| t.len()),
                };
            }
            if let Some(next) = Instr::load(self.repo, &p) {
                self.stack.push(p);
                self.visit(&next, Some(at));
                self.stack.pop();
            }
        }
    }
}

fn imports(repo: &Repo, f: &Instr, all: &HashMap<&str, &Instr>, opts: &Options, s: &mut Sink) {
    let own = resolve_imports(repo, f);
    for (line, col, p, exists) in &own {
        if !exists {
            s.add("import-missing", *line, *col, format!("импорт `@{p}` указывает на файл, которого нет"), Some("Claude Code подключает такие файлы в контекст; несуществующий импорт молча теряется"));
        }
    }
    // обход графа: циклы, глубина, суммарный размер
    let mut w = Walk {
        repo,
        all,
        stack: vec![f.rel.clone()],
        visited: BTreeSet::from([f.rel.clone()]),
        total: f.text.len(),
        cycle: None,
        deep: false,
    };
    w.visit(f, None);
    if let Some((line, col, p)) = w.cycle.take() {
        s.add(
            "import-cycle",
            line,
            col,
            format!("цепочка импортов возвращается к `{p}`"),
            Some("уберите один из импортов, иначе содержимое подключится не так, как задумано"),
        );
    }
    if w.deep {
        let (line, col) = own.first().map_or((1, 1), |(l, c, _, _)| (*l, *c));
        s.add("import-depth", line, col, "цепочка импортов глубже 5 уровней".to_string(), Some("Claude Code раскрывает импорты не глубже пяти уровней: остальное не попадёт в контекст"));
    }
    let (total, visited) = (w.total, w.visited);
    let tokens = total / 4;
    if tokens > opts.max_tokens {
        let extra = if visited.len() > 1 {
            format!(" вместе с {} импортированными файлами", visited.len() - 1)
        } else {
            String::new()
        };
        s.add("too-long", 1, 1, format!("файл{extra} занимает около {tokens} токенов при пределе {}", opts.max_tokens), Some("агент читает файл целиком в каждой сессии: оставьте главное, подробности вынесите в docs/ и ссылайтесь"));
    }
}

// ---------- команды ----------

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur.push((prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost));
        }
        prev = cur;
    }
    prev[b.len()]
}

fn suggest<'a>(name: &str, options: impl Iterator<Item = &'a String>) -> Option<String> {
    options
        .filter(|o| {
            levenshtein(name, o) <= 2 || o.starts_with(name) || name.starts_with(o.as_str())
        })
        .min_by_key(|o| levenshtein(name, o))
        .cloned()
}

const NPM_BUILTIN_START: &[&str] = &["test", "t", "tst", "start", "stop", "restart"];
const PNPM_BUILTIN: &[&str] = &[
    "add",
    "audit",
    "bin",
    "cache",
    "config",
    "create",
    "dedupe",
    "deploy",
    "dlx",
    "doctor",
    "env",
    "exec",
    "fetch",
    "i",
    "import",
    "init",
    "install",
    "install-test",
    "it",
    "link",
    "list",
    "ln",
    "ls",
    "outdated",
    "pack",
    "patch",
    "patch-commit",
    "prune",
    "publish",
    "rb",
    "rebuild",
    "remove",
    "rm",
    "root",
    "run",
    "self-update",
    "setup",
    "store",
    "uninstall",
    "unlink",
    "up",
    "update",
    "upgrade",
    "why",
    "help",
    "-v",
    "--version",
];
const YARN_BUILTIN: &[&str] = &[
    "add",
    "audit",
    "bin",
    "cache",
    "config",
    "constraints",
    "create",
    "dedupe",
    "dlx",
    "exec",
    "explain",
    "global",
    "help",
    "import",
    "info",
    "init",
    "install",
    "link",
    "list",
    "node",
    "npm",
    "outdated",
    "pack",
    "patch",
    "plugin",
    "publish",
    "rebuild",
    "remove",
    "run",
    "set",
    "unlink",
    "unplug",
    "up",
    "upgrade",
    "upgrade-interactive",
    "version",
    "why",
    "workspace",
    "workspaces",
    "dev",
];
const SKIP_FLAGS: &[&str] = &[
    "-w",
    "--workspace",
    "--workspaces",
    "-ws",
    "--filter",
    "-F",
    "-r",
    "--recursive",
    "--prefix",
    "-C",
    "--cwd",
    "--dir",
    "--filter-prod",
    "--if-present",
];

fn node_script(tool: &str, args: &[String]) -> Option<String> {
    let mut it = args.iter().filter(|a| !a.starts_with('-'));
    let sub = it.next()?.as_str();
    let run_like = |next: Option<&String>| next.cloned();
    match tool {
        "npm" => match sub {
            "run" | "run-script" | "rum" | "urn" => run_like(it.next()),
            s if NPM_BUILTIN_START.contains(&s) => Some(match s {
                "t" | "tst" => "test".to_string(),
                other => other.to_string(),
            }),
            _ => None,
        },
        "pnpm" => match sub {
            "run" => run_like(it.next()),
            "test" | "start" | "restart" | "stop" => Some(sub.to_string()),
            s if PNPM_BUILTIN.contains(&s) => None,
            s => Some(s.to_string()),
        },
        "yarn" => match sub {
            "run" => run_like(it.next()),
            "test" | "start" => Some(sub.to_string()),
            s if YARN_BUILTIN.contains(&s) => None,
            s => Some(s.to_string()),
        },
        "bun" => match sub {
            "run" => run_like(it.next()).filter(|n| !n.contains('/') && !has_file_ext(n)),
            _ => None,
        },
        _ => None,
    }
}

fn commands(repo: &Repo, f: &Instr, cmds: &[Cmd], s: &mut Sink) {
    let mut seen: HashSet<(usize, String)> = HashSet::new();
    for c in cmds {
        let Some(cwd) = &c.cwd else { continue };
        if !seen.insert((c.line, c.text.clone())) {
            continue;
        }
        let tool = c.words[0].as_str();
        let args = &c.words[1..];
        match tool {
            "npm" | "pnpm" | "yarn" | "bun" => {
                if args.iter().any(|a| {
                    SKIP_FLAGS.contains(&a.as_str())
                        || a.starts_with("--workspace")
                        || a.starts_with("--filter")
                        || a.starts_with("--prefix")
                }) {
                    continue;
                }
                let Some(script) = node_script(tool, args) else {
                    continue;
                };
                if script.contains('$') || script.contains('<') || script.contains('{') {
                    continue;
                }
                let Some(dir) = repo.nearest(cwd, "package.json") else {
                    if repo.name_exists("package.json") {
                        continue; // монорепозиторий: пакет выбирается каталогом запуска, который из текста не вывести
                    }
                    s.add(
                        "no-manifest",
                        c.line,
                        c.col,
                        format!(
                            "команда `{}`, но package.json нет ни в `{}`, ни выше",
                            c.text,
                            shown(cwd)
                        ),
                        Some("добавьте package.json или поправьте каталог в инструкции (cd …)"),
                    );
                    continue;
                };
                let Some(pkg) = repo.package_json(&dir) else {
                    continue;
                };
                if pkg.scripts.contains(&script) || scripts_above(repo, &dir).contains(&script) {
                    continue;
                }
                if script == "start"
                    && repo.exists(&join_norm(&dir, "server.js").unwrap_or_default())
                {
                    continue; // npm start без скрипта запускает server.js
                }
                let hint = match suggest(&script, pkg.scripts.iter()) {
                    Some(h) => format!(
                        "возможно, имелся в виду `{h}`; в package.json есть: {}",
                        list(&pkg.scripts)
                    ),
                    None if pkg.scripts.is_empty() => {
                        "в package.json нет раздела scripts".to_string()
                    }
                    None => format!("в package.json есть: {}", list(&pkg.scripts)),
                };
                s.add(
                    "missing-script",
                    c.line,
                    c.col,
                    format!(
                        "команда `{}`, но скрипта `{script}` нет в {}",
                        c.text,
                        pkg_path(&dir)
                    ),
                    Some(&hint),
                );
            }
            "make" => {
                if args.iter().any(|a| {
                    a == "-f"
                        || a == "--file"
                        || a == "-C"
                        || a == "--directory"
                        || a.starts_with("--directory=")
                        || a.starts_with("--file=")
                }) {
                    continue;
                }
                let Some(mk) = merged_makefile(repo, cwd) else {
                    if repo.name_exists("Makefile") {
                        continue;
                    }
                    s.add("no-manifest", c.line, c.col, format!("команда `{}`, но Makefile нет ни в `{}`, ни выше", c.text, shown(cwd)), Some("make не ищет Makefile в других каталогах: поправьте путь (cd …) или добавьте файл"));
                    continue;
                };
                if mk.has_include || mk.has_pattern_rules {
                    continue;
                }
                for t in args
                    .iter()
                    .filter(|a| !a.starts_with('-') && !a.contains('='))
                {
                    if !mk.targets.contains(t) {
                        let hint = match suggest(t, mk.targets.iter()) {
                            Some(h) => format!("возможно, имелась в виду цель `{h}`"),
                            None => format!("в Makefile есть: {}", list(&mk.targets)),
                        };
                        s.add(
                            "missing-script",
                            c.line,
                            c.col,
                            format!("команда `{}`, но цели `{t}` нет в Makefile", c.text),
                            Some(&hint),
                        );
                    }
                }
            }
            "just" => {
                if args.iter().any(|a| a.starts_with('-')) {
                    continue;
                }
                let Some(dir) = repo
                    .nearest(cwd, "justfile")
                    .or_else(|| repo.nearest(cwd, "Justfile"))
                    .or_else(|| repo.nearest(cwd, ".justfile"))
                else {
                    s.add(
                        "no-manifest",
                        c.line,
                        c.col,
                        format!("команда `{}`, но justfile не найден", c.text),
                        None,
                    );
                    continue;
                };
                let Some((recipes, partial)) = repo.justfile_recipes(&dir) else {
                    continue;
                };
                if partial {
                    continue;
                }
                if let Some(t) = args
                    .iter()
                    .find(|a| !a.contains('=') && !a.contains('/') && !a.contains(':'))
                {
                    if !recipes.contains(t) {
                        let hint = match suggest(t, recipes.iter()) {
                            Some(h) => format!("возможно, имелся в виду рецепт `{h}`"),
                            None => format!("в justfile есть: {}", list(&recipes)),
                        };
                        s.add(
                            "missing-script",
                            c.line,
                            c.col,
                            format!("команда `{}`, но рецепта `{t}` нет в justfile", c.text),
                            Some(&hint),
                        );
                    }
                }
            }
            "cargo" => {
                if args
                    .iter()
                    .any(|a| a == "--manifest-path" || a.starts_with("--manifest-path="))
                {
                    continue;
                }
                let sub = args
                    .iter()
                    .find(|a| !a.starts_with('-') && !a.starts_with('+'))
                    .map(String::as_str)
                    .unwrap_or("");
                if matches!(
                    sub,
                    "install"
                        | "new"
                        | "init"
                        | "search"
                        | "login"
                        | "logout"
                        | "owner"
                        | "yank"
                        | "version"
                        | "help"
                        | ""
                ) {
                    continue;
                }
                if repo.nearest(cwd, "Cargo.toml").is_none() && !repo.name_exists("Cargo.toml") {
                    s.add(
                        "no-manifest",
                        c.line,
                        c.col,
                        format!(
                            "команда `{}`, но Cargo.toml нет ни в `{}`, ни выше",
                            c.text,
                            shown(cwd)
                        ),
                        None,
                    );
                }
            }
            "go" => {
                let sub = args
                    .iter()
                    .find(|a| !a.starts_with('-'))
                    .map(String::as_str)
                    .unwrap_or("");
                if matches!(
                    sub,
                    "install" | "version" | "env" | "help" | "bug" | "telemetry" | ""
                ) || (sub == "mod" && args.iter().any(|a| a == "init"))
                    || (sub == "run" && args.iter().any(|a| a.contains('@')))
                {
                    continue;
                }
                if repo.nearest(cwd, "go.mod").is_none()
                    && repo.nearest(cwd, "go.work").is_none()
                    && !repo.name_exists("go.mod")
                {
                    s.add(
                        "no-manifest",
                        c.line,
                        c.col,
                        format!(
                            "команда `{}`, но go.mod нет ни в `{}`, ни выше",
                            c.text,
                            shown(cwd)
                        ),
                        None,
                    );
                }
            }
            _ => {}
        }
    }
    let _ = f;
}

/// Скрипты package.json во всех каталогах выше `dir`: вложенные инструкции часто просят запускать команды из корня.
fn scripts_above(repo: &Repo, dir: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut d = dir.to_string();
    while !d.is_empty() {
        d = parent_dir(&d).to_string();
        if let Some(p) = repo.package_json(&d) {
            out.extend(p.scripts);
        }
    }
    out
}

/// Makefile в `dir` и выше, объединённые: цели, шаблонные правила и include.
fn merged_makefile(repo: &Repo, dir: &str) -> Option<crate::repo::MakeFacts> {
    let mut merged: Option<crate::repo::MakeFacts> = None;
    let mut d = dir.to_string();
    loop {
        if let Some(m) = repo.makefile(&d) {
            let acc = merged.get_or_insert_with(Default::default);
            acc.targets.extend(m.targets);
            acc.has_pattern_rules |= m.has_pattern_rules;
            acc.has_include |= m.has_include;
        }
        if d.is_empty() {
            break;
        }
        d = parent_dir(&d).to_string();
    }
    merged
}

fn shown(dir: &str) -> &str {
    if dir.is_empty() {
        "корне репозитория"
    } else {
        dir
    }
}

fn pkg_path(dir: &str) -> String {
    if dir.is_empty() {
        "package.json".to_string()
    } else {
        format!("{dir}/package.json")
    }
}

fn list(items: &[String]) -> String {
    if items.is_empty() {
        return "ничего".to_string();
    }
    let mut v: Vec<&str> = items.iter().take(8).map(String::as_str).collect();
    if items.len() > 8 {
        v.push("…");
    }
    v.join(", ")
}

// ---------- менеджеры пакетов ----------

fn lockfiles(repo: &Repo, dir: &str) -> BTreeSet<&'static str> {
    let mut set = BTreeSet::new();
    let mut d = dir.to_string();
    loop {
        for (name, pm) in [
            ("pnpm-lock.yaml", "pnpm"),
            ("yarn.lock", "yarn"),
            ("package-lock.json", "npm"),
            ("npm-shrinkwrap.json", "npm"),
            ("bun.lock", "bun"),
            ("bun.lockb", "bun"),
        ] {
            if repo.is_file(&join_norm(&d, name).unwrap_or_default()) {
                set.insert(pm);
            }
        }
        if !set.is_empty() || d.is_empty() {
            break;
        }
        d = parent_dir(&d).to_string();
    }
    set
}

/// Команда относится к проекту, а не к машине или реестру: без `-g`/`global` и не `publish`, `view`, `login` и т. п.
fn project_scoped(words: &[String]) -> bool {
    if words
        .iter()
        .any(|w| w == "-g" || w == "--global" || w == "global")
    {
        return false;
    }
    let sub = words
        .iter()
        .skip(1)
        .find(|w| !w.starts_with('-'))
        .map(String::as_str)
        .unwrap_or("");
    !matches!(
        sub,
        "publish"
            | "view"
            | "info"
            | "login"
            | "logout"
            | "whoami"
            | "pack"
            | "dist-tag"
            | "access"
            | "owner"
            | "search"
            | "config"
            | "token"
            | "profile"
            | "adduser"
            | "deprecate"
            | "unpublish"
            | "audit"
            | "doctor"
            | "help"
            | "version"
            | "-v"
    )
}

fn package_managers(repo: &Repo, f: &Instr, cmds: &[Cmd], s: &mut Sink) {
    let mut first_use: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    let mut reported: HashSet<&str> = HashSet::new();
    for c in cmds {
        let tool = c.words[0].as_str();
        if !matches!(tool, "npm" | "pnpm" | "yarn" | "bun") || !project_scoped(&c.words) {
            continue;
        }
        let tool: &str = match tool {
            "npm" => "npm",
            "pnpm" => "pnpm",
            "yarn" => "yarn",
            _ => "bun",
        };
        first_use.entry(tool).or_insert((c.line, c.col));
        let Some(cwd) = &c.cwd else { continue };
        let Some(dir) = repo.nearest(cwd, "package.json") else {
            continue;
        };
        if reported.contains(tool) {
            continue;
        }
        let locks = lockfiles(repo, &dir);
        if !locks.is_empty() && !locks.contains(tool) {
            let have: Vec<&str> = locks.iter().copied().collect();
            s.add("pm-lockfile-mismatch", c.line, c.col, format!("в инструкции `{tool}`, а lockfile в репозитории только для {}", have.join(", ")), Some("агент поставит зависимости не тем менеджером и изменит lockfile; согласуйте инструкцию с репозиторием"));
            reported.insert(tool);
            continue;
        }
        if let Some(pkg) = repo.package_json(&dir) {
            if let Some(pm) = pkg
                .package_manager
                .as_deref()
                .and_then(|p| p.split('@').next())
            {
                if matches!(pm, "npm" | "pnpm" | "yarn" | "bun") && pm != tool {
                    s.add(
                        "pm-field-mismatch",
                        c.line,
                        c.col,
                        format!(
                            "в инструкции `{tool}`, а в package.json packageManager: `{}`",
                            pkg.package_manager.as_deref().unwrap_or(pm)
                        ),
                        None,
                    );
                    reported.insert(tool);
                }
            }
        }
    }
    if first_use.len() > 1 {
        let names: Vec<&str> = first_use.keys().copied().collect();
        // отмечаем второй по порядку появления менеджер
        let mut by_pos: Vec<(&str, (usize, usize))> =
            first_use.iter().map(|(k, v)| (*k, *v)).collect();
        by_pos.sort_by_key(|(_, p)| *p);
        let (_, (line, col)) = by_pos[1];
        s.add(
            "pm-inconsistent",
            line,
            col,
            format!(
                "в одном файле используются разные менеджеры пакетов: {}",
                names.join(", ")
            ),
            Some("оставьте один: иначе агент выберет случайный"),
        );
    }
    let _ = f;
}

// ---------- версии ----------

fn mention_after(text: &str, word_lc: &str, allow_minor: bool) -> Vec<(usize, u32)> {
    // ищем «word 18», «word.js v20», для Go/Python — «go 1.21», «python 3.11»; возвращаем (столбец, число)
    let lc = text.to_lowercase();
    let chars: Vec<char> = lc.chars().collect();
    let w: Vec<char> = word_lc.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + w.len() <= chars.len() {
        if chars[i..i + w.len()] == w[..] && (i == 0 || !chars[i - 1].is_alphanumeric()) {
            let mut j = i + w.len();
            if chars.get(j) == Some(&'.')
                && chars.get(j + 1) == Some(&'j')
                && chars.get(j + 2) == Some(&'s')
            {
                j += 3;
            } else if chars.get(j) == Some(&'j') && chars.get(j + 1) == Some(&'s') {
                j += 2;
            }
            let mut k = j;
            while chars.get(k) == Some(&' ') {
                k += 1;
            }
            if k == j && j < chars.len() && chars[j].is_alphanumeric() && chars[j] != 'v' {
                i += 1;
                continue;
            }
            if chars.get(k) == Some(&'v') {
                k += 1;
            }
            let d1: String = chars[k..]
                .iter()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if !d1.is_empty() {
                let mut num = d1.parse::<u32>().ok();
                if allow_minor {
                    let after = k + d1.len();
                    if chars.get(after) == Some(&'.') {
                        let d2: String = chars[after + 1..]
                            .iter()
                            .take_while(|c| c.is_ascii_digit())
                            .collect();
                        num = d2.parse::<u32>().ok();
                        if d2.is_empty() {
                            num = None;
                        }
                    } else {
                        num = None;
                    }
                }
                if let Some(n) = num {
                    out.push((i + 1, n));
                }
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    out
}

fn versions(repo: &Repo, f: &Instr, s: &mut Sink) {
    // факты
    let node_exact = ["", &f.dir]
        .iter()
        .find_map(|d| {
            [".nvmrc", ".node-version"]
                .iter()
                .find_map(|n| repo.read(&join_norm(d, n)?))
        })
        .and_then(|t| leading_major(&t));
    let node_min = repo
        .nearest(&f.dir, "package.json")
        .and_then(|d| repo.package_json(&d))
        .and_then(|p| p.engines_node)
        .and_then(|e| min_major_in_range(&e));
    let go_min: Option<u32> = repo
        .nearest(&f.dir, "go.mod")
        .and_then(|d| repo.read(&join_norm(&d, "go.mod")?))
        .and_then(|t| {
            t.lines().find_map(|l| {
                let v = l.trim().strip_prefix("go ")?;
                v.trim().split('.').nth(1)?.parse().ok()
            })
        });
    let py: Option<(u32, bool)> = repo
        .read(".python-version")
        .and_then(|t| t.trim().split('.').nth(1)?.parse().ok())
        .map(|m| (m, true))
        .or_else(|| {
            let t = repo.read("pyproject.toml")?;
            let l = t
                .lines()
                .find(|l| l.trim_start().starts_with("requires-python"))?;
            let spec = l.split_once('=')?.1;
            let m = spec
                .split('.')
                .nth(1)?
                .chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .ok()?;
            Some((m, false))
        });

    let mut texts: Vec<(usize, String)> = f.doc.prose.clone();
    texts.extend(f.doc.spans.iter().map(|sp| (sp.line, sp.text.clone())));
    let mut seen = HashSet::new();
    for (line, text) in &texts {
        for (col, n) in mention_after(text, "node", false) {
            let msg = match (node_exact, node_min) {
                (Some(e), _) if e != n => Some(format!(
                    "в тексте Node {n}, а в .nvmrc/.node-version Node {e}"
                )),
                (None, Some(m)) if n < m => Some(format!(
                    "в тексте Node {n}, а package.json (engines.node) требует не ниже {m}"
                )),
                _ => None,
            };
            if let Some(m) = msg {
                if seen.insert((*line, "node")) {
                    s.add("stale-version", *line, col, m, Some("обновите упоминание версии или файл с версией: агент ориентируется на текст"));
                }
            }
        }
        for (col, n) in mention_after(text, "go", true) {
            if let Some(m) = go_min {
                if n < m && seen.insert((*line, "go")) {
                    s.add(
                        "stale-version",
                        *line,
                        col,
                        format!("в тексте Go 1.{n}, а go.mod требует не ниже 1.{m}"),
                        Some("обновите упоминание версии"),
                    );
                }
            }
        }
        for (col, n) in mention_after(text, "python", true) {
            if let Some((m, exact)) = py {
                let stale = if exact { n != m } else { n < m };
                if stale && seen.insert((*line, "python")) {
                    s.add(
                        "stale-version",
                        *line,
                        col,
                        format!("в тексте Python 3.{n}, а проект указывает 3.{m}"),
                        Some("обновите упоминание версии"),
                    );
                }
            }
        }
    }
}

// ---------- секреты ----------

fn take_while_at(ch: &[char], from: usize, f: impl Fn(char) -> bool) -> usize {
    ch[from..].iter().take_while(|c| f(**c)).count()
}

fn find_secret(line: &str) -> Option<(usize, &'static str, String)> {
    let ch: Vec<char> = line.chars().collect();
    let alnum = |c: char| c.is_ascii_alphanumeric();
    let tok = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    for i in 0..ch.len() {
        if i > 0 && (alnum(ch[i - 1]) || ch[i - 1] == '_') {
            continue; // только с начала «слова»
        }
        let rest: String = ch[i..].iter().take(12).collect();
        let n;
        let kind = if rest.starts_with("AKIA") {
            n = 4 + take_while_at(&ch, i + 4, |c| c.is_ascii_uppercase() || c.is_ascii_digit());
            if n == 20 {
                Some("ключ доступа AWS")
            } else {
                None
            }
        } else if ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"]
            .iter()
            .any(|p| rest.starts_with(p))
        {
            n = 4 + take_while_at(&ch, i + 4, alnum);
            if n >= 40 {
                Some("токен GitHub")
            } else {
                None
            }
        } else if rest.starts_with("github_pat_") {
            n = 11 + take_while_at(&ch, i + 11, tok);
            if n >= 33 {
                Some("токен GitHub")
            } else {
                None
            }
        } else if rest.starts_with("sk-") {
            n = 3 + take_while_at(&ch, i + 3, tok);
            if n >= 23 {
                Some("API-ключ (sk-…)")
            } else {
                None
            }
        } else if rest.starts_with("xox")
            && matches!(ch.get(i + 3), Some('a' | 'b' | 'p' | 'r' | 's'))
            && ch.get(i + 4) == Some(&'-')
        {
            n = 5 + take_while_at(&ch, i + 5, tok);
            if n >= 15 {
                Some("токен Slack")
            } else {
                None
            }
        } else if rest.starts_with("AIza") {
            n = 4 + take_while_at(&ch, i + 4, tok);
            if n >= 39 {
                Some("ключ Google API")
            } else {
                None
            }
        } else if line[line.char_indices().nth(i).map_or(0, |(b, _)| b)..]
            .starts_with("-----BEGIN ")
            && line.contains("PRIVATE KEY-----")
        {
            n = 11;
            Some("приватный ключ")
        } else {
            n = 0;
            None
        };
        if let Some(k) = kind {
            let shown: String = ch[i..(i + n.min(6)).min(ch.len())].iter().collect();
            return Some((i + 1, k, format!("{shown}…")));
        }
    }
    None
}

fn secrets(f: &Instr, s: &mut Sink) {
    for (i, line) in f.doc.lines.iter().enumerate() {
        if let Some((col, kind, masked)) = find_secret(line) {
            s.add("secret", i + 1, col, format!("похоже на секрет: {kind} ({masked})"), Some("уберите значение и отзовите ключ: файл-инструкция читается агентом и лежит в git"));
        }
    }
}

// ---------- расплывчатые указания ----------

const VAGUE: &[&str] = &[
    "write clean code",
    "follow best practices",
    "be careful",
    "write good code",
    "high quality code",
    "high-quality code",
    "write maintainable code",
    "use common sense",
    "be thorough",
    "follow coding standards",
    "пиши чистый код",
    "следуй лучшим практикам",
    "будь внимателен",
    "пиши качественный код",
    "используй здравый смысл",
    "пиши хороший код",
];

fn vague(f: &Instr, s: &mut Sink) {
    for (no, line) in &f.doc.prose {
        let lc = line.to_lowercase();
        for v in VAGUE {
            if let Some(b) = lc.find(v) {
                let col = lc[..b].chars().count() + 1;
                s.add("vague-instruction", *no, col, format!("«{v}» не задаёт проверяемого действия"), Some("замените конкретным правилом или командой: например, «перед коммитом запускай `npm run lint`»"));
            }
        }
    }
}

// ---------- повторы ----------

fn norm_line(l: &str) -> String {
    l.trim()
        .trim_start_matches(['-', '*', '+', '>', ' '])
        .trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn duplicates(f: &Instr, s: &mut Sink) {
    let mut seen: HashMap<String, usize> = HashMap::new();
    for (no, line) in &f.doc.prose {
        let n = norm_line(line);
        if n.chars().count() < 25 || n.starts_with('#') || n.starts_with('|') || n.contains("](") {
            continue;
        }
        match seen.get(&n) {
            Some(first) => s.add(
                "duplicate-line",
                *no,
                1,
                format!("строка повторяет строку {first}"),
                Some("удалите повтор: он расходует контекст и может разойтись с оригиналом"),
            ),
            None => {
                seen.insert(n, *no);
            }
        }
    }
}

fn duplicate_files(instrs: &[Instr], out: &mut Vec<Diagnostic>) {
    let mut by_dir: BTreeMap<&str, Vec<&Instr>> = BTreeMap::new();
    for i in instrs {
        let name = i.rel.rsplit('/').next().unwrap_or("").to_lowercase();
        if matches!(name.as_str(), "agents.md" | "claude.md" | "gemini.md") {
            by_dir.entry(i.dir.as_str()).or_default().push(i);
        }
    }
    for files in by_dir.values() {
        for (a_idx, a) in files.iter().enumerate() {
            for b in &files[a_idx + 1..] {
                let lines = |x: &Instr| -> BTreeSet<String> {
                    x.doc
                        .prose
                        .iter()
                        .map(|(_, l)| norm_line(l))
                        .filter(|l| l.chars().count() >= 20)
                        .collect()
                };
                let (la, lb) = (lines(a), lines(b));
                let common = la.intersection(&lb).count();
                let small = la.len().min(lb.len());
                let links = |x: &Instr, y: &Instr| -> bool {
                    let yn = y.rel.rsplit('/').next().unwrap_or("");
                    x.doc
                        .prose
                        .iter()
                        .any(|(_, l)| l.contains(&format!("@{yn}")))
                };
                if links(a, b) || links(b, a) || small == 0 {
                    continue;
                }
                if common * 2 >= small {
                    let bn = b.rel.rsplit('/').next().unwrap_or("");
                    let an = a.rel.rsplit('/').next().unwrap_or("");
                    out.push(Diagnostic {
                        rule: "duplicate-files",
                        severity: Severity::Info,
                        file: b.rel.clone(),
                        line: 1,
                        col: 1,
                        message: format!("{bn} повторяет {common} строк из {an} и разойдётся с ним со временем"),
                        hint: Some(format!("оставьте содержимое в одном файле, а в другом одну строку `@{an}` (импорт)")),
                    });
                }
            }
        }
    }
}

// ---------- противоречия ----------

const POSITIVE: &[&str] = &[
    "always use ",
    "must use ",
    "should use ",
    "prefer ",
    "use ",
    "всегда используй ",
    "используй ",
    "используйте ",
    "предпочитай ",
];
const NEGATIVE: &[&str] = &[
    "never use ",
    "do not use ",
    "don't use ",
    "avoid ",
    "never ",
    "не используй ",
    "не используйте ",
    "избегай ",
    "избегайте ",
    "никогда не используй ",
];
const EXCLUSIVE: &[&[&str]] = &[
    &["tabs", "spaces", "табы", "пробелы", "табуляцию", "пробелов"],
    &["npm", "pnpm", "yarn", "bun"],
    &["jest", "vitest", "mocha"],
];

fn directive(line: &str) -> Option<(bool, String)> {
    let t = line
        .trim()
        .trim_start_matches(['-', '*', '+', '>', ' ', '#'])
        .trim();
    let lc = t.to_lowercase();
    // «- use X»: указание в начале пункта
    for (set, neg) in [(NEGATIVE, true), (POSITIVE, false)] {
        for trig in set {
            if let Some(rest) = lc.strip_prefix(trig) {
                let obj: String = rest
                    .split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_' || c == '.'))
                    .next()
                    .unwrap_or("")
                    .to_string();
                let obj = obj.trim_matches('.').to_string();
                if !obj.is_empty()
                    && !matches!(obj.as_str(), "use" | "using" | "do" | "be" | "write")
                {
                    return Some((!neg, obj));
                }
            }
        }
    }
    None
}

fn contradictions(f: &Instr, s: &mut Sink) {
    let mut dirs: Vec<(usize, bool, String)> = Vec::new();
    for (no, l) in &f.doc.prose {
        // встроенный код стёрт в prose; для объектов вроде `pnpm` смотрим исходную строку
        let raw = &f.doc.lines[no - 1];
        let cleaned = raw.replace('`', "");
        if let Some((pos, obj)) = directive(&cleaned).or_else(|| directive(l)) {
            dirs.push((*no, pos, obj));
        }
    }
    let mut reported: HashSet<(usize, usize)> = HashSet::new();
    for (i, (la, pa, oa)) in dirs.iter().enumerate() {
        for (lb, pb, ob) in &dirs[i + 1..] {
            let clash = (oa == ob && pa != pb)
                || (*pa
                    && *pb
                    && oa != ob
                    && EXCLUSIVE.iter().any(|g| {
                        g.contains(&oa.as_str())
                            && g.contains(&ob.as_str())
                            && !(oa == "tabs" && ob == "табы")
                    }));
            if clash && reported.insert((*la, *lb)) {
                let what = if oa == ob {
                    format!("`{oa}` одновременно предписывается и запрещается")
                } else {
                    format!("предписываются взаимоисключающие `{oa}` и `{ob}`")
                };
                s.add("contradiction", *lb, 1, format!("{what} (строки {la} и {lb})"), Some("если это разные случаи, уточните область («в тестах», «во фронтенде»); иначе оставьте одно указание"));
            }
        }
    }
}

// ---------- отсутствие команд ----------

fn no_commands(f: &Instr, cmds: &[Cmd], s: &mut Sink) {
    let name = f.rel.rsplit('/').next().unwrap_or("").to_lowercase();
    if !matches!(name.as_str(), "agents.md" | "claude.md" | "gemini.md") || !f.dir.is_empty() {
        return;
    }
    // любая команда (в блоке sh или во встроенном коде) показывает агенту, чем пользоваться
    let has = !cmds.is_empty();
    let pointer = f.doc.links.iter().any(|l| {
        let base = l
            .target
            .split(['#', '?'])
            .next()
            .unwrap_or("")
            .rsplit('/')
            .next()
            .unwrap_or("")
            .to_lowercase();
        matches!(
            base.as_str(),
            "agents.md" | "claude.md" | "gemini.md" | "copilot-instructions.md"
        ) || base.ends_with(".instructions.md")
    });
    let imports_other = pointer
        || f.doc
            .prose
            .iter()
            .any(|(_, l)| !import_tokens(l).is_empty());
    if !has && !imports_other && f.text.trim().len() > 40 {
        s.add("no-commands", 1, 1, "в файле нет команд сборки, тестов или проверки".to_string(), Some("агенту нужно знать, чем проверить свою работу: добавьте блок ```sh с командами test, lint, build"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_heuristics() {
        for yes in [
            "src/lib.rs",
            "docs/",
            "./scripts/build.sh",
            "package.json",
            "../x/y.md",
            ".github/workflows/ci.yml",
        ] {
            assert!(looks_like_path(yes), "{yes}");
        }
        for no in [
            "npm run build",
            "foo",
            "a/b",
            "https://x.dev/a.md",
            "src/**/*.ts",
            "<file>.md",
            "$HOME/a.md",
            "1.5",
            "e.g.",
            "--flag.json",
            "user@host.com",
            "a=b.js",
        ] {
            assert!(!looks_like_path(no), "{no}");
        }
    }

    #[test]
    fn secrets_are_found_and_masked() {
        assert!(find_secret("key AKIAABCDEFGHIJKLMNOP end").is_some());
        assert!(find_secret("token ghp_abcdefghijklmnopqrstuvwxyz0123456789").is_some());
        assert!(find_secret("sk-ant-api03-abcdefghijklmnopqrstuvwxyz").is_some());
        assert!(find_secret("-----BEGIN RSA PRIVATE KEY-----").is_some());
        assert!(find_secret("use sk-... as the key").is_none());
        assert!(find_secret("AKIA is a prefix").is_none());
        let (_, _, masked) = find_secret("ghp_abcdefghijklmnopqrstuvwxyz0123456789").unwrap();
        assert_eq!(masked, "ghp_ab…");
    }

    #[test]
    fn version_mentions() {
        assert_eq!(
            mention_after("Requires Node 18 or newer", "node", false),
            vec![(10, 18)]
        );
        assert_eq!(mention_after("Node.js v20", "node", false), vec![(1, 20)]);
        assert!(mention_after("nodes 20", "node", false).is_empty());
        assert_eq!(mention_after("use Go 1.21 here", "go", true), vec![(5, 21)]);
        assert!(mention_after("go 2 times", "go", true).is_empty());
        assert_eq!(mention_after("Python 3.11+", "python", true), vec![(1, 11)]);
    }

    #[test]
    fn directives() {
        assert_eq!(
            directive("- Always use tabs for indentation"),
            Some((true, "tabs".into()))
        );
        assert_eq!(directive("Never use any"), Some((false, "any".into())));
        assert_eq!(directive("Never use `any`"), None);
        assert_eq!(directive("- Don't use yarn."), Some((false, "yarn".into())));
        assert_eq!(
            directive("- Не используй lodash"),
            Some((false, "lodash".into()))
        );
        assert_eq!(directive("The tests use jest"), None);
    }

    #[test]
    fn edit_distance_suggestions() {
        let names = ["build".to_string(), "test".to_string(), "lint".to_string()];
        assert_eq!(suggest("biuld", names.iter()).as_deref(), Some("build"));
        assert_eq!(suggest("deploy-everything", names.iter()), None);
    }

    #[test]
    fn percent_decoding() {
        assert_eq!(percent_decode("my%20file.md"), "my file.md");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("a%zzb"), "a%zzb");
    }
}
