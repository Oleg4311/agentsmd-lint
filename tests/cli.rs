mod common;
use common::Fx;
use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_agentsmd-lint"))
        .args(args)
        .output()
        .unwrap()
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn code(o: &Output) -> i32 {
    o.status.code().unwrap()
}

fn path(fx: &Fx) -> String {
    fx.root.to_string_lossy().into_owned()
}

#[test]
fn help_version_and_bad_usage() {
    let h = run(&["--help"]);
    assert_eq!(code(&h), 0);
    assert!(out(&h).contains("Использование"));
    assert!(out(&run(&["--version"])).starts_with("agentsmd-lint 0."));
    for bad in [
        &["--nope"][..],
        &["--format", "xml"],
        &["--fail-on", "x"],
        &["--ignore", "no-such-rule"],
        &["a", "b"],
        &["/definitely/not/here"],
        &["--max-tokens", "abc"],
    ] {
        let o = run(bad);
        assert_eq!(code(&o), 2, "{bad:?}");
        assert!(
            String::from_utf8_lossy(&o.stderr).contains("Ошибка"),
            "{bad:?}"
        );
    }
}

#[test]
fn list_rules_and_explain() {
    let l = out(&run(&["--list-rules"]));
    assert!(l.lines().count() >= 18);
    assert!(l.contains("dead-link") && l.contains("secret"));
    assert!(out(&run(&["--explain", "dead-link"])).contains("ссылка на локальный файл"));
    assert_eq!(code(&run(&["--explain", "nope"])), 2);
}

#[test]
fn no_instruction_files_is_not_an_error() {
    let fx = Fx::new();
    fx.write("README.md", "x");
    let o = run(&[&path(&fx)]);
    assert_eq!(code(&o), 0);
    assert!(out(&o).contains("не найдено"));
}

#[test]
fn exit_codes_follow_fail_on() {
    let fx = Fx::new();
    fx.write(
        "AGENTS.md",
        "Run `npm run build`. Write clean code.\n[x](gone.md)\n",
    );
    let p = path(&fx);
    assert_eq!(code(&run(&[&p])), 1, "ошибка dead-link при --fail-on error");
    assert_eq!(
        code(&run(&[&p, "--ignore", "dead-link"])),
        0,
        "остались предупреждения и замечания"
    );
    assert_eq!(
        code(&run(&[&p, "--ignore", "dead-link", "--fail-on", "warning"])),
        1
    );
    assert_eq!(
        code(&run(&[
            &p,
            "--ignore",
            "dead-link",
            "--ignore",
            "no-manifest",
            "--ignore",
            "vague-instruction",
            "--fail-on",
            "info"
        ])),
        0
    );
    assert_eq!(code(&run(&[&p, "--fail-on", "never"])), 0);
}

#[test]
fn text_report_has_positions_rule_ids_and_summary() {
    let fx = Fx::new();
    fx.write("AGENTS.md", "# T\n\nSee [x](gone.md).\n");
    let t = out(&run(&[&path(&fx), "--color", "never"]));
    assert!(
        t.contains("AGENTS.md")
            && t.contains("3:")
            && t.contains("dead-link")
            && t.contains("1 ошибка"),
        "{t}"
    );
    assert!(!t.contains('\x1b'));
    let c = out(&run(&[&path(&fx), "--color", "always"]));
    assert!(c.contains('\x1b'));
}

#[test]
fn json_report_parses_and_matches_summary() {
    let fx = Fx::new();
    fx.write("AGENTS.md", "[x](gone.md)\nWrite clean code.\n");
    let o = run(&[&path(&fx), "--format", "json"]);
    let v: serde_json::Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(v["filesChecked"], 1);
    assert_eq!(v["summary"]["errors"], 1);
    assert_eq!(v["summary"]["infos"], 1);
    let d = v["diagnostics"].as_array().unwrap();
    assert_eq!(d.len(), 2);
    assert_eq!(d[0]["rule"], "dead-link");
    assert_eq!(d[0]["line"], 1);
    assert!(d[0]["hint"].is_string());
}

