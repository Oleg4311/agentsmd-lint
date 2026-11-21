//! Режим --exec: запускает команды проверки из инструкций (test, lint, build) и сообщает, какие из них на самом деле не работают.
//! Выполняет код репозитория, поэтому включается только явным флагом и только для команд из белого списка префиксов.

use crate::shell;
use crate::{Diagnostic, Instr, Options, Severity};
use std::collections::HashSet;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const DEFAULT_ALLOW: &[&str] = &[
    "npm test",
    "npm run test",
    "npm run lint",
    "npm run build",
    "npm run typecheck",
    "npm run check",
    "pnpm test",
    "pnpm lint",
    "pnpm build",
    "pnpm typecheck",
    "pnpm run test",
    "pnpm run lint",
    "pnpm run build",
    "yarn test",
    "yarn lint",
    "yarn build",
    "yarn typecheck",
    "yarn run test",
    "yarn run lint",
    "yarn run build",
    "bun test",
    "bun run test",
    "bun run lint",
    "bun run build",
    "cargo test",
    "cargo check",
    "cargo clippy",
    "cargo fmt",
    "cargo build",
    "go test",
    "go vet",
    "go build",
    "make test",
    "make lint",
    "make build",
    "make check",
    "just test",
    "just lint",
    "just build",
    "just check",
];

/// Команда подходит под префикс, если её слова начинаются с слов префикса.
pub fn allowed(words: &[String], allow: &[String]) -> bool {
    let defaults: Vec<String>;
    let list: &[String] = if allow.is_empty() {
        defaults = DEFAULT_ALLOW.iter().map(|s| s.to_string()).collect();
        &defaults
    } else {
        allow
    };
    list.iter().any(|p| {
        let pw: Vec<&str> = p.split_whitespace().collect();
        !pw.is_empty() && words.len() >= pw.len() && words.iter().zip(&pw).all(|(a, b)| a == b)
    })
}

/// Допустимы только безопасные символы: исполнение идёт через оболочку (на Windows нужен cmd для .cmd-файлов).
fn safe(words: &[String]) -> bool {
    words.iter().all(|w| {
        !w.is_empty()
            && w.chars().all(|c| {
                c.is_ascii_alphanumeric()
                    || matches!(c, '_' | '.' | '/' | ':' | '=' | '@' | '+' | ',' | '-')
            })
    })
}

fn tail(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join(" | ")
}

fn run_one(
    dir: &std::path::Path,
    words: &[String],
    timeout: Duration,
) -> Result<(i32, String), String> {
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.arg("/C").args(words);
        c
    } else {
        let mut c = Command::new(&words[0]);
        c.args(&words[1..]);
        c
    };
    cmd.current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("не удалось запустить: {e}"))?;
    let mut so = child.stdout.take();
    let mut se = child.stderr.take();
    let t1 = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(o) = so.as_mut() {
            let mut b = Vec::new();
            let _ = o.read_to_end(&mut b);
            s = String::from_utf8_lossy(&b).into_owned();
        }
        s
    });
    let t2 = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(o) = se.as_mut() {
            let mut b = Vec::new();
            let _ = o.read_to_end(&mut b);
            s = String::from_utf8_lossy(&b).into_owned();
        }
        s
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let out = format!(
                    "{}\n{}",
                    t2.join().unwrap_or_default(),
                    t1.join().unwrap_or_default()
                );
                return Ok((status.code().unwrap_or(-1), out));
            }
            Ok(None) => {
                if start.elapsed() > timeout {
                    if cfg!(windows) {
                        let _ = Command::new("taskkill")
                            .args(["/PID", &child.id().to_string(), "/T", "/F"])
                            .stdout(Stdio::null())
                            .stderr(Stdio::null())
                            .status();
                    }
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("timeout".to_string());
                }
                std::thread::sleep(Duration::from_millis(40));
            }
            Err(e) => return Err(format!("ошибка ожидания: {e}")),
        }
    }
}

pub fn run(repo: &crate::repo::Repo, instrs: &[Instr], opts: &Options, out: &mut Vec<Diagnostic>) {
    let mut done: HashSet<(String, String)> = HashSet::new();
    for f in instrs {
        for c in shell::extract(&f.doc, &f.dir) {
            let Some(cwd) = &c.cwd else { continue };
            if !allowed(&c.words, &opts.exec_allow) || !safe(&c.words) {
                continue;
            }
            if !done.insert((cwd.clone(), c.text.clone())) {
                continue;
            }
            let dir = repo.abs(cwd);
            if !dir.is_dir() {
                continue;
            }
            match run_one(&dir, &c.words, Duration::from_secs(opts.exec_timeout_secs)) {
                Ok((0, _)) => {}
                Ok((code, output)) => out.push(Diagnostic {
                    rule: "command-failed",
                    severity: Severity::Error,
                    file: f.rel.clone(),
                    line: c.line,
                    col: c.col,
                    message: format!("команда `{}` завершилась с кодом {code}", c.text),
                    hint: Some(format!("последние строки вывода: {}", tail(&output, 6))),
                }),
                Err(e) if e == "timeout" => out.push(Diagnostic {
                    rule: "command-timeout",
                    severity: Severity::Warning,
                    file: f.rel.clone(),
                    line: c.line,
                    col: c.col,
                    message: format!("команда `{}` не завершилась за {} с", c.text, opts.exec_timeout_secs),
                    hint: Some("агент будет ждать так же долго: ускорьте команду или укажите более быструю".to_string()),
                }),
                Err(e) => out.push(Diagnostic {
                    rule: "command-failed",
                    severity: Severity::Error,
                    file: f.rel.clone(),
                    line: c.line,
                    col: c.col,
                    message: format!("команду `{}` не удалось выполнить: {e}", c.text),
                    hint: None,
                }),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn default_allow_list_matches_prefixes_only() {
        assert!(allowed(&w("npm test"), &[]));
        assert!(allowed(&w("cargo test --all"), &[]));
        assert!(allowed(&w("go test ./..."), &[]));
        assert!(!allowed(&w("npm publish"), &[]));
        assert!(!allowed(&w("rm -rf /"), &[]));
        assert!(!allowed(&w("npm"), &[]));
        assert!(!allowed(&w("npm run deploy"), &[]));
    }

    #[test]
    fn custom_allow_replaces_defaults() {
        let allow = vec!["node".to_string()];
        assert!(allowed(&w("node -e 1"), &allow));
        assert!(!allowed(&w("npm test"), &allow));
    }

    #[test]
    fn unsafe_characters_are_rejected() {
        assert!(safe(&w("go test ./...")));
        assert!(!safe(&w("npm test ; rm")));
        assert!(!safe(&["npm".into(), "test>out".into()]));
        assert!(!safe(&["echo".into(), "$(whoami)".into()]));
        assert!(!safe(&["a".into(), "`b`".into()]));
    }
}
