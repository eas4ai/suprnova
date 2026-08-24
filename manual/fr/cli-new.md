# suprnova new

`suprnova new` scaffolde un projet Suprnova - une crate Cargo fraîche
avec des contrôleurs, des routes, des migrations, une SPA Inertia, et
un flux d'auth fonctionnel déjà câblé. Exécutez-la une fois par
application, puis passez l'essentiel de votre temps dans
`suprnova serve`.

## Utilisation

```bash
suprnova new [name] [options]
```

Si `name` est omis, l'assistant interactif le demande. Le nom devient
le répertoire du projet, le nom du package Cargo (après mise en
snake_case), et le `APP_NAME` par défaut dans `.env`. Les noms doivent
être des lettres/chiffres ASCII/`-`/`_`, commencer par une lettre, ne
contenir aucun séparateur de chemin ni `..`, et compter 64 caractères
ou moins.

## Options

| Option | Description |
|---|---|
| `--frontend <svelte\|react\|vue>` | Choisit le framework SPA de manière non interactive. Entre en conflit avec `--api`. |
| `--api` | Scaffolde un projet JSON:API uniquement (pas d'Inertia, pas de SPA, auth par token au lieu de sessions). |
| `--no-interaction` | Ignore toutes les invites et utilise les valeurs par défaut (nom `my-suprnova-app`, frontend `svelte`, auteur/description vides). |
| `--no-git` | Ignore `git init` dans le nouveau projet. |
| `--with-portless` | Émet un `portless.json` pour que [`suprnova dev:tls`](dev-tls.md) puisse servir l'application à `https://<name>.localhost`. Optionnel ; ne change rien d'autre. |

## Mode interactif

```bash
suprnova new my-app
```

L'assistant pose quatre questions, dans cet ordre :

1. **Nom du projet** - par défaut l'argument du répertoire (`my-app`)
2. **Description** - utilisée comme description du package Cargo
3. **Auteur** - utilisé comme auteur du package Cargo ; par défaut
   votre `git config user.name <name@email>` si défini
4. **Framework frontend** - `Svelte (recommandé)`, `React`, ou `Vue`

Après confirmation, le scaffolder écrit le projet, exécute `git init`
(sauf `--no-git`), et affiche les prochaines étapes :

```
Backend  http://localhost:8765
Frontend http://localhost:5765
```

## Mode non interactif

Pour la CI, les dotfiles, ou une configuration scriptée, passez
`--no-interaction` plus les flags que vous voulez redéfinir :

```bash
suprnova new my-app --frontend svelte --no-interaction
```

Valeurs par défaut sous `--no-interaction` :

- Frontend : `svelte`
- Description : `"A web application built with Suprnova"`
- Auteur : vide
- Git : initialisé

Il n'existe pas de flags `--description` ou `--author` ; ces valeurs
ne se définissent que via les invites interactives, ou prennent leurs
valeurs par défaut.

## Projet API uniquement

Pour des backends de service sans SPA, utilisez `--api` :

```bash
suprnova new my-api --api
```

Le starter API est nettement plus petit : pas de répertoire `frontend/`, pas
d'Inertia, pas de vues d'auth, et une disposition mono-crate `src/main.rs`.
Il initialise Magnetar sur la connexion SeaORM partagée, crée le modèle
canonique `app_users`, installe `BearerTokenMiddleware`, et utilise
`Auth::password()` pour l'inscription et la connexion. `PASSKEY_RP_ID` et
`PASSKEY_RP_ORIGIN` sont lus par le bootstrap généré avec des valeurs par
défaut locales. Le starter inclut aussi un contrôleur `users` d'exemple et un
sérialiseur JSON `UserResource`, et se lie au port 8765 dans `.env`.

`--api` est mutuellement exclusif avec `--frontend` ; passer les deux
produit une erreur. Sous `--api`, seul le nom du projet est demandé -
les invites description/auteur/frontend sont ignorées.

## Ce qui est scaffoldé

Une visite complète des répertoires vit dans [Structure des
répertoires](structure.md) ; la version courte est :

- `cmd/main.rs` - point d'entrée du binaire ; appelle
  `Application::new()…run()`
- `src/` - contrôleurs, actions, commandes, config, middleware,
  modèles, migrations, plus `bootstrap.rs` et `routes.rs`. Le
  `bootstrap.rs` généré câble la chaîne de middleware globale -
  journalisation, session, locale, CSRF, analyse des includes - et
  appelle [`Inertia::install`](frontend-inertia-responses.md), qui ajoute les
  middlewares du protocole Inertia (`409` sur la version des assets,
  `302 → 303` sur les redirections non-GET). La version d'assets qu'il annonce
  vaut par défaut le hachage du manifeste de build Vite, si bien qu'un build
  frontend livré la modifie automatiquement - voir
  [Détection de version](frontend-inertia-responses.md). Le même appel
  épingle le frontend que vous
  avez scaffoldé, si bien que la coquille HTML charge le point d'entrée Vite de
  ce framework ; `.env` porte le `SUPRNOVA_FRONTEND` correspondant pour les
  générateurs du CLI lui-même.
- `src/bin/console.rs` - l'équivalent de `php artisan` par projet
- `frontend/` - Vite 8 + Tailwind v4 + le framework que vous avez
  choisi, avec les pages Home / Dashboard / Login / Register déjà
  câblées via Inertia
- `src/migrations/` - les tables `users`, `sessions` et
  `remember_tokens` prêtes à l'emploi
- `.env` - base de données SQLite par défaut, avec une `APP_KEY`
  fraîchement générée pour que l'application démarre sans
  intervention d'un opérateur
- `.gitignore`, `Cargo.toml`

### Pourquoi Suprnova diverge

Laravel est livré avec Blade et tire un frontend via Breeze/Jetstream
après coup. Suprnova prend l'autre chemin : `suprnova new` scaffolde
toujours soit une vraie SPA (Svelte/React/Vue sur Inertia), soit un
vrai projet JSON:API. Il n'y a pas de starter orienté moteur de
templates d'abord - si vous voulez du HTML rendu côté serveur, Tera
est disponible, mais ce n'est pas la forme par défaut et aucun chemin
du scaffolder ne place des vues à l'avant de votre application.

Le frontend par défaut est **Svelte 5** (runes activées), pas React.
Nous l'avons choisi parce que c'est le plus léger des trois à
l'exécution et le plus proche de la philosophie du framework, « les
gains à la compilation l'emportent sur l'astuce à l'exécution ». React
et Vue sont tout autant de première classe - prenez ce que votre
équipe connaît.

## Distribution

Le CLI lui-même est livré via git, pas crates.io (pré-lancement) :

```bash
cargo install --git https://github.com/eas4ai/suprnova.git --tag v1.3.0 suprnova-cli
```

`--force` sur la même commande met à jour une installation existante.
Les projets scaffoldés dépendent de la crate du framework de la même
façon - une dépendance git dans leur `Cargo.toml`, épinglée à
l'étiquette de la version courante. Voir
[Installation](installation.md) pour les prérequis complets de la
chaîne d'outils.

## Suivant

- [Installation](installation.md) - prérequis Rust/Node/BD et
  configuration de la chaîne d'outils
- [Structure des répertoires](structure.md) - ce que fait chaque
  fichier scaffoldé
- [Démarrage rapide](quickstart.md) - les 5 premières minutes après
  `suprnova new`
- [suprnova serve](cli-serve.md) - le lanceur de dev que vous
  utiliserez ensuite
- [Console](console.md) - `cargo run --bin console` et le système
  `#[command]`
