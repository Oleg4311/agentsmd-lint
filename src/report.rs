//! Вывод: человекочитаемый текст, JSON и SARIF 2.1.0 (для вкладки Code scanning на GitHub).

use crate::json::escape;
use crate::rules::{self, RULES};
use crate::{Diagnostic, Severity};
use std::collections::BTreeMap;

pub fn counts(d: &[Diagnostic]) -> (usize, usize, usize) {
    let c = |s| d.iter().filter(|x| x.severity == s).count();
    (c(Severity::Error), c(Severity::Warning), c(Severity::Info))
}

fn plural(n: usize, one: &str, few: &str, many: &str) -> String {
    let m10 = n % 10;
    let m100 = n % 100;
    let w = if m10 == 1 && m100 != 11 {
        one
    } else if (2..=4).contains(&m10) && !(12..=14).contains(&m100) {
        few
    } else {
        many
    };
    format!("{n} {w}")
}

pub fn text(d: &[Diagnostic], files_checked: usize, color: bool) -> String {
    let paint = |code: &str, s: &str| {
        if color {
            format!("\x1b[{code}m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    };
    let mut out = String::new();
    let mut by_file: BTreeMap<&str, Vec<&Diagnostic>> = BTreeMap::new();
    for x in d {
        by_file.entry(x.file.as_str()).or_default().push(x);
    }
    for (file, list) in by_file {
        out.push_str(&paint("1", file));
        out.push('\n');
        for x in list {
            let (label, code) = match x.severity {
                Severity::Error => ("ошибка", "31"),
                Severity::Warning => ("предупреждение", "33"),
                Severity::Info => ("замечание", "36"),
            };
            out.push_str(&format!(
                "  {:>4}:{:<3} {} {}  {}\n",
                x.line,
                x.col,
                paint(code, label),
                paint("2", x.rule),
                x.message
            ));
            if let Some(h) = &x.hint {
                out.push_str(&format!("            → {h}\n"));
            }
        }
        out.push('\n');
    }
    let (e, w, i) = counts(d);
    if d.is_empty() {
        out.push_str(&format!(
            "Проверено файлов: {files_checked}. Замечаний нет.\n"
        ));
    } else {
        out.push_str(&format!(
            "Проверено файлов: {files_checked}. Найдено: {}, {}, {}.\n",
            plural(e, "ошибка", "ошибки", "ошибок"),
            plural(w, "предупреждение", "предупреждения", "предупреждений"),
            plural(i, "замечание", "замечания", "замечаний"),
        ));
    }
    out
}

fn diag_json(d: &Diagnostic) -> String {
    format!(
        "{{\"rule\":\"{}\",\"severity\":\"{}\",\"file\":\"{}\",\"line\":{},\"column\":{},\"message\":\"{}\",\"hint\":{}}}",
        d.rule,
        d.severity.as_str(),
        escape(&d.file),
        d.line,
        d.col,
        escape(&d.message),
        d.hint.as_ref().map_or("null".to_string(), |h| format!("\"{}\"", escape(h)))
    )
}

pub fn json(d: &[Diagnostic], files_checked: usize) -> String {
    let (e, w, i) = counts(d);
    let items: Vec<String> = d.iter().map(diag_json).collect();
    format!(
        "{{\"version\":1,\"filesChecked\":{files_checked},\"summary\":{{\"errors\":{e},\"warnings\":{w},\"infos\":{i}}},\"diagnostics\":[{}]}}\n",
        items.join(",")
    )
}

pub fn sarif(d: &[Diagnostic], version: &str) -> String {
    let level = |s: Severity| match s {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "note",
    };
    let rules_json: Vec<String> = RULES
        .iter()
        .map(|r| {
            format!(
                "{{\"id\":\"{}\",\"shortDescription\":{{\"text\":\"{}\"}},\"defaultConfiguration\":{{\"level\":\"{}\"}},\"helpUri\":\"https://github.com/Oleg4311/agentsmd-lint#rules\"}}",
                r.id,
                escape(r.summary),
                level(r.severity)
            )
        })
        .collect();
    let results: Vec<String> = d
        .iter()
        .map(|x| {
            let idx = RULES.iter().position(|r| r.id == x.rule).unwrap_or(0);
            let msg = match &x.hint {
                Some(h) => format!("{} ({})", x.message, h),
                None => x.message.clone(),
            };
            format!(
                "{{\"ruleId\":\"{}\",\"ruleIndex\":{},\"level\":\"{}\",\"message\":{{\"text\":\"{}\"}},\"locations\":[{{\"physicalLocation\":{{\"artifactLocation\":{{\"uri\":\"{}\"}},\"region\":{{\"startLine\":{},\"startColumn\":{}}}}}}}]}}",
                x.rule,
                idx,
                level(x.severity),
                escape(&msg),
                escape(&x.file),
                x.line.max(1),
                x.col.max(1)
            )
        })
        .collect();
    format!(
        "{{\"$schema\":\"https://json.schemastore.org/sarif-2.1.0.json\",\"version\":\"2.1.0\",\"runs\":[{{\"tool\":{{\"driver\":{{\"name\":\"agentsmd-lint\",\"version\":\"{}\",\"informationUri\":\"https://github.com/Oleg4311/agentsmd-lint\",\"rules\":[{}]}}}},\"results\":[{}]}}]}}\n",
        escape(version),
        rules_json.join(","),
        results.join(",")
    )
}

pub fn explain(id: &str) -> Option<String> {
    let r = rules::info(id)?;
    Some(format!(
        "{} ({}): {}\n",
        r.id,
        r.severity.as_str(),
        r.summary
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(sev: Severity) -> Diagnostic {
        Diagnostic {
            rule: "dead-link",
            severity: sev,
            file: "A\"B.md".into(),
            line: 3,
            col: 4,
            message: "m\n2".into(),
            hint: Some("h".into()),
        }
    }

    #[test]
    fn plural_forms() {
        assert_eq!(plural(1, "ошибка", "ошибки", "ошибок"), "1 ошибка");
        assert_eq!(plural(3, "ошибка", "ошибки", "ошибок"), "3 ошибки");
        assert_eq!(plural(5, "ошибка", "ошибки", "ошибок"), "5 ошибок");
        assert_eq!(plural(11, "ошибка", "ошибки", "ошибок"), "11 ошибок");
        assert_eq!(plural(22, "ошибка", "ошибки", "ошибок"), "22 ошибки");
    }

    #[test]
    fn text_without_findings_and_with() {
        assert!(text(&[], 2, false).contains("Замечаний нет"));
        let t = text(&[d(Severity::Error)], 1, false);
        assert!(
            t.contains("3:4")
                && t.contains("ошибка")
                && t.contains("→ h")
                && t.contains("1 ошибка, 0 предупреждений")
        );
    }

    #[test]
    fn json_and_sarif_are_valid_json() {
        let list = [d(Severity::Error), d(Severity::Info)];
        for out in [json(&list, 1), sarif(&list, "0.1.0")] {
            assert!(crate::json::parse(&out).is_ok(), "{out}");
        }
    }
}
