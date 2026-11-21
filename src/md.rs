//! Лёгкий сканер Markdown: ограждённые блоки кода, встроенный код, ссылки. Полный разбор CommonMark не нужен:
//! линтеру важно понимать, где команды, где пути, а где обычный текст.

#[derive(Debug, Clone)]
pub struct Fence {
    pub lang: String,
    /// Номер строки с открывающей оградой (с 1).
    pub start: usize,
    pub body: Vec<(usize, String)>,
}

#[derive(Debug, Clone)]
pub struct Span {
    pub line: usize,
    pub col: usize,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct Link {
    pub line: usize,
    pub col: usize,
    pub target: String,
}

#[derive(Debug, Default)]
pub struct Doc {
    pub lines: Vec<String>,
    pub fences: Vec<Fence>,
    pub spans: Vec<Span>,
    pub links: Vec<Link>,
    /// Строки вне блоков кода и front matter; встроенный код заменён пробелами (позиции символов сохранены).
    pub prose: Vec<(usize, String)>,
}

fn fence_open(line: &str) -> Option<(char, usize, String)> {
    let t = line.trim_start_matches(' ');
    if line.len() - t.len() > 3 {
        return None;
    }
    let c = t.chars().next()?;
    if c != '`' && c != '~' {
        return None;
    }
    let n = t.chars().take_while(|&x| x == c).count();
    if n < 3 {
        return None;
    }
    let info: String = t.chars().skip(n).collect();
    if c == '`' && info.contains('`') {
        return None;
    }
    Some((c, n, info.trim().to_string()))
}

fn fence_close(line: &str, c: char, n: usize) -> bool {
    let t = line.trim_start_matches(' ');
    if line.len() - t.len() > 3 {
        return false;
    }
    let run = t.chars().take_while(|&x| x == c).count();
    run >= n && t.chars().skip(run).all(char::is_whitespace)
}

/// Возвращает встроенные фрагменты кода строки и саму строку с пустотой на их месте.
fn inline_code(line: &str, lineno: usize) -> (Vec<Span>, String) {
    let ch: Vec<char> = line.chars().collect();
    let mut masked = ch.clone();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < ch.len() {
        if ch[i] != '`' {
            i += 1;
            continue;
        }
        let n = ch[i..].iter().take_while(|&&c| c == '`').count();
        let mut j = i + n;
        let mut close = None;
        while j < ch.len() {
            if ch[j] == '`' {
                let m = ch[j..].iter().take_while(|&&c| c == '`').count();
                if m == n {
                    close = Some(j);
                    break;
                }
                j += m;
            } else {
                j += 1;
            }
        }
        match close {
            Some(j) => {
                let mut text: String = ch[i + n..j].iter().collect();
                if text.len() > 1 && text.starts_with(' ') && text.ends_with(' ') {
                    text = text[1..text.len() - 1].to_string();
                }
                spans.push(Span {
                    line: lineno,
                    col: i + 1,
                    text,
                });
                for m in &mut masked[i..j + n] {
                    *m = ' ';
                }
                i = j + n;
            }
            None => i += n,
        }
    }
    (spans, masked.into_iter().collect())
}

fn links_in(masked: &str, lineno: usize, out: &mut Vec<Link>) {
    let ch: Vec<char> = masked.chars().collect();
    let mut i = 0;
    while i + 1 < ch.len() {
        if ch[i] == ']' && ch[i + 1] == '(' && ch[..i].contains(&'[') {
            let start = i + 2;
            let (target, _end) = if ch.get(start) == Some(&'<') {
                let e = ch[start + 1..]
                    .iter()
                    .position(|&c| c == '>')
                    .map(|p| start + 1 + p);
                match e {
                    Some(e) => (ch[start + 1..e].iter().collect::<String>(), e),
                    None => (String::new(), start),
                }
            } else {
                let mut depth = 0;
                let mut k = start;
                while k < ch.len() {
                    match ch[k] {
                        '(' => depth += 1,
                        ')' if depth == 0 => break,
                        ')' => depth -= 1,
                        c if c.is_whitespace() && depth == 0 => break,
                        _ => {}
                    }
                    k += 1;
                }
                (ch[start..k].iter().collect::<String>(), k)
            };
            if !target.is_empty() {
                out.push(Link {
                    line: lineno,
                    col: start + 1,
                    target,
                });
            }
            i = start;
        } else {
            i += 1;
        }
    }
    // определение ссылки: [id]: путь
    let t = masked.trim_start_matches(' ');
    if masked.len() - t.len() <= 3 && t.starts_with('[') {
        if let Some(close) = t.find("]:") {
            let rest = t[close + 2..].trim_start();
            let target: String = rest.chars().take_while(|c| !c.is_whitespace()).collect();
            if !target.is_empty() && !t[1..close].contains(']') {
                let col = masked.chars().count() - rest.chars().count() + 1;
                out.push(Link {
                    line: lineno,
                    col,
                    target: target.trim_matches(|c| c == '<' || c == '>').to_string(),
                });
            }
        }
    }
}

pub fn parse(text: &str) -> Doc {
    let lines: Vec<String> = text.lines().map(str::to_string).collect();
    let mut doc = Doc::default();
    let mut i = 0;
    // front matter (.mdc и обычный Markdown с YAML-шапкой)
    if lines.first().map(|l| l.trim_end()) == Some("---") {
        if let Some(end) = lines.iter().skip(1).position(|l| l.trim_end() == "---") {
            i = end + 2;
        }
    }
    while i < lines.len() {
        let line = &lines[i];
        if let Some((c, n, info)) = fence_open(line) {
            let mut body = Vec::new();
            let start = i + 1;
            i += 1;
            while i < lines.len() && !fence_close(&lines[i], c, n) {
                body.push((i + 1, lines[i].clone()));
                i += 1;
            }
            i += 1; // закрывающая ограда (или конец файла)
            let lang = info
                .split(|c: char| c.is_whitespace() || c == '{')
                .next()
                .unwrap_or("")
                .to_lowercase();
            doc.fences.push(Fence { lang, start, body });
            continue;
        }
        let (spans, masked) = inline_code(line, i + 1);
        doc.spans.extend(spans);
        links_in(&masked, i + 1, &mut doc.links);
        doc.prose.push((i + 1, masked));
        i += 1;
    }
    doc.lines = lines;
    doc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_fences_spans_and_links() {
        let d = parse("# T\nrun `npm test` and see [docs](docs/a.md#x).\n```bash\nnpm run build\n```\n~~~\nplain\n~~~\n![img](pic.png)\n");
        assert_eq!(d.fences.len(), 2);
        assert_eq!(d.fences[0].lang, "bash");
        assert_eq!(d.fences[0].body, vec![(4, "npm run build".to_string())]);
        assert_eq!(d.spans.len(), 1);
        assert_eq!(d.spans[0].text, "npm test");
        assert_eq!(d.spans[0].col, 5);
        let targets: Vec<_> = d.links.iter().map(|l| l.target.as_str()).collect();
        assert_eq!(targets, ["docs/a.md#x", "pic.png"]);
    }

    #[test]
    fn code_inside_fences_is_not_inline_code_or_links() {
        let d = parse("```\n`not span` [a](b.md)\n```\n");
        assert!(d.spans.is_empty() && d.links.is_empty());
    }

    #[test]
    fn double_backtick_spans_and_unclosed_runs() {
        let d = parse("a ``code with ` tick`` b `unclosed\n");
        assert_eq!(d.spans.len(), 1);
        assert_eq!(d.spans[0].text, "code with ` tick");
    }

    #[test]
    fn links_inside_code_spans_are_ignored_and_parens_balance() {
        let d = parse("`[x](y.md)` and [w](a_(b).md) and [t](<my file.md>)\n");
        let targets: Vec<_> = d.links.iter().map(|l| l.target.as_str()).collect();
        assert_eq!(targets, ["a_(b).md", "my file.md"]);
    }

    #[test]
    fn link_reference_definitions_and_front_matter() {
        let d = parse("---\nglobs: src/**/*.ts\n---\n[ref]: docs/guide.md \"Title\"\n");
        assert_eq!(d.links.len(), 1);
        assert_eq!(d.links[0].target, "docs/guide.md");
        assert!(d.prose.iter().all(|(n, _)| *n > 3));
    }

    #[test]
    fn unterminated_fence_runs_to_the_end() {
        let d = parse("```sh\nmake test\n");
        assert_eq!(d.fences.len(), 1);
        assert_eq!(d.fences[0].body.len(), 1);
    }

    #[test]
    fn longer_closing_fence_and_indent_limits() {
        let d = parse("````md\n```\ninner\n```\n````\n    ```\nnot a fence\n");
        assert_eq!(d.fences.len(), 1);
        assert_eq!(d.fences[0].body.len(), 3);
    }
}
