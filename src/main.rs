use agentsmd_lint::json::{self, Json};
use agentsmd_lint::repo::Repo;
use agentsmd_lint::report;
use agentsmd_lint::rules::RULES;
use agentsmd_lint::{discover, lint, select_files, Options, Severity};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const HELP: &str = "agentsmd-lint: проверка файлов-инструкций для ИИ-агентов (AGENTS.md, CLAUDE.md, GEMINI.md, правила Cursor и Copilot).

Использование:
  agentsmd-lint [ПУТЬ] [параметры]      ПУТЬ — каталог репозитория или один файл (по умолчанию .)

Параметры:
  --format text|json|sarif   формат отчёта (по умолчанию text)
  --fail-on error|warning|info|never
                             с какого уровня код выхода 1 (по умолчанию error)
  --max-tokens N             предел размера файла с импортами в токенах (по умолчанию 6000)
  --ignore ПРАВИЛО           отключить правило (можно несколько раз)
  --ignore-path ПУТЬ         не проверять файл или каталог (оканчивается на /)
  --exec                     ВЫПОЛНЯТЬ команды test/lint/build из инструкций и сообщать о неработающих.
                             Запускает код репозитория: только для доверенных репозиториев
  --exec-allow ПРЕФИКС       разрешённый префикс команды для --exec (вместо набора по умолчанию; можно несколько раз)
  --exec-timeout СЕК         предел времени одной команды (по умолчанию 120)
  --color auto|always|never  цвета в тексте (по умолчанию auto)
  --list-rules               список правил
  --explain ПРАВИЛО          описание правила
  -V, --version              версия
  -h, --help                 справка

