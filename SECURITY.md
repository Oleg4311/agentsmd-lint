# Политика безопасности

## Как сообщить об уязвимости

Пожалуйста, **не создавайте публичный issue**. Сообщите приватно через GitHub: вкладка **Security** → **Report a vulnerability**
([создать сообщение](https://github.com/Oleg4311/agentsmd-lint/security/advisories/new)).

Укажите, что именно не так, как это воспроизвести и какая версия затронута. Я постараюсь ответить в течение недели, но это проект одного человека, поэтому сроки не гарантирую.

## Поддерживаемые версии

Исправления выходят для последней версии: ветка `main` и последний релиз.

## Что считается уязвимостью

Линтер читает чужие репозитории и файлы-инструкции. Интересны: чтение или запись за пределами проверяемого каталога (например, через ссылки, импорты или символические ссылки), зависание или чрезмерный расход памяти на специально подготовленном репозитории или файле, утечка найденного секрета в отчёт (он должен быть скрыт), выполнение команд без флага `--exec` либо команд вне белого списка при `--exec`, внедрение разметки или управляющих символов в отчёт.

## Что вне области

Режим `--exec` по замыслу выполняет команды из белого списка в проверяемом репозитории: запускать его на недоверенном коде не нужно. Уже известные проблемы в зависимостях (их закрывает Dependabot) и отчёты без способа воспроизведения тоже вне области.

---

## English

Please report vulnerabilities privately via GitHub Security Advisories (**Security** → **Report a vulnerability**) and do not open a public issue. Include what is wrong, how to reproduce it and which version is affected. This is a one-person project, so response times are best-effort. Only the latest release and `main` receive fixes. `--exec` deliberately runs allow-listed commands from the repository under test: do not use it on untrusted code.
