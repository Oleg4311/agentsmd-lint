mod common;
use agentsmd_lint::{Options, Severity};
use common::*;

const PKG: &str =
    r#"{"name":"app","scripts":{"build":"tsc","test":"node --test","lint":"eslint ."}}"#;

#[test]
fn dead_link_found_with_position_and_valid_links_pass() {
    let fx = Fx::new();
    fx.write("docs/guide.md", "# g\n").write("AGENTS.md", "# A\nSee [guide](docs/guide.md#x), [gone](docs/missing.md), [web](https://a.dev/x.md), [top](#a), [enc](docs/guide%2Emd).\n");
    let d = fx.lint();
    let dead = find(&d, "dead-link");
    assert_eq!(dead.len(), 1, "{d:?}");
    assert_eq!((dead[0].line, dead[0].severity), (2, Severity::Error));
    assert!(dead[0].message.contains("docs/missing.md"));
}

#[test]
fn links_resolve_relative_to_the_file_and_from_root_for_slash() {
    let fx = Fx::new();
    fx.write("pkg/README.md", "x")
        .write(
            "pkg/AGENTS.md",
            "[r](README.md) [up](../top.md) [abs](/pkg/README.md)\n",
        )
        .write("top.md", "x");
    assert_eq!(count(&fx.lint(), "dead-link"), 0);
}

#[test]
fn import_missing_cycle_and_depth() {
    let fx = Fx::new();
    fx.write(
        "AGENTS.md",
        "# A\n@docs/a.md and @docs/nope.md\nmail me at x@example.com\n",
    )
    .write("docs/a.md", "see @b.md\n")
    .write("docs/b.md", "see @a.md\n");
    let d = fx.lint();
    let miss = find(&d, "import-missing");
    assert_eq!(miss.len(), 1, "{d:?}");
    assert!(miss[0].message.contains("docs/nope.md"));
    assert_eq!(count(&d, "import-cycle"), 1, "{d:?}");

    let fx = Fx::new();
    fx.write("AGENTS.md", "@l1.md\n");
    for i in 1..=6 {
        fx.write(&format!("l{i}.md"), &format!("@l{}.md\n", i + 1));
    }
    fx.write("l7.md", "end\n");
    assert_eq!(count(&fx.lint(), "import-depth"), 1);

    let fx = Fx::new();
    fx.write("AGENTS.md", "@l1.md\n")
        .write("l1.md", "@l2.md\n")
        .write("l2.md", "end\n");
    assert_eq!(count(&fx.lint(), "import-depth"), 0);
}

#[test]
fn imports_inside_code_are_not_imports_and_home_imports_are_skipped() {
    let fx = Fx::new();
    fx.write(
        "CLAUDE.md",
        "use `@nope.md` literally\n```\n@also/nope.md\n```\n@~/.claude/mine.md\n",
    );
    assert_eq!(count(&fx.lint(), "import-missing"), 0);
}

#[test]
fn broken_paths_and_the_cases_that_must_not_be_flagged() {
    let fx = Fx::new();
    fx.write("src/lib.rs", "").write("packages/web/src/app.ts", "").write(".gitignore", "secrets/\n*.local\n").write("AGENTS.md", "Edit `src/lib.rs`, `src/gone.rs`, `src/app.ts` and `docs/`. Not: `dist/main.js`, `.env`, `secrets/key.json`, `x.local`, `https://a/b.md`, `src/**/*.ts`, `foo.bar`, `lib.rs`, `ghost.json`, `src/lib.rs:12`.\n");
    let d = fx.lint();
    let broken: Vec<&str> = find(&d, "broken-path")
        .iter()
        .map(|x| x.message.as_str())
        .collect();
    assert_eq!(broken.len(), 3, "{broken:?}");
    assert!(broken.iter().any(|m| m.contains("src/gone.rs")));
    assert!(broken.iter().any(|m| m.contains("docs/")));
    assert!(broken.iter().any(|m| m.contains("ghost.json")));
    // `src/app.ts` есть как packages/web/src/app.ts (монорепозиторий), `lib.rs` есть где-то в дереве
}