Файл настроек agentsmd-lint.json в корне: {\"ignore\": [\"vague-instruction\"], \"ignorePaths\": [\"vendor/\"], \"maxTokens\": 6000}.
В тексте инструкции: <!-- agentsmd-lint-disable-next-line правило --> и <!-- agentsmd-lint-disable правило -->.
Код выхода: 0 — нет замечаний выбранного уровня, 1 — есть, 2 — ошибка запуска.
";

fn fail(msg: &str) -> ExitCode {
    eprintln!("Ошибка: {msg}");
    ExitCode::from(2)
}

fn find_root(file: &Path) -> PathBuf {
    let start = file.canonicalize().unwrap_or_else(|_| file.to_path_buf());
    let mut cur = start
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let fallback = cur.clone();
    loop {
        if cur.join(".git").exists() {
            return cur;
        }
        if !cur.pop() {
            return fallback;
        }
    }
}

fn load_config(root: &Path, opts: &mut Options) -> Result<(), String> {
    let p = root.join("agentsmd-lint.json");
    let Ok(text) = std::fs::read_to_string(&p) else {
        return Ok(());
    };
    let j = json::parse(&text).map_err(|e| format!("agentsmd-lint.json: {e}"))?;
    let strings = |k: &str| -> Vec<String> {
        j.get(k)
            .and_then(Json::as_arr)
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    opts.ignore_rules.extend(strings("ignore"));
    opts.ignore_paths.extend(strings("ignorePaths"));
    if let Some(Json::Num(n)) = j.get("maxTokens") {
        if *n >= 1.0 {
            opts.max_tokens = *n as usize;
        }
    }
    Ok(())
}

fn real_main() -> Result<ExitCode, String> {
    let mut args = std::env::args().skip(1);
    let mut path: Option<String> = None;
    let mut opts = Options::default();
    let mut format = "text".to_string();
    let mut fail_on = "error".to_string();
    let mut color = "auto".to_string();
    let mut cli_ignore = Vec::new();
    let mut cli_ignore_paths = Vec::new();
    let mut cli_max_tokens: Option<usize> = None;
    while let Some(a) = args.next() {
        let mut val = |name: &str| {
            args.next()
                .ok_or_else(|| format!("{name} требует значение"))
        };
        match a.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                return Ok(ExitCode::SUCCESS);
            }
            "-V" | "--version" => {
                println!("agentsmd-lint {}", env!("CARGO_PKG_VERSION"));
                return Ok(ExitCode::SUCCESS);
            }
            "--list-rules" => {
                for r in RULES {
                    println!("{:<22} {:<8} {}", r.id, r.severity.as_str(), r.summary);
                }
                return Ok(ExitCode::SUCCESS);
            }
            "--explain" => {
                let id = val("--explain")?;
                return match report::explain(&id) {
                    Some(t) => {
                        print!("{t}");
                        Ok(ExitCode::SUCCESS)
                    }
                    None => Err(format!("нет правила `{id}`; список: --list-rules")),
                };
            }
            "--format" => format = val("--format")?,
            "--fail-on" => fail_on = val("--fail-on")?,
            "--color" => color = val("--color")?,
            "--max-tokens" => {
                cli_max_tokens = Some(
                    val("--max-tokens")?
                        .parse()
                        .map_err(|_| "--max-tokens: нужно целое число".to_string())?,
                )
            }
            "--ignore" => cli_ignore.push(val("--ignore")?),
            "--ignore-path" => cli_ignore_paths.push(val("--ignore-path")?),
            "--exec" => opts.exec = true,
            "--exec-allow" => opts.exec_allow.push(val("--exec-allow")?),
            "--exec-timeout" => {
                opts.exec_timeout_secs = val("--exec-timeout")?
                    .parse()
                    .map_err(|_| "--exec-timeout: нужно целое число секунд".to_string())?
            }
            s if s.starts_with('-') => {
                return Err(format!("неизвестный параметр {s}; справка: --help"))
            }
            _ => {
                if path.replace(a).is_some() {
                    return Err("указан больше одного пути".to_string());
                }
            }
        }
    }
    if !matches!(format.as_str(), "text" | "json" | "sarif") {
        return Err("--format: text, json или sarif".to_string());
    }
    if !matches!(fail_on.as_str(), "error" | "warning" | "info" | "never") {
        return Err("--fail-on: error, warning, info или never".to_string());
    }
    for r in &cli_ignore {
        if agentsmd_lint::rules::info(r).is_none() {
            return Err(format!("--ignore: нет правила `{r}`; список: --list-rules"));
        }
    }
    let target = PathBuf::from(path.unwrap_or_else(|| ".".to_string()));
    if !target.exists() {
        return Err(format!("{} не существует", target.display()));
    }
    let (root, only_file) = if target.is_file() {
        (find_root(&target), Some(target.clone()))
    } else {
        (target.clone(), None)
    };
    let repo =
        Repo::scan(&root).map_err(|e| format!("не удалось прочитать {}: {e}", root.display()))?;
    load_config(&repo.root, &mut opts)?;
    opts.ignore_rules.extend(cli_ignore);
    opts.ignore_paths.extend(cli_ignore_paths);
    if let Some(n) = cli_max_tokens {
        opts.max_tokens = n;
    }
    let files: Vec<String> = match only_file {
        Some(f) => {
            let abs = f.canonicalize().map_err(|e| e.to_string())?;
            let rel = abs
                .strip_prefix(&repo.root)
                .map_err(|_| "файл вне корня репозитория".to_string())?;
            vec![rel.to_string_lossy().replace('\\', "/")]
        }
        None => discover(&repo),
    };
    let files = select_files(files, &opts);
    if files.is_empty() {
        println!("Файлов-инструкций не найдено (AGENTS.md, CLAUDE.md, GEMINI.md, .cursor/rules, .github/copilot-instructions.md).");
        return Ok(ExitCode::SUCCESS);
    }
    let diags = lint(&repo, &files, &opts);
    let use_color = match color.as_str() {
        "always" => true,
        "never" => false,
        _ => std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
    };
    match format.as_str() {
        "json" => print!("{}", report::json(&diags, files.len())),
        "sarif" => print!("{}", report::sarif(&diags, env!("CARGO_PKG_VERSION"))),
        _ => print!("{}", report::text(&diags, files.len(), use_color)),
    }
    let threshold = match fail_on.as_str() {
        "error" => Some(Severity::Error),
        "warning" => Some(Severity::Warning),
        "info" => Some(Severity::Info),
        _ => None,
    };
    let bad = threshold.is_some_and(|t| diags.iter().any(|d| d.severity >= t));
    Ok(if bad {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

fn main() -> ExitCode {
    match real_main() {
        Ok(c) => c,
        Err(e) => fail(&e),
    }
}