#[test]
fn sarif_is_structurally_valid_and_consistent() {
    let fx = Fx::new();
    fx.write(
        "AGENTS.md",
        "[x](gone.md)\n```sh\nnpm test\n```\nkey AKIAABCDEFGHIJKLMNOP\n",
    );
    let v: serde_json::Value =
        serde_json::from_str(&out(&run(&[&path(&fx), "--format", "sarif"]))).unwrap();
    assert_eq!(v["version"], "2.1.0");
    let run0 = &v["runs"][0];
    let rules = run0["tool"]["driver"]["rules"].as_array().unwrap();
    assert!(rules.len() >= 18);
    let results = run0["results"].as_array().unwrap();
    assert!(results.len() >= 3);
    for r in results {
        let idx = r["ruleIndex"].as_u64().unwrap() as usize;
        assert_eq!(
            rules[idx]["id"], r["ruleId"],
            "ruleIndex указывает на то же правило"
        );
        assert!(["error", "warning", "note"].contains(&r["level"].as_str().unwrap()));
        let loc = &r["locations"][0]["physicalLocation"];
        assert_eq!(loc["artifactLocation"]["uri"], "AGENTS.md");
        assert!(loc["region"]["startLine"].as_u64().unwrap() >= 1);
        assert!(loc["region"]["startColumn"].as_u64().unwrap() >= 1);
    }
    let s = serde_json::to_string(&v).unwrap();
    assert!(
        !s.contains("ABCDEFGHIJKLMNOP"),
        "секрет не должен попадать в отчёт"
    );
}

#[test]
fn single_file_argument_finds_the_repo_root_via_git() {
    let fx = Fx::new();
    fx.write(".git/HEAD", "ref")
        .write("docs/guide.md", "x")
        .write(
            "pkg/AGENTS.md",
            "[g](../docs/guide.md) [m](../docs/none.md)\n",
        )
        .write("AGENTS.md", "[bad](zzz.md)\n");
    let file = fx.root.join("pkg").join("AGENTS.md");
    let o = run(&[&file.to_string_lossy(), "--format", "json"]);
    let v: serde_json::Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(v["filesChecked"], 1);
    assert_eq!(v["diagnostics"].as_array().unwrap().len(), 1);
    assert_eq!(v["diagnostics"][0]["file"], "pkg/AGENTS.md");
}

#[test]
fn config_file_ignores_rules_paths_and_sets_max_tokens() {
    let fx = Fx::new();
    fx.write("AGENTS.md", "[x](gone.md)\nWrite clean code.\n")
        .write("vendor/AGENTS.md", "[y](gone.md)\n")
        .write(
            "agentsmd-lint.json",
            r#"{"ignore":["vague-instruction"],"ignorePaths":["vendor/"],"maxTokens":10}"#,
        );
    let v: serde_json::Value =
        serde_json::from_str(&out(&run(&[&path(&fx), "--format", "json"]))).unwrap();
    let rules: Vec<&str> = v["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["rule"].as_str().unwrap())
        .collect();
    assert!(!rules.contains(&"vague-instruction"));
    assert!(v["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .all(|d| d["file"] == "AGENTS.md"));
    fx.write("agentsmd-lint.json", "{ nope");
    assert_eq!(code(&run(&[&path(&fx)])), 2);
}

#[test]
fn exec_flag_runs_commands_and_exit_code_reflects_failures() {
    let fx = Fx::new();
    fx.write("fail.js", "process.exit(5)\n")
        .write("AGENTS.md", "```sh\nnode fail.js\n```\n");
    let p = path(&fx);
    assert_eq!(
        code(&run(&[&p, "--ignore", "no-commands"])),
        0,
        "без --exec команда не запускается"
    );
    let o = run(&[
        &p,
        "--exec",
        "--exec-allow",
        "node",
        "--ignore",
        "no-commands",
    ]);
    assert_eq!(code(&o), 1);
    assert!(out(&o).contains("command-failed"));
}

#[test]
fn utf8_with_bom_and_crlf_files_are_handled() {
    let fx = Fx::new();
    fx.write(
        "AGENTS.md",
        "\u{feff}# T\r\nSee [x](gone.md).\r\n```sh\r\nnpm test\r\n```\r\n",
    );
    let v: serde_json::Value =
        serde_json::from_str(&out(&run(&[&path(&fx), "--format", "json"]))).unwrap();
    let d = v["diagnostics"].as_array().unwrap();
    assert!(d.iter().any(|x| x["rule"] == "dead-link" && x["line"] == 2));
    assert!(d
        .iter()
        .any(|x| x["rule"] == "no-manifest" && x["line"] == 4));
}
