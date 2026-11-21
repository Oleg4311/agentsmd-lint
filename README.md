# agentsmd-lint

[![CI](https://github.com/Oleg4311/agentsmd-lint/actions/workflows/ci.yml/badge.svg)](https://github.com/Oleg4311/agentsmd-lint/actions/workflows/ci.yml)
[Релизы](https://github.com/Oleg4311/agentsmd-lint/releases) · [Проверка на 10 реальных репозиториях](docs/real-world.md)

**Линтер файлов-инструкций для ИИ-агентов** (AGENTS.md, CLAUDE.md, GEMINI.md, `.cursor/rules`, `.github/copilot-instructions.md`). Он не оценивает «хороший ли это промпт»: он **сверяет написанное с репозиторием**. В инструкции сказано `npm run biuld`, а скрипта нет; указан путь, которого давно нет; ссылка ведёт в никуда; написано «Node 18», а проект требует 20; `pnpm install` при lockfile от npm; два указания взаимно исключают друг друга; в файле лежит ключ. Агент прочитает всё это как правду и либо потратит ходы, либо сделает не то.

Один бинарник на Rust без зависимостей. Формат вывода: текст, JSON, **SARIF** (для вкладки Code scanning на GitHub). Режим `--exec` по желанию **запускает** команды test/lint/build из инструкции и показывает те, что на самом деле падают.

*English: lints agent instruction files (AGENTS.md, CLAUDE.md, Cursor/Copilot rules) by checking their claims against the real repository — missing npm scripts and make targets, dead links and paths, `@imports`, package manager vs lockfile, stale Node/Go/Python versions, size budget, secrets, contradictions. Zero-dependency Rust CLI, text/JSON/SARIF output, optional `--exec` to run the documented commands. See the [English section](#english).*

## Пример

Репозиторий [examples/demo](examples/demo) с намеренно неточным [AGENTS.md](examples/demo/AGENTS.md):

```text
$ agentsmd-lint examples/demo
AGENTS.md
     6:1   предупреждение pm-lockfile-mismatch  в инструкции `pnpm`, а lockfile в репозитории только для npm
     7:1   предупреждение missing-script  команда `npm run biuld`, но скрипта `biuld` нет в package.json
            → возможно, имелся в виду `build`; в package.json есть: build, test, lint
    11:10  предупреждение stale-version  в тексте Node 18, а в .nvmrc/.node-version Node 20
    11:38  предупреждение broken-path  упомянут путь `src/main.ts`, но файла с таким путём нет в репозитории
    12:36  ошибка dead-link  ссылка ведёт на `docs/arch.md`, но такого файла нет
    17:1   предупреждение contradiction  предписываются взаимоисключающие `tabs` и `spaces` (строки 16 и 17)
    18:3   замечание vague-instruction  «write clean code» не задаёт проверяемого действия
    20:1   замечание duplicate-line  строка повторяет строку 19
Проверено файлов: 1. Найдено: 1 ошибка, 7 предупреждений, 3 замечания.
```

([полный вывод](docs/demo-output.txt), он же закреплён тестом.)

## Установка и запуск

Бинарники для Linux, macOS и Windows лежат на странице [Releases](https://github.com/Oleg4311/agentsmd-lint/releases) (контрольные суммы и подтверждение происхождения: `gh attestation verify agentsmd-lint-v0.1.0-linux-amd64.tar.gz --repo Oleg4311/agentsmd-lint`). Или из исходников: `cargo install --git https://github.com/Oleg4311/agentsmd-lint`.

```bash
agentsmd-lint                       # проверить репозиторий в текущем каталоге
agentsmd-lint path/to/AGENTS.md     # один файл (корень репозитория находится по .git)
agentsmd-lint --format sarif > agentsmd.sarif
agentsmd-lint --fail-on warning     # код выхода 1 и на предупреждениях (по умолчанию только на ошибках)
agentsmd-lint --exec                # запустить команды test/lint/build из инструкции (только для доверенных репозиториев)
agentsmd-lint --list-rules          # все правила;  --explain dead-link — одно
```

Код выхода: `0` нет замечаний выбранного уровня, `1` есть, `2` ошибка запуска. Настройки: файл `agentsmd-lint.json` в корне (`{"ignore": ["vague-instruction"], "ignorePaths": ["vendor/"], "maxTokens": 8000}`) и параметры командной строки. Подавление в тексте: `<!-- agentsmd-lint-disable-next-line broken-path -->` и `<!-- agentsmd-lint-disable правило -->`.

### GitHub Actions с вкладкой Code scanning

```yaml
- run: |
    gh release download v0.1.0 --repo Oleg4311/agentsmd-lint --pattern 'agentsmd-lint-*-linux-amd64.tar.gz'
    tar xzf agentsmd-lint-*-linux-amd64.tar.gz --strip-components=1
  env: { GH_TOKEN: "${{ github.token }}" }
- run: ./agentsmd-lint --format sarif --fail-on never > agentsmd.sarif
- uses: github/codeql-action/upload-sarif@v3        # закрепите по SHA в своём репозитории
  with: { sarif_file: agentsmd.sarif }
```

## Что проверяется

| Правило | Уровень | Что ловит |
|---|---|---|
| `dead-link` | ошибка | ссылка на локальный файл, которого нет |
| `link-root-relative` | предупреждение | ссылка записана от корня, но считается от каталога файла (на GitHub сломана); подсказка: `/путь` |
| `import-missing` | ошибка | `@файл` (импорт Claude Code) указывает на несуществующий файл |
| `import-cycle`, `import-depth` | ошибка / предупр. | импорты по кругу; цепочка глубже 5 (Claude Code дальше не раскрывает) |
| `broken-path` | предупреждение | упомянутого пути или файла нет в репозитории |
| `missing-script` | предупреждение | `npm/pnpm/yarn/bun run X`, `make X`, `just X`, где `X` нет в `package.json`, Makefile, justfile (с подсказкой «имелось в виду…») |
| `no-manifest` | предупреждение | команда требует `package.json`, Makefile, `Cargo.toml`, `go.mod`, а их нет |
| `pm-lockfile-mismatch`, `pm-field-mismatch`, `pm-inconsistent` | предупреждение | менеджер пакетов не совпадает с lockfile или `packageManager`; в одном файле разные менеджеры |
| `stale-version` | предупреждение | Node, Go или Python в тексте ниже того, что требует проект (`.nvmrc`, `engines`, `go.mod`, `pyproject.toml`) |
| `too-long` | предупреждение | файл вместе с импортами больше предела токенов (по умолчанию 6000; байты / 4) |
| `secret` | ошибка | ключ AWS, токен GitHub или Slack, `sk-…`, ключ Google, приватный ключ (в отчёте значение скрыто) |
| `contradiction` | предупреждение | «всегда X» и «никогда X»; взаимоисключающие `tabs`/`spaces`, `npm`/`pnpm`/`yarn`/`bun` (на английском и русском) |
| `vague-instruction` | замечание | «write clean code», «будь внимателен» и подобное без проверяемого действия |
| `duplicate-line`, `duplicate-files` | замечание | повтор строки; AGENTS.md и CLAUDE.md дублируют друг друга (достаточно `@AGENTS.md`) |
| `no-commands` | замечание | в корневом файле нет ни одной команды |
| `command-failed`, `command-timeout` | ошибка / предупр. | только с `--exec`: команда из инструкции завершилась ошибкой или не уложилась во время |

### Режим `--exec`

Выполняет **код репозитория**, поэтому включается только явным флагом. Запускаются лишь команды из белого списка префиксов (`npm test`, `npm run lint|build|typecheck|check`, `pnpm|yarn|bun` то же, `cargo test|check|clippy|fmt|build`, `go test|vet|build`, `make|just test|lint|build|check`), без оболочечных конструкций, с пределом времени (`--exec-timeout`, 120 с) и в каталоге, указанном в инструкции (`cd web && npm test`). Свой список: `--exec-allow "npm run e2e"` (заменяет набор по умолчанию).

## Как устроено

- **Markdown без зависимостей** ([src/md.rs](src/md.rs)): ограждённые блоки, встроенный код, ссылки, front matter; встроенный код не разбирается как ссылки и импорты.
- **Команды** ([src/shell.rs](src/shell.rs)): из блоков `sh` и встроенного кода, с цепочками `&&`/`;`/`|`, продолжением строк, переменными окружения, `sudo` и **отслеживанием `cd`**: `cd web && npm run dev` проверяется по `web/package.json`.
- **Факты о репозитории** ([src/repo.rs](src/repo.rs)): дерево файлов (без `node_modules`, `target`, `.git`), упрощённый `.gitignore`, `package.json`, Makefile (цели, шаблонные правила, `include`), justfile (рецепты, алиасы), версии.
- **JSON** ([src/json.rs](src/json.rs)): собственный разборщик, чтобы не тянуть зависимости; сверяется с `serde_json` на 9000 случайных документах и их порче (принятие и результат совпадают, глубина вложенности как у serde_json).
- **Отчёты**: текст, JSON, SARIF 2.1.0 ([src/report.rs](src/report.rs)).

## Что проверено

- **81 тест**: правила на временных репозиториях (по положительному и отрицательному случаю на правило, плюс охрана от ложных срабатываний), CLI (коды выхода, форматы, конфигурация, режим одного файла, BOM и CRLF), SARIF (структура и согласованность `ruleIndex`), режим `--exec` (успех, ошибка с выводом, превышение времени), золотой тест на [examples/demo](examples/demo).
- **Сверка собственного JSON-разборщика с `serde_json`** (proptest): валидные документы, испорченные документы, произвольный текст, предел глубины.
- **Мутации**: пять поломок (проверка ссылок, поиск скрипта, смещение комментария подавления, обнаружение цикла импортов, отслеживание `cd`) тесты ловят.
- **10 реальных репозиториев** (ruff, biome, langchain, pydantic, svelte, react-router, astro, anthropic-sdk-typescript, deno, VS Code; 57 файлов): [результаты с разбором каждой находки](docs/real-world.md). Итог: 28 замечаний, 12 однозначно верных, 7 спорных, 9 ложных; почти весь шум в одном правиле `broken-path`. Правила исправлены по итогам проверки (на девяти репозиториях было 31 замечание, осталось 5), список изменений в том же документе.
- CI: `cargo fmt`, `clippy -D warnings`, тесты на Linux/Windows/macOS, сборка на минимальной версии Rust 1.82, линтер проверяет собственный [AGENTS.md](AGENTS.md).

## Честные ограничения

- Это эвристики над текстом. Каталог запуска команды выводится только из `cd` в той же цепочке или блоке; фраза «из каталога build» в прозе не понимается. Для монорепозиториев это источник ложных срабатываний.
- `broken-path` самое шумное правило: описания поведения программы («создаёт `workspace.yaml`») отличить от ссылок на файлы нельзя наверняка. Поэтому это предупреждение с несколькими защитами и способом подавления; на реальных репозиториях правило ошибалось чаще остальных (см. [результаты](docs/real-world.md)).
- Версии и противоречия определяются по шаблонам фраз на английском и русском. «Use spaces in YAML» рядом с «use tabs» помечается как возможное противоречие, даже если области разные.
- Токены считаются грубо (байты / 4); `.gitignore` понимается упрощённо (имена, каталоги, `*.ext`, пути от корня).
- Проверяются только команды npm/pnpm/yarn/bun, make, just, cargo и go; `cargo` и `go` проверяются лишь на наличие манифеста, а не на подкоманды.
- Сообщения на русском. Файлы инструкций могут быть на любом языке.

## Разработка

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings
cargo test                           # 81 тест; режим --exec использует Node.js
cargo run -- examples/demo
```

Лицензия: MIT. Сделано с помощью Claude.

---

## English

**agentsmd-lint** checks agent instruction files (AGENTS.md, CLAUDE.md, GEMINI.md, `.cursor/rules`, `.github/copilot-instructions.md`) against the real repository instead of grading the prose: missing `npm run` scripts / `make` / `just` targets (with "did you mean"), dead links and `@imports` (cycles, depth > 5), mentioned paths that do not exist, package manager vs lockfile / `packageManager`, stale Node/Go/Python versions, size budget including imports, secrets (masked in output), contradictions (EN + RU), vague filler, duplicate files. `cd` in command chains is tracked, so `cd web && npm run dev` is checked against `web/package.json`.

Zero runtime dependencies (own JSON parser, differentially tested against `serde_json`), text / JSON / **SARIF 2.1.0** output, `--fail-on`, config file and inline suppression comments, and an opt-in `--exec` that runs allow-listed test/lint/build commands from the file and reports the ones that fail.

Validated on 10 public repositories (57 instruction files): 28 findings — 12 clearly right, 7 debatable, 9 false (mostly the `broken-path` heuristic) — documented in [docs/real-world.md](docs/real-world.md) along with the fixes it prompted. 81 tests, mutation-checked, release binaries carry SHA-256 sums and build provenance. MIT licensed. Built with Claude.
