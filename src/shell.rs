//! Выделение команд из инструкций: блоки ```sh и встроенный код. Оболочку не исполняем и не разбираем целиком:
//! нужны простые команды, цепочки `&&`/`;`/`|` и отслеживание `cd`.

use crate::md::Doc;
use crate::repo::join_norm;

#[derive(Debug, Clone, PartialEq)]
pub struct Cmd {
    pub line: usize,
    pub col: usize,
    /// Каталог запуска относительно корня репозитория; None — неизвестен (после `cd ~`, `cd $VAR`).
    pub cwd: Option<String>,
    pub words: Vec<String>,
    /// Простая команда без `cd`/`sudo`/переменных окружения, как её видит исполнитель.
    pub text: String,
    pub from_fence: bool,
}

const SHELL_LANGS: &[&str] = &[
    "sh",
    "bash",
    "shell",
    "zsh",
    "console",
    "terminal",
    "shell-session",
    "sh-session",
    "shellscript",
    "fish",
];
pub const TOOLS: &[&str] = &["npm", "pnpm", "yarn", "bun", "make", "just", "cargo", "go"];

/// Делит строку на простые команды по `&&`, `||`, `;`, `|` вне кавычек.
pub fn split_chain(line: &str) -> Vec<String> {
    let ch: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut i = 0;
    while i < ch.len() {
        let c = ch[i];
        if let Some(q) = quote {
            cur.push(c);
            if c == q {
                quote = None;
            } else if c == '\\' && q == '"' && i + 1 < ch.len() {
                i += 1;
                cur.push(ch[i]);
            }
        } else if c == '\'' || c == '"' {
            quote = Some(c);
            cur.push(c);
        } else if c == '#' && (cur.is_empty() || cur.ends_with(char::is_whitespace)) {
            break; // комментарий
        } else if (c == '&' && ch.get(i + 1) == Some(&'&'))
            || (c == '|' && ch.get(i + 1) == Some(&'|'))
        {
            out.push(std::mem::take(&mut cur));
            i += 1;
        } else if c == ';' || c == '|' {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
        i += 1;
    }
    out.push(cur);
    out.into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Слова простой команды с учётом кавычек.
pub fn words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut has = false;
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => cur.push(c),
            None if c == '\'' || c == '"' => {
                quote = Some(c);
                has = true;
            }
            None if c == '\\' => {
                if let Some(n) = it.next() {
                    cur.push(n);
                    has = true;
                }
            }
            None if c.is_whitespace() => {
                if has || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            None => cur.push(c),
        }
    }
    if has || !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn is_env_assign(w: &str) -> bool {
    let Some(eq) = w.find('=') else { return false };
    eq > 0
        && w[..eq]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !w.starts_with(|c: char| c.is_ascii_digit())
}

fn new_cwd(cwd: &Option<String>, target: Option<&String>) -> Option<String> {
    let t = target?;
    if t.starts_with('~') || t.starts_with('$') || t == "-" || t.contains('$') {
        return None;
    }
    if t.starts_with('/') {
        return None; // абсолютные пути вне репозитория проверять нечем
    }
    join_norm(cwd.as_deref()?, t)
}

/// Обрабатывает одну логическую строку; `persist` — менять ли каталог для следующих строк (внутри блока кода).
fn process_line(
    text: &str,
    line: usize,
    col: usize,
    cwd: &mut Option<String>,
    persist: bool,
    from_fence: bool,
    out: &mut Vec<Cmd>,
) {
    let mut local = cwd.clone();
    for seg in split_chain(text) {
        let mut w = words(&seg);
        while w
            .first()
            .is_some_and(|x| x == "$" || x == ">" || x == "sudo" || x == "time" || is_env_assign(x))
        {
            w.remove(0);
        }
        if w.is_empty() {
            continue;
        }
        if w[0] == "cd" {
            local = new_cwd(&local, w.get(1));
            if persist {
                *cwd = local.clone();
            }
            continue;
        }
        out.push(Cmd {
            line,
            col,
            cwd: local.clone(),
            text: w.join(" "),
            words: w,
            from_fence,
        });
    }
}

pub fn extract(doc: &Doc, file_dir: &str) -> Vec<Cmd> {
    let mut out = Vec::new();
    for f in &doc.fences {
        let labeled = SHELL_LANGS.contains(&f.lang.as_str());
        if !labeled && !f.lang.is_empty() {
            continue;
        }
        let mut cwd = Some(file_dir.to_string());
        let mut pending = String::new();
        let mut pending_start: Option<(usize, usize)> = None;
        for (no, raw) in &f.body {
            let t = raw.trim_start();
            let col = raw.chars().count() - t.chars().count() + 1;
            let cont = t.trim_end().ends_with('\\');
            let piece = if cont {
                t.trim_end().trim_end_matches('\\')
            } else {
                t
            };
            if pending_start.is_none() {
                pending_start = Some((*no, col));
            }
            pending.push_str(piece);
            pending.push(' ');
            if cont {
                continue;
            }
            let (ln, cl) = pending_start.take().unwrap_or((*no, col));
            let text = std::mem::take(&mut pending);
            let text = text.trim();
            let (prompted, body) = match text.strip_prefix("$ ") {
                Some(b) => (true, b),
                None => (false, text),
            };
            if !labeled && !prompted {
                // блок без языка: берём только строки, начинающиеся с известной утилиты
                let first = body.split_whitespace().next().unwrap_or("");
                if !TOOLS.contains(&first) {
                    continue;
                }
            }
            process_line(body, ln, cl, &mut cwd, true, true, &mut out);
        }
    }
    for s in &doc.spans {
        let first = s.text.split_whitespace().next().unwrap_or("");
        if !TOOLS.contains(&first) && first != "cd" {
            continue;
        }
        // «npm», «go» и «make» без аргументов в прозе — чаще слова, чем команды; «make» с целью и остальные — команды
        if s.text.split_whitespace().count() < 2 {
            continue;
        }
        let mut cwd = Some(file_dir.to_string());
        process_line(&s.text, s.line, s.col, &mut cwd, false, false, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::md;

    fn cmds(src: &str) -> Vec<(String, Option<String>)> {
        extract(&md::parse(src), "")
            .into_iter()
            .map(|c| (c.text, c.cwd))
            .collect()
    }

    #[test]
    fn splits_chains_outside_quotes() {
        assert_eq!(
            split_chain("a && b; c | d || e # tail"),
            ["a", "b", "c", "d", "e"]
        );
        assert_eq!(split_chain("echo 'a && b' && ok"), ["echo 'a && b'", "ok"]);
    }

    #[test]
    fn tokenises_with_quotes_and_escapes() {
        assert_eq!(
            words(r#"npm run "my script" a\ b 'c d'"#),
            ["npm", "run", "my script", "a b", "c d"]
        );
    }

    #[test]
    fn fence_commands_follow_cd_and_skip_env_prompt_sudo() {
        let c = cmds("```bash\n$ cd web && FOO=1 npm test\nmake build\ncd ..\nnpm run lint\n```\n");
        assert_eq!(c[0], ("npm test".to_string(), Some("web".to_string())));
        assert_eq!(c[1], ("make build".to_string(), Some("web".to_string())));
        assert_eq!(c[2], ("npm run lint".to_string(), Some("".to_string())));
    }

    #[test]
    fn continuation_lines_are_joined() {
        let c = cmds("```sh\nnpm run \\\n  build\n```\n");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].0, "npm run build");
    }

    #[test]
    fn unlabeled_blocks_only_yield_known_tools_and_other_languages_are_skipped() {
        assert_eq!(cmds("```\nsrc/\n  lib.rs\nnpm test\n```\n").len(), 1);
        assert!(cmds("```json\n{\"a\": \"npm test\"}\n```\n").is_empty());
        assert!(cmds("```python\nnpm test\n```\n").is_empty());
    }

    #[test]
    fn inline_code_commands_need_arguments() {
        assert_eq!(
            cmds("use `npm run build` or just `make` or `go`\n").len(),
            1
        );
        let c = cmds("`cd api && go test ./...`\n");
        assert_eq!(
            c,
            vec![("go test ./...".to_string(), Some("api".to_string()))]
        );
    }

    #[test]
    fn unknown_cwd_after_cd_home() {
        let c = cmds("```sh\ncd ~/work\nnpm test\n```\n");
        assert_eq!(c[0].1, None);
    }
}