#[test]
fn missing_script_with_suggestion_and_correct_scripts() {
    let fx = Fx::new();
    fx.write("package.json", PKG).write("AGENTS.md", "```sh\nnpm run biuld\nnpm run build\nnpm test\nnpm run lint -- --fix\nnpm run deploy\n```\n");
    let d = fx.lint();
    let m = find(&d, "missing-script");
    assert_eq!(m.len(), 2, "{d:?}");
    assert!(
        m[0].hint.as_deref().unwrap().contains("`build`"),
        "{:?}",
        m[0].hint
    );
    assert!(m[1].message.contains("deploy"));
}

#[test]
fn npm_test_without_script_and_npm_start_with_server_js() {
    let fx = Fx::new();
    fx.write("package.json", r#"{"scripts":{"build":"x"}}"#)
        .write("AGENTS.md", "`npm test` and `npm start`\n");
    assert_eq!(count(&fx.lint(), "missing-script"), 2);
    fx.write("server.js", "");
    let d = fx.lint();
    assert_eq!(count(&d, "missing-script"), 1);
    assert!(find(&d, "missing-script")[0].message.contains("`test`"));
}

#[test]
fn cd_selects_the_right_package_json() {
    let fx = Fx::new();
    fx.write("package.json", r#"{"scripts":{"build":"x"}}"#)
        .write("web/package.json", r#"{"scripts":{"dev":"x"}}"#)
        .write(
            "AGENTS.md",
            "```sh\nnpm run dev\ncd web && npm run dev\nnpm run dev\n```\n",
        );
    let d = fx.lint();
    let m = find(&d, "missing-script");
    assert_eq!(m.len(), 1, "{d:?}");
    assert_eq!(
        m[0].line, 2,
        "в корне скрипта dev нет; после cd web (он действует и на следующие строки) всё в порядке"
    );
}

#[test]
fn pnpm_and_yarn_run_scripts_directly_and_builtins_are_not_scripts() {
    let fx = Fx::new();
    fx.write("package.json", PKG)
        .write("pnpm-lock.yaml", "")
        .write(
            "AGENTS.md",
            "```sh\npnpm build\npnpm install\npnpm exec tsc\npnpm shipit\n```\n",
        );
    let d = fx.lint();
    let m = find(&d, "missing-script");
    assert_eq!(m.len(), 1, "{d:?}");
    assert!(m[0].message.contains("shipit"));
}

#[test]
fn workspace_flags_are_not_checked_and_bun_files_are_not_scripts() {
    let fx = Fx::new();
    fx.write("package.json", PKG).write("AGENTS.md", "```sh\nnpm run nope -w web\npnpm --filter api run nope\nbun run src/index.ts\nbun test\n```\n");
    assert_eq!(count(&fx.lint(), "missing-script"), 0);
}

#[test]
fn no_manifest_for_node_make_cargo_go() {
    let fx = Fx::new();
    fx.write("AGENTS.md", "```sh\nnpm test\nmake build\ncargo test\ngo test ./...\ncargo install foo\ngo version\n```\n");
    let d = fx.lint();
    assert_eq!(count(&d, "no-manifest"), 4, "{d:?}");
    let fx = Fx::new();
    fx.write("package.json", PKG)
        .write("Makefile", "build:\n\techo\n")
        .write("Cargo.toml", "")
        .write("go.mod", "module x\n")
        .write(
            "AGENTS.md",
            "```sh\nnpm test\nmake build\ncargo test\ngo test ./...\n```\n",
        );
    assert_eq!(count(&fx.lint(), "no-manifest"), 0);
}

#[test]
fn make_targets_pattern_rules_and_includes() {
    let fx = Fx::new();
    fx.write(
        "Makefile",
        ".PHONY: all\nall: build test\nbuild:\n\tgo build\ntest lint:\n\tgo test\nVAR := x\n",
    )
    .write(
        "AGENTS.md",
        "`make build` `make lint` `make all` `make release` `make -j4 test`\n",
    );
    let d = fx.lint();
    let m = find(&d, "missing-script");
    assert_eq!(m.len(), 1, "{d:?}");
    assert!(m[0].message.contains("release"));
    fx.write("Makefile", "%.o: %.c\n\tcc\n");
    assert_eq!(
        count(&fx.lint(), "missing-script"),
        0,
        "с шаблонными правилами цели не проверяются"
    );
    fx.write("Makefile", "include other.mk\nbuild:\n\tx\n");
    assert_eq!(
        count(&fx.lint(), "missing-script"),
        0,
        "с include список целей неполон"
    );
}

#[test]
fn just_recipes() {
    let fx = Fx::new();
    fx.write("justfile", "set shell := [\"sh\"]\nname := \"x\"\n\nbuild:\n  cargo build\n\ntest arg='x': build\n  cargo test\n").write("AGENTS.md", "`just build` `just test` `just ship` `just --list`\n");
    let d = fx.lint();
    let m = find(&d, "missing-script");
    assert_eq!(m.len(), 1, "{d:?}");
    assert!(m[0].message.contains("ship"));
}

#[test]
fn package_manager_vs_lockfile_field_and_mixed_use() {
    let fx = Fx::new();
    fx.write("package.json", PKG)
        .write("package-lock.json", "{}")
        .write("AGENTS.md", "```sh\npnpm install\npnpm build\n```\n");
    let d = fx.lint();
    assert_eq!(
        count(&d, "pm-lockfile-mismatch"),
        1,
        "один раз на менеджера: {d:?}"
    );

    let fx = Fx::new();
    fx.write(
        "package.json",
        r#"{"packageManager":"yarn@4.1.0","scripts":{"build":"x"}}"#,
    )
    .write("AGENTS.md", "`npm run build`\n");
    assert_eq!(count(&fx.lint(), "pm-field-mismatch"), 1);

    let fx = Fx::new();
    fx.write("package.json", PKG)
        .write("AGENTS.md", "`npm run build` then later `yarn lint`\n");
    assert_eq!(count(&fx.lint(), "pm-inconsistent"), 1);

    let fx = Fx::new();
    fx.write("package.json", PKG)
        .write("pnpm-lock.yaml", "")
        .write("AGENTS.md", "```sh\npnpm install\npnpm build\n```\n");
    let d = fx.lint();
    assert_eq!(
        count(&d, "pm-lockfile-mismatch")
            + count(&d, "pm-inconsistent")
            + count(&d, "pm-field-mismatch"),
        0
    );
}

#[test]
fn stale_versions_against_nvmrc_engines_gomod_python() {
    let fx = Fx::new();
    fx.write(".nvmrc", "v20.11.0\n")
        .write("package.json", PKG)
        .write("AGENTS.md", "Needs Node 18. Also Node.js 20 is fine.\n");
    let d = fx.lint();
    assert_eq!(count(&d, "stale-version"), 1, "{d:?}");
    assert!(find(&d, "stale-version")[0].message.contains("Node 18"));

    let fx = Fx::new();
    fx.write("package.json", r#"{"engines":{"node":">=20"}}"#)
        .write("AGENTS.md", "Node 18 required.\nNode 22 recommended.\n");
    assert_eq!(
        count(&fx.lint(), "stale-version"),
        1,
        "выше минимума — не устаревшее"
    );

    let fx = Fx::new();
    fx.write("go.mod", "module x\n\ngo 1.22.3\n")
        .write("AGENTS.md", "Written for Go 1.19 and Go 1.23.\n");
    assert_eq!(count(&fx.lint(), "stale-version"), 1);

    let fx = Fx::new();
    fx.write(
        "pyproject.toml",
        "[project]\nrequires-python = \">=3.11\"\n",
    )
    .write("AGENTS.md", "Use Python 3.9.\n");
    assert_eq!(count(&fx.lint(), "stale-version"), 1);

    let fx = Fx::new();
    fx.write("AGENTS.md", "Nodes 18 and go 2 times and python 3 rocks\n");
    assert_eq!(count(&fx.lint(), "stale-version"), 0);
}

#[test]
fn too_long_counts_imported_files() {
    let fx = Fx::new();
    fx.write("AGENTS.md", "@big.md\n")
        .write("big.md", &"word ".repeat(6000));
    let d = fx.lint();
    let t = find(&d, "too-long");
    assert_eq!(t.len(), 1, "{d:?}");
    assert!(t[0].message.contains("импортированными"));
    let opts = Options {
        max_tokens: 50_000,
        ..Options::default()
    };
    assert_eq!(count(&fx.lint_with(&opts), "too-long"), 0);
}

#[test]
fn secrets_are_errors_and_masked() {
    let fx = Fx::new();
    fx.write("CLAUDE.md", "# k\nDeploy key: AKIAABCDEFGHIJKLMNOP\n```\nexport T=ghp_abcdefghijklmnopqrstuvwxyz0123456789\n```\nuse sk-... placeholder\n");
    let d = fx.lint();
    let s = find(&d, "secret");
    assert_eq!(s.len(), 2, "{d:?}");
    assert!(s.iter().all(|x| x.severity == Severity::Error));
    assert!(s.iter().all(|x| !x.message.contains("ABCDEFGHIJKLMNOP")
        && !x.message.contains("abcdefghijklmnopqrstuvwxyz")));
}

#[test]
fn contradictions_and_non_contradictions() {
    let fx = Fx::new();
    fx.write("AGENTS.md", "- Always use tabs for indentation.\n- Use spaces in YAML.\n- Never use any in new code.\n- Always use any when unsure.\n- Prefer pnpm.\n- Avoid yarn.\n- Don't use var.\n");
    let d = fx.lint();
    let c = find(&d, "contradiction");
    assert_eq!(c.len(), 2, "tabs/spaces и any: {d:?}");
    // «prefer pnpm» + «avoid yarn» согласованы
    assert!(c.iter().all(|x| !x.message.contains("yarn")));
}

#[test]
fn russian_directives() {
    let fx = Fx::new();
    fx.write(
        "AGENTS.md",
        "- Всегда используй pnpm.\n- Не используй pnpm в скриптах.\n",
    );
    assert_eq!(count(&fx.lint(), "contradiction"), 1);
}

#[test]
fn vague_duplicates_and_missing_commands() {
    let fx = Fx::new();
    fx.write("AGENTS.md", "# Rules\n- Write clean code and follow best practices.\n- Keep the public API stable and documented in the README.\n- keep the public api stable and documented in the readme.\n");
    let d = fx.lint();
    assert_eq!(count(&d, "vague-instruction"), 2, "{d:?}");
    assert_eq!(count(&d, "duplicate-line"), 1);
    assert_eq!(count(&d, "no-commands"), 1);
    fx.write(
        "AGENTS.md",
        "# Rules\nRun `npm test` before finishing, and keep the API stable.\n",
    );
    assert_eq!(count(&fx.lint(), "no-commands"), 0);
}

#[test]
fn duplicate_files_unless_linked() {
    let body = "# Project\n- Keep the public API stable and documented.\n- Run the full test suite before you finish.\n- Prefer small focused commits with clear messages.\n";
    let fx = Fx::new();
    fx.write("AGENTS.md", body).write("CLAUDE.md", body);
    assert_eq!(count(&fx.lint(), "duplicate-files"), 1);
    fx.write("CLAUDE.md", "@AGENTS.md\n");
    assert_eq!(count(&fx.lint(), "duplicate-files"), 0);
    fx.write(
        "CLAUDE.md",
        "# Other\n- Something entirely different here for the agent.\n",
    );
    assert_eq!(count(&fx.lint(), "duplicate-files"), 0);
}

#[test]
fn suppression_comments_and_options() {
    let fx = Fx::new();
    fx.write("AGENTS.md", "<!-- agentsmd-lint-disable-next-line dead-link -->\n[a](nope.md)\n[b](nope2.md)\nWrite clean code.\n");
    let d = fx.lint();
    assert_eq!(count(&d, "dead-link"), 1);
    assert_eq!(find(&d, "dead-link")[0].line, 3);

    fx.write("AGENTS.md", "<!-- agentsmd-lint-disable dead-link, vague-instruction -->\n[a](nope.md)\nWrite clean code.\n");
    let d = fx.lint();
    assert_eq!(count(&d, "dead-link") + count(&d, "vague-instruction"), 0);

    fx.write(
        "AGENTS.md",
        "<!-- agentsmd-lint-disable -->\n[a](nope.md)\n",
    );
    assert!(fx.lint().is_empty());

    fx.write("AGENTS.md", "[a](nope.md)\nWrite clean code.\n");
    let o = Options {
        ignore_rules: vec!["dead-link".into()],
        ..Options::default()
    };
    assert_eq!(rules(&fx.lint_with(&o)), ["vague-instruction"]);
    let o = Options {
        ignore_paths: vec!["AGENTS.md".into()],
        ..Options::default()
    };
    assert!(fx.lint_with(&o).is_empty());
}

#[test]
fn discovery_covers_nested_and_tool_specific_files() {
    let fx = Fx::new();
    fx.write("AGENTS.md", "x\n")
        .write("pkg/api/CLAUDE.md", "x\n")
        .write(
            ".cursor/rules/style.mdc",
            "---\nglobs: *.ts\n---\n[x](gone.md)\n",
        )
        .write(".github/copilot-instructions.md", "[y](gone2.md)\n")
        .write("node_modules/dep/AGENTS.md", "[z](gone3.md)\n")
        .write("README.md", "[n](gone4.md)\n");
    let d = fx.lint();
    let files: std::collections::BTreeSet<&str> = d.iter().map(|x| x.file.as_str()).collect();
    assert!(
        files.contains(".cursor/rules/style.mdc")
            && files.contains(".github/copilot-instructions.md"),
        "{files:?}"
    );
    assert!(!files
        .iter()
        .any(|f| f.contains("node_modules") || *f == "README.md"));
}

#[test]
fn exec_runs_only_allowed_commands_and_reports_failures_and_timeouts() {
    let fx = Fx::new();
    fx.write("ok.js", "process.exit(0)\n")
        .write("fail.js", "console.error('boom');process.exit(3)\n")
        .write("slow.js", "setInterval(()=>{},1000)\n")
        .write(
            "AGENTS.md",
            "```sh\nnode ok.js\nnode fail.js\nnode slow.js\nrm -rf nothing\n```\n",
        );
    let opts = Options {
        exec: true,
        exec_allow: vec!["node".into()],
        exec_timeout_secs: 2,
        ..Options::default()
    };
    let d = fx.lint_with(&opts);
    let failed = find(&d, "command-failed");
    assert_eq!(failed.len(), 1, "{d:?}");
    assert!(failed[0].message.contains("fail.js") && failed[0].message.contains("кодом 3"));
    assert!(failed[0].hint.as_deref().unwrap().contains("boom"));
    assert_eq!(count(&d, "command-timeout"), 1);
    // без --exec ничего не запускается
    assert_eq!(count(&fx.lint(), "command-failed"), 0);
}

#[test]
fn false_positive_guards_found_on_real_repositories() {
    // justfile: алиасы считаются командами
    let fx = Fx::new();
    fx.write(
        "justfile",
        "alias f := format
format:
  x
",
    )
    .write(
        "AGENTS.md",
        "Run `just f` and `just g`.
",
    );
    let d = fx.lint();
    assert_eq!(count(&d, "missing-script"), 1, "{d:?}");
    assert!(find(&d, "missing-script")[0].message.contains("`g`"));

    // голые имена исходников и системных заголовков — примеры, а не ссылки; а конфигурацию проверяем
    let fx = Fx::new();
    fx.write(
        "AGENTS.md",
        "Routes like `_layout.tsx`, `foo.py`, `stdarg.h`, but `tsconfig.json` must exist.
",
    );
    let d = fx.lint();
    assert_eq!(count(&d, "broken-path"), 1, "{d:?}");
    assert!(find(&d, "broken-path")[0].message.contains("tsconfig.json"));

    // строки про создаваемые файлы и примеры не проверяются
    let fx = Fx::new();
    fx.write(
        "AGENTS.md",
        "Create a new file `docs/new-thing.md`. For example `src/example.rs`.
Edit `src/real.rs`.
",
    );
    let d = fx.lint();
    assert_eq!(count(&d, "broken-path"), 1, "{d:?}");
    assert!(find(&d, "broken-path")[0].message.contains("src/real.rs"));

    // глобальные установки и команды реестра не относятся к менеджеру проекта
    let fx = Fx::new();
    fx.write("package.json", PKG)
        .write("pnpm-lock.yaml", "")
        .write(
            "AGENTS.md",
            "`pnpm build`, then `npm install -g tool`, `npm publish`, `npm view x`.
",
        );
    let d = fx.lint();
    assert_eq!(
        count(&d, "pm-lockfile-mismatch") + count(&d, "pm-inconsistent"),
        0,
        "{d:?}"
    );

    // любая команда означает, что агенту есть чем проверять работу
    let fx = Fx::new();
    fx.write(
        "AGENTS.md",
        "# Project notes for the agent
Run `pnpm test:ecosystem` or `just f` before you push.
",
    );
    assert_eq!(count(&fx.lint(), "no-commands"), 0);

    // повтор строки со ссылкой — указатель, а не дублирующееся правило
    let fx = Fx::new();
    fx.write(
        "AGENTS.md",
        "See [the contributing guide](CONTRIBUTING.md) for details on building.
See [the contributing guide](CONTRIBUTING.md) for details on building.
",
    )
    .write("CONTRIBUTING.md", "x");
    assert_eq!(count(&fx.lint(), "duplicate-line"), 0);
}

#[test]
fn nested_instruction_files_may_use_root_scripts_and_root_makefile() {
    let fx = Fx::new();
    fx.write("package.json", PKG)
        .write("Makefile", "check:\n\tx\n")
        .write("pkg/package.json", r#"{"scripts":{"dev":"x"}}"#)
        .write(
            "pkg/AGENTS.md",
            "```sh\nnpm run build\nnpm run dev\nnpm run nowhere\nmake check\nmake nowhere\n```\n",
        );
    let d = fx.lint();
    let m: Vec<&str> = find(&d, "missing-script")
        .iter()
        .map(|x| x.message.as_str())
        .collect();
    assert_eq!(m.len(), 2, "{m:?}");
    assert!(m
        .iter()
        .any(|x| x.contains("nowhere") && x.contains("package.json")));
    assert!(m.iter().any(|x| x.contains("цели `nowhere`")));
}

#[test]
fn link_written_from_the_root_is_reported_as_such_not_as_dead() {
    let fx = Fx::new();
    fx.write("build/next/index.ts", "").write(
        ".github/instructions/a.instructions.md",
        "[x](build/next/index.ts) [y](build/nope.ts) [ok](/build/next/index.ts)\n",
    );
    let d = fx.lint();
    assert_eq!(count(&d, "link-root-relative"), 1, "{d:?}");
    assert_eq!(count(&d, "dead-link"), 1, "{d:?}");
}

#[test]
fn compiled_js_and_relative_prefixes_resolve_to_sources() {
    let fx = Fx::new();
    fx.write("src/vs/common/search.ts", "").write("src/vs/ui/panel.tsx", "").write("AGENTS.md", "See `../../common/search.ts`, `./ui/panel.js`, `vs/common/search.js`, but not `src/vs/gone.js`.\n");
    let d = fx.lint();
    let b = find(&d, "broken-path");
    assert_eq!(b.len(), 1, "{d:?}");
    assert!(b[0].message.contains("src/vs/gone.js"));
}

#[test]
fn pointer_files_are_not_asked_for_commands() {
    let fx = Fx::new();
    fx.write(".github/copilot-instructions.md", "# Real instructions\nRun `npm test`.\n").write("package.json", PKG).write("AGENTS.md", "# Agents\nPlease follow the guidance in [Copilot instructions](.github/copilot-instructions.md) for this repository.\n");
    assert_eq!(count(&fx.lint(), "no-commands"), 0);
}

#[test]
fn monorepo_without_root_manifest_is_not_flagged_but_empty_repo_is() {
    let fx = Fx::new();
    fx.write(
        "libs/core/Makefile",
        "test:
	x
",
    )
    .write("libs/web/package.json", PKG)
    .write(
        "AGENTS.md",
        "`make test` and `npm run build`
",
    );
    assert_eq!(count(&fx.lint(), "no-manifest"), 0);
    let fx = Fx::new();
    fx.write(
        "AGENTS.md",
        "`make test` and `npm run build`
",
    );
    assert_eq!(count(&fx.lint(), "no-manifest"), 2);
}

#[test]
fn lines_describing_runtime_output_are_not_checked_for_paths() {
    let fx = Fx::new();
    fx.write(
        "AGENTS.md",
        "The build writes `out.manifest.json` and stores `cache/` data.
See `config.json`.
",
    );
    let d = fx.lint();
    assert_eq!(count(&d, "broken-path"), 1, "{d:?}");
}
