# agentsmd-lint: instructions for contributors and agents

A zero-dependency Rust CLI that checks agent instruction files (AGENTS.md, CLAUDE.md, ...) against the real repository.

## Commands

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo run -- . --fail-on warning      # the tool lints its own instructions (config: agentsmd-lint.json)
```

## Layout

- `src/rules.rs`: all rules; each one checks a claim from the instructions against a fact about the repository.
- `src/md.rs` and `src/shell.rs`: the Markdown scanner and command extraction; `src/repo.rs`: repository facts.
- `tests/rules.rs`: one positive and one negative case per rule on throw-away repositories; `tests/demo.rs` pins the output for `examples/demo`.
- `docs/real-world.md`: results on public repositories; recheck it after changing a heuristic.

## Rules of the game

- A new rule needs a positive and a negative test, and a line in the README table and in `RULES` (`src/rules.rs`).
- Prefer a fact over a guess: if a heuristic cannot be verified against the repository, make it `info` or leave it out.
- Messages are in Russian, like the rest of the user-facing text; commit messages are in English.
- Do not add dependencies; `serde_json` and `proptest` are dev-only and used for differential tests.
