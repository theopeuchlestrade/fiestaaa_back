# Fiestaaa Back

Backend Rust de Fiestaaa, une application d'organisation d'événements privés.

L'API gère l'authentification, les événements, invitations, listes d'items,
covoiturages, frais partagés, QR codes d'accès, notifications et flux temps réel.

## Stack

- Rust 1.90
- Actix Web
- PostgreSQL via SQLx
- Redis pour certains états éphémères
- Docker Compose pour le développement local

## Prérequis

- Docker CLI + Docker Compose v2
- Rust, si vous lancez l'API hors Docker
- Une copie locale de `.env.example` vers `.env`

## Configuration

```bash
cp .env.example .env
```

Les valeurs de `.env.example` sont des placeholders ou des valeurs de
développement local. Les secrets réels ne doivent jamais être commités.

Variables importantes :

- `DATABASE_URL` : connexion PostgreSQL
- `JWT_SECRET` : secret de signature des sessions
- `DATA_ENCRYPTION_KEY` et `DATA_LOOKUP_KEY` : clés applicatives, au moins 32 caractères
- `CORS_ALLOWED_ORIGINS` : origines front autorisées
- `APP_BASE_URL` : URL du front pour les liens d'invitation
- `RESEND_API_KEY` et `INVITATION_EMAIL_SENDER` : envoi d'emails d'invitation
- `FCM_*` et `FIESTAAA_FCM_VAPID_KEY` : notifications push

## Développement local

Lancement complet avec Postgres :

```bash
docker compose up --build
```

API locale :

```text
http://127.0.0.1:8080
```

Pour lancer l'API avec `cargo`, démarrez seulement la base :

```bash
docker compose up -d db
cargo run
```

Dans ce mode, gardez une URL locale de type :

```bash
DATABASE_URL=postgres://postgres:postgres@localhost:5432/fiestaaa
```

## Utilisateur local

Pour créer ou mettre à jour un utilisateur local directement en base :

```bash
cargo run --bin create_local_user -- --email test@local.dev --password changeme --handle test_local
```

La commande hash le mot de passe avec Argon2 et supprime une éventuelle
inscription en attente pour le même email.

## Base de données

Les migrations SQL sont dans `migrations/` et sont appliquées au démarrage via
`sqlx::migrate!`.

Réinitialisation locale :

```bash
docker compose down -v
docker compose up --build
```

Ou reconstruction directe depuis le schéma courant :

```bash
./scripts/rebuild_db_from_schema.sh
```

## Qualité et tests

Format :

```bash
cargo fmt --all --check
```

Lint :

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

Tests avec Docker :

```bash
docker compose run --rm api cargo test
```

Suite CI équivalente, avec une base de test disponible :

```bash
TEST_DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5432/fiestaaa_test cargo test --locked --all-targets --jobs 1 -- --test-threads=1
```

## Déploiement

La documentation de déploiement et d'exploitation est dans
`docs/deploiement.md`.

Le passage des dépôts privés vers des dépôts publics est documenté dans
`docs/passage-public-open-source.md`.

## Sécurité

Ne signalez pas de vulnérabilité via une issue publique. Consultez
`SECURITY.md` pour le canal de signalement et les attentes de divulgation.

Avant toute publication publique du dépôt, relancez un scan de secrets sur
l'état courant et sur tout l'historique Git.

## Licence

`fiestaaa_back` est distribué sous licence `AGPL-3.0-only`. Voir `LICENSE`.
