# Installation

Ce chapitre vous amène de « aucun Suprnova sur cette machine » à un projet
scaffolé en fonctionnement. Si vous y êtes déjà, allez directement au
[Démarrage rapide](quickstart.md).

## Prérequis

- **Rust 1.94.0+** pour la branche `main` actuelle (le workspace utilise l’édition 2024). La version étiquetée v1.3.6 exige également Rust 1.94.0 au minimum. Installez-le via [rustup](https://rustup.rs/) :
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```
- **Node.js 20+** et **npm** (ou pnpm/yarn/bun) pour la chaîne d'outils
  frontend. Suprnova utilise Vite 8 et votre starter inclut TypeScript +
  Tailwind v4. Installez via [nodejs.org](https://nodejs.org/) ou votre
  gestionnaire de paquets.
- **Une bibliothèque cliente de base de données** qui correspond au driver que
  vous souhaitez utiliser :
  - SQLite - aucun extra nécessaire ; sqlite est fourni
  - PostgreSQL - `libpq` sur la plupart des systèmes (souvent pré-installé)
  - MySQL ou MariaDB - `libmariadb` / `libmysqlclient` sur la plupart des systèmes

Vous n'avez pas besoin de choisir une base de données maintenant. Le scaffolder
par défaut utilise SQLite pour qu'une application nouvelle fonctionne sans
configuration.


La branche `main` actuelle utilise SeaORM 2.0, SeaQuery 1.0 et SQLx 0.9. Les applications qui appellent directement SeaORM doivent importer `ExprTrait` pour les méthodes d’expression SeaQuery et utiliser des méthodes de connexion `*_raw` explicites pour les valeurs `Statement` préconstruites. La mise à niveau des dépendances ne nécessite aucune migration des données de l’application.

## Installer le CLI

Suprnova est distribué comme un projet Cargo, et l'installateur du
CLI tire le framework depuis git (pas depuis crates.io - voir la
[note de pré-lancement](#pre-launch-note) ci-dessous) :

```bash
cargo install --git https://github.com/eas4ai/suprnova.git --tag v1.3.6 suprnova-cli
```

Cela compile le binaire `suprnova` et le dépose dans `~/.cargo/bin`.
Vérifiez que cela a fonctionné :

```bash
suprnova --version
```

Vous devriez voir `suprnova 0.x.x`.

Si `suprnova` est introuvable, c'est que votre `~/.cargo/bin` n'est
pas dans le `PATH`. Ajoutez ceci à la configuration de votre shell :

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

## Créer un projet

`suprnova new` crée un projet complet - backend + frontend choisi + configuration Vite + migrations d'authentification + routes d'exemple.
Il est interactif par défaut :


```bash
suprnova new my-app
```

L'assistant vous demande, dans l'ordre :

1. **Nom du projet** - ignoré lorsque vous le passez en argument (`my-app`)
2. **Description** - utilisée dans `Cargo.toml`
3. **Auteur** - utilisé dans `Cargo.toml` ; par défaut votre `user.name` git
4. **Framework frontend** - l'un de `svelte` (défaut), `react`, `vue`

Si vous voulez ignorer les invites (CI, configuration en script), passez
`--no-interaction` et choisissez un frontend explicitement :

```bash
suprnova new my-app --frontend svelte --no-interaction
```

`--no-interaction` accepte les valeurs par défaut pour la description
(« Une application web construite avec Suprnova ») et l'auteur (vide). Pour
les définir, modifiez le `Cargo.toml` généré après le scaffolding.

Les trois options de frontend incluent chacune leurs propres starters
Svelte-5/runes-on, React-19 ou Vue-3.5. Les trois utilisent Inertia v3 +
Vite 8 + Tailwind v4 et pré-câblent un flux Connexion/Inscription/Tableau
de bord avec authentification par session.

Suprnova fournit également un starter d'API plus léger pour les backends de
service sans SPA :

```bash
suprnova new my-api --api
```

Le starter API n'a pas de couche frontend ou Inertia. Il initialise Magnetar sur la base de données de l'application, installe `BearerTokenMiddleware` et crée un scaffold pour l'enregistrement et la connexion par mot de passe contre `app_users`.

## Premier lancement

```bash
cd my-app

# Exécutez les migrations (users, sessions, etc.)
suprnova migrate

# Installez les dépendances du frontend
npm install              # à la racine du projet

# Démarrez le backend et Vite ensemble
suprnova serve
```

`suprnova serve` exécute le backend sur `http://127.0.0.1:8765` et Vite
sur `http://127.0.0.1:5765`. Accédez à l'URL du backend - Vite est mandataire
pour que vous n'ayez pas besoin de le visiter directement.

Vous devriez voir la page de bienvenue. Visitez ensuite `/register` pour créer
un compte et `/login` pour vous connecter.

## Ce qui a été créé

```
my-app/
├── Cargo.toml          # manifeste de la crate, deux cibles [[bin]]
├── .env                # config locale (URL de BD, app key, ports)
├── .env.example        # modèle pour ops/CI
├── .gitignore
├── cmd/
│   └── main.rs         # l'entrée du binaire ; appelle Application::new().run()
├── src/
│   ├── lib.rs          # câblage des modules
│   ├── bootstrap.rs    # enregistrement des services (l'analogue Suprnova des fournisseurs)
│   ├── routes.rs       # l'arbre de la macro routes!
│   ├── bin/
│   │   └── console.rs  # `cargo run --bin console <subcommand>`
│   ├── actions/        # contrôleurs invocables à méthode unique
│   ├── commands/       # handlers annotés `#[command]`
│   ├── config/         # sections de config typées (database, mail)
│   ├── controllers/    # home, auth, dashboard
│   ├── middleware/     # logging, authenticate
│   ├── migrations/     # migrateurs SeaORM (users, sessions, etc.)
│   └── models/         # structs `#[suprnova::model]` (user)
├── frontend/
│   ├── package.json
│   ├── vite.config.ts
│   ├── tsconfig.json
│   ├── index.html
│   └── src/
│       ├── main.{tsx,ts}
│       ├── app.css
│       ├── pages/
│       │   ├── Home, Dashboard
│       │   └── auth/{Login,Register}
│       └── types/
│           └── inertia-props.ts
└── public/
    └── assets/         # sortie du build de production de Vite
```

La visite complète des répertoires se trouve dans [Structure des répertoires](structure.md).

## Mettre à jour le CLI

Le CLI vit dans votre `~/.cargo/bin`. Pour passer à la dernière
version :

```bash
cargo install --force --git https://github.com/eas4ai/suprnova.git --tag v1.3.6 suprnova-cli
```

`--force` fait écraser le binaire existant par Cargo.

## Mettre à jour la version du framework de votre application

Une application scaffoldée dépend de la crate du framework
`suprnova` via une dépendance git dans `Cargo.toml` :

```toml
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.3.6" }
```

Pour récupérer les dernières modifications du framework :

```bash
cargo update -p suprnova
```

La dépendance git suit l'étiquette de version nommée. Mettez à jour
l'étiquette dans `Cargo.toml`, puis lancez `cargo update -p
suprnova` ; votre `Cargo.lock` enregistre le commit exact qu'elle a
résolu, si bien que les builds restent reproductibles entre les
mises à jour - il n'y a pas besoin d'épingler un `rev` à la main
dans `Cargo.toml`.

## Modèle de distribution

Suprnova est distribué par git, pas par crates.io - le framework comme le
CLI s'installent depuis GitHub. Chaque version fait l'objet d'une
publication GitHub étiquetée (p. ex. `v1.2.4`), et c'est de l'étiquette que
dépend votre application : un `Cargo.toml` scaffoldé épingle
`tag = "v1.3.6"`, et `Cargo.lock` enregistre le commit exact que cette
étiquette a résolu, si bien que les builds sont reproductibles jusqu'à ce
que vous décidiez d'en changer. La mise à jour est délibérée, jamais
accidentelle - incrémentez l'étiquette et lancez
`cargo update -p suprnova` ; la section sur la mise à jour de la version
du framework de votre application détaille la marche à suivre.

## Configuration de l'éditeur

Quelques extensions VS Code rendent l'expérience plus fluide :

- **rust-analyzer** - le serveur de langage Rust
- **Svelte for VS Code** (ou React/Vue si vous avez choisi ceux-ci)
- **Tailwind CSS IntelliSense**
- **Even Better TOML**

`rust-analyzer` indexera le projet à sa première ouverture ; comptez sur 1-2
minutes la première fois, puis de manière incrémentale.

## Suivant

- [Démarrage rapide](quickstart.md) - construire une petite application en 5 minutes
- [Structure des répertoires](structure.md) - ce qui se trouve dans chaque
  fichier généré par le scaffolder
- [Configuration](configuration.md) - l'histoire de `.env` et de la configuration
  typée
- [Routage](routing.md) - ajouter votre première route
