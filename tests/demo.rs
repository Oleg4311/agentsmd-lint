//! Золотой тест: примеры из README должны давать ровно те замечания, что описаны.
use agentsmd_lint::repo::Repo;
use agentsmd_lint::{discover, lint, Options};

#[test]
fn demo_repository_produces_the_documented_findings() {
    let repo = Repo::scan(std::path::Path::new("examples/demo")).unwrap();
    let files = discover(&repo);
    assert_eq!(files, ["AGENTS.md"]);
    let d = lint(&repo, &files, &Options::default());
    let got: Vec<(&str, usize)> = d.iter().map(|x| (x.rule, x.line)).collect();
    let want = [
        ("pm-lockfile-mismatch", 6),
        ("missing-script", 7),
        ("pm-inconsistent", 7),
        ("stale-version", 11),
        ("broken-path", 11),
        ("broken-path", 11),
        ("dead-link", 12),
        ("contradiction", 17),
        ("vague-instruction", 18),
        ("vague-instruction", 18),
        ("duplicate-line", 20),
    ];
    let mut want_sorted: Vec<(&str, usize)> = want.to_vec();
    want_sorted.sort_by_key(|(r, l)| (*l, *r));
    let mut got_sorted = got.clone();
    got_sorted.sort_by_key(|(r, l)| (*l, *r));
    assert_eq!(got_sorted, want_sorted, "{d:#?}");
}
