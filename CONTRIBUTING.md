# Contributing to fiestaaa_back

Merci de contribuer au back.

## Prérequis
- Rust (toolchain stable)
- Docker + Docker Compose (recommandé pour Postgres)

## Installation
1. Copier `.env.example` en `.env` et ajuster si besoin.
2. Démarrer Postgres via Docker :
   - `docker compose up --build`

## Lancer l’API
- Docker (recommandé) : `docker compose up --build`
- Local : `docker compose up -d db` puis `cargo run` avec `DATABASE_URL` local.

## Pre-commit (hooks locaux)
Les hooks exécutent `cargo fmt` + `cargo clippy -D warnings`.

Installer les hooks :
- Depuis ce repo : `sh scripts/install-hooks.sh`
- Depuis la racine mono-repo : `sh scripts/install-hooks.sh`

Si besoin de bypass ponctuel : `SKIP_LINT=1 git commit ...`

## Lint / Format
- `cargo fmt --all`
- `cargo clippy --all-targets --all-features -- -D warnings`

## Tests
- Docker : `docker compose run --rm api cargo test`
- Local : `cargo test` (nécessite `TEST_DATABASE_URL` ou `DATABASE_URL`).

## Migrations
Les migrations SQL sont dans `migrations/` et appliquées au démarrage via `sqlx::migrate!`.

## PR / MR
- Décrire le contexte, le changement, et l’impact.
- Ajouter/mettre à jour les tests si applicable.
- Assurer que `cargo fmt` et `cargo clippy -D warnings` passent.
- Les vulnérabilités de sécurité ne doivent pas être remontées via une issue publique ; suivre `SECURITY.md`.
