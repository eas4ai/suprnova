# Console

Chaque projet Suprnova est livré avec un binaire `console` - le
dispatcher de commandes à l'exécution pour tout ce qui a besoin des
types compilés de l'app : seeders de base de données, élagueurs,
tâches de maintenance ponctuelles, tout ce que vous construiriez avec
le `php artisan` de Laravel. Les commandes sont soit des structs
typées qui `#[derive(Command)]` (construites par-dessus
`clap::Parser`), soit des fns async annotées avec `#[command]` ; le
framework les collecte via `inventory` au moment du link, si bien
qu'ajouter une nouvelle commande tient dans un seul fichier, sans
registre central à modifier. C'est l'analogue Suprnova de
`php artisan` - même script, même processus, même espace d'adressage,
se termine quand le handler retourne.

## Démarrage rapide

La forme recommandée utilise `#[derive(clap::Parser, Command)]` pour
des args typés :

```rust
use async_trait::async_trait;
use clap::Parser;
use suprnova::{Command, FrameworkError, TypedCommand};

#[derive(Parser, Command, Debug)]
#[console(name = "greet", description = "Print a friendly greeting")]
pub struct Greet {
    #[arg(short, long, default_value = "world")]
    pub name: String,

    #[arg(long, default_value_t = false)]
    pub loud: bool,
}

#[async_trait]
impl TypedCommand for Greet {
    async fn run(self) -> Result<(), FrameworkError> {
        let prefix = if self.loud { "HELLO" } else { "Hello" };
        println!("{prefix}, {}!", self.name);
        Ok(())
    }
}
```

Déposez cela dans `src/commands/greet.rs`, ajoutez `pub mod greet;` à
`src/commands/mod.rs`, et exécutez-la :

```bash
cargo run --bin console -- greet
# Hello, world!
cargo run --bin console -- greet --name Alice --loud
# HELLO, Alice!
cargo run --bin console -- greet --help
# (aide par sous-commande générée par clap, incluant les flags typés)
```

Aucun registre central à modifier. `#[derive(Command)]` soumet une
`CommandEntry { name, description, clap_builder, handler }` via
inventory ; le binaire console appelle
`suprnova::console::dispatch_argv_with_init(argv, init)`, qui
construit un arbre d'analyseur clap unique à partir de chaque entrée
enregistrée, exécute la closure `init` d'amorçage seulement quand une
vraie sous-commande correspond, et route les `ArgMatches` analysés
vers le bon handler.

### Le chemin plus simple : `Vec<String>` brut

Pour les commandes triviales qui n'ont pas besoin d'args typés,
l'attribut `#[command]` sur une fn async fonctionne aussi :

```rust
use suprnova::{command, FrameworkError};

#[command(name = "ping", description = "Smoke test")]
pub async fn ping(_args: Vec<String>) -> Result<(), FrameworkError> {
    println!("pong");
    Ok(())
}
```

Sous le capot, les deux chemins atterrissent dans le même registre
`CommandEntry` ; la forme brute utilise simplement une sous-commande
clap avec un `trailing_var_arg` pour capturer argv dans le
`Vec<String>`. Préférez la forme typée pour toute commande avec des
arguments - vous obtenez `--help` par commande, l'analyse de valeurs,
les valeurs par défaut, et les paires de flags courts/longs sans
écrire de parser à la main.

## Le binaire console

`suprnova new` scaffolde deux binaires dans chaque nouveau projet :

- **`<project>`** (`cmd/main.rs` ou `src/main.rs`) - le serveur HTTP,
  démarré par `cargo run` ou `suprnova serve`. De longue durée ; sert
  jusqu'à ce qu'il soit tué.
- **`console`** (`src/bin/console.rs`) - le dispatcher de commandes à
  l'exécution. Ponctuel ; se termine quand le handler retourne.

Le `main` du binaire console est petit et prévisible :

```rust
use std::process::ExitCode;

#[suprnova::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    // Expose la version de ce projet via `--version` / `--help`.
    // env! se résout vers la version de l'app de l'utilisateur, pas celle du framework.
    suprnova::console::set_version(env!("CARGO_PKG_VERSION"));

    let argv: Vec<String> = std::env::args().collect();
    let result = suprnova::console::dispatch_argv_with_init(argv, || async {
        my_app::config::register_all();
        my_app::bootstrap::register().await;
    })
    .await;

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}
```

Tokio s'exécute en saveur `current_thread` - il n'y a pas de travail à
paralléliser entre les cœurs dans une commande ponctuelle, et le pool
de workers du runtime multi-thread ne serait que du surcoût.

Deux choses à remarquer :

- **L'amorçage est paresseux.** La closure passée à
  `dispatch_argv_with_init` ne s'exécute que quand clap fait
  correspondre une vraie sous-commande enregistrée. `console --help`,
  `console --version`, l'absence de sous-commande, et les chemins
  d'erreur d'analyse la sautent tous - donc `console --help`
  fonctionne sur un checkout tout frais qui n'a pas encore
  `DATABASE_URL` défini.
- **`main` n'imprime pas les erreurs.** `dispatch_argv_with_init`
  possède tout le stderr visible par l'utilisateur - elle fait un
  `eprintln` du message d'erreur du handler (sauf si l'erreur est
  silencieuse, comme un échec d'analyse clap que clap a déjà imprimé)
  et imprime la sortie help / version / erreur d'analyse propre à
  clap. `main` est une pure traduction `Result → ExitCode` ; ajouter
  un `eprintln!` redondant imprimerait en double.

Si vous voulez qu'une commande particulière saute entièrement une
étape d'amorçage coûteuse, conditionnez l'étape elle-même à une
variable d'environnement plutôt que de faire transiter un flag «
amorçage paresseux » à travers le framework.

## Commandes intégrées

Le framework enregistre lui-même un petit ensemble de commandes. Lier
le framework dans un projet les récupère automatiquement.

| Commande      | Ce qu'elle fait                           |
|---------------|-------------------------------------------|
| `db:seed`     | Exécute chaque `Seeder` enregistré, dans l'ordre. Accepte `--class=<Name>` (ou un positionnel nu) pour exécuter un seul seeder nommé, à l'image de `php artisan db:seed --class=UserSeeder`. |
| `model:prune` | Parcourt le registre `PrunerEntry` et supprime de force chaque ligne que chaque scope `Prunable` / `MassPrunable` enregistré retourne. `--model=<Name>` restreint à un seul type ; `--pretend` rapporte le nombre de lignes sans en modifier aucune. |
| `--help` / `-h` | Liste les commandes disponibles ; le `--help` par sous-commande est construit par clap à partir des args typés. |
| `--version`   | Imprime la version enregistrée par `set_version` (typiquement le `CARGO_PKG_VERSION` de votre app). Entièrement omis si `set_version` n'a jamais été appelé. |

`db:seed` exécute tout ce que vous avez enregistré dans
`bootstrap::register()` avec `suprnova::seed::register::<MySeeder>()`.
Sur un registre vide, elle imprime un avertissement et retourne
`Ok(())` - invoquer `db:seed` avant d'enregistrer des seeders est une
erreur d'utilisateur bénigne, pas une erreur de programmeur.

> Les daemons de worker (`queue:work`, `schedule:run`,
> `schedule:work`, `schedule:list`, `workflow:work`) ne sont **pas**
> sur le binaire console. Ils vivent sur l'analyseur clap du binaire
> app/serveur (le même binaire qui sert HTTP). Le CLI global
> `suprnova` shelle vers `cargo run --quiet -- <name>` pour ceux-là.
> Voir la [section sur l'asymétrie](#asymétrie-avec-suprnova-migrate)
> ci-dessous.

## Définir des commandes

Deux macros, un seul registre. Choisissez celle qui correspond à la
forme de la commande.

### `#[derive(Command)]` - args typés (recommandé)

Se place par-dessus `#[derive(clap::Parser)]`. Les champs de la struct
sont les args de la commande ; clap analyse argv dans la struct ; le
framework appelle votre `TypedCommand::run(self)`.

```rust
use async_trait::async_trait;
use clap::Parser;
use suprnova::{Command, FrameworkError, TypedCommand};

#[derive(Parser, Command, Debug)]
#[console(name = "users:purge", description = "Purge users older than N days")]
pub struct UsersPurge {
    #[arg(long)]
    pub older_than_days: u32,

    #[arg(long, default_value_t = false)]
    pub dry_run: bool,
}

#[async_trait]
impl TypedCommand for UsersPurge {
    async fn run(self) -> Result<(), FrameworkError> {
        // self.older_than_days, self.dry_run - typés, validés par clap
        Ok(())
    }
}
```

Attributs :

| Attribut    | Requis | Objectif                                       |
|--------------|----------|-----------------------------------------------|
| `#[console(name = "...")]` | oui | Le nom d'invocation sur le CLI (`"users:purge"`, `"mail:send"`, `"greet"`). |
| `#[console(description = "...")]` | non | Description sur une ligne affichée dans l'aide de premier niveau. |
| `#[arg(...)]` (clap) | n/a | Les propres attributs de champ de clap pour les flags courts/longs, les valeurs par défaut, les parsers de valeur, etc. |

Vous obtenez aussi gratuitement l'aide par commande auto-générée par
clap (`console users:purge --help`).

### `#[command]` - `Vec<String>` brut (cas simples)

Pour les commandes qui ne prennent aucun argument ou qui ne consomment
que des positionnels sous forme de liste, l'attribut sur une fn async
suffit :

```rust
use suprnova::{command, FrameworkError};

#[command(name = "cache:clear", description = "Drop every entry from the cache")]
pub async fn cache_clear(_args: Vec<String>) -> Result<(), FrameworkError> {
    suprnova::Cache::flush().await
}
```

La fonction annotée doit être
`async fn(Vec<String>) -> Result<(), FrameworkError>`. La macro
préserve la fonction d'origine, donc vous pouvez aussi l'appeler
directement depuis Rust - utile pour les tests unitaires qui ne
veulent pas faire transiter des chaînes argv à travers le dispatcher.

Les noms des deux formes prennent en charge l'espace de noms à la
Laravel : `mail:send`, `queue:work`, `db:fresh`. Les deux-points sont
purement cosmétiques - c'est une chaîne que le dispatcher fait
correspondre à `argv[1]`.

## `suprnova make:command`

Le générateur CLI dépose un stub exécutable. Le fichier généré utilise
la **forme typée** (`#[derive(Parser, Command)]` +
`impl TypedCommand`) - c'est la valeur par défaut recommandée, et elle
vous donne `--help` par commande gratuitement :

```bash
suprnova make:command cache:clear
# → src/commands/cache_clear.rs (pub struct CacheClear avec #[console(name = "cache:clear")])
# → src/commands/mod.rs reçoit `pub mod cache_clear;` en ajout (créé s'il est absent)
```

Le stub est exécutable tel quel -
`cargo run --bin console -- cache:clear` imprimera
`cache:clear: not yet implemented` et retournera `Ok(())`, pour que
vous puissiez le câbler et itérer. Remplissez les champs de la struct
pour les args typés et remplacez le corps de `TypedCommand::run`.

Normalisation du nom :

| Entrée         | Fichier           | Nom de commande |
|----------------|-------------------|----------------|
| `greet`        | `greet.rs`        | `greet`        |
| `CleanCache`   | `clean_cache.rs`  | `clean-cache`  |
| `clean-cache`  | `clean_cache.rs`  | `clean-cache`  |
| `mail:send`    | `mail_send.rs`    | `mail:send`    |

Si l'entrée contient `:`, l'espace de noms à deux-points est préservé
tel quel. Sinon, le nom de la fn Rust est en snake_case et le nom de
la commande est en kebab-case.

Assurez-vous que `pub mod commands;` est déclaré dans `src/lib.rs`
pour que la soumission à l'inventory soit atteignable au link depuis
le binaire console. Le générateur le scaffolde pour les nouveaux
projets et émet un avertissement bien visible s'il est absent ; si
vous l'avez retiré, le bloc `inventory::submit!` du nouveau fichier
compilera mais ne finira jamais dans le registre.

### Pourquoi Suprnova diverge

Le framework délibérément **ne fait pas** de commande CLI globale
`suprnova` pour les tâches à l'exécution comme `db:seed`. Un binaire
global ne peut pas charger statiquement les seeders, factories, ou fns
async `#[command]` de votre app sans soit :

- un shell vers `cargo run --bin app -- ...` (lent - compilation
  complète à chaque invocation, ce qui va à l'encontre du but), soit
- du chargement dynamique (trop de complexité pour la v1)

Donc le projet de l'utilisateur produit un binaire `console`.
Exécutez-le directement :

```bash
./target/debug/console db:seed
./target/release/console greet Alice
cargo run --bin console -- mail:send
```

Laravel résout le même problème avec `php artisan` - un script par
projet qui amorce le framework et dispatche vers les commandes
définies par l'utilisateur. PHP peut le faire dynamiquement parce que
le code du framework vit à côté de celui de l'utilisateur à
l'exécution. Le modèle compile-and-link de Rust exclut cela, donc nous
livrons le dispatcher comme une bibliothèque (`suprnova::console::*`)
et laissons chaque projet lier son propre binaire `console` d'une
ligne.

### Asymétrie avec `suprnova migrate`

Il existe trois chemins d'invocation de commande distincts dans un
projet Suprnova, et l'asymétrie est **structurelle** - n'essayez pas
de les unifier :

| Surface de commande                                   | Invocation                                              | Pourquoi                                                 |
|---------------------------------------------------|---------------------------------------------------------|-----------------------------------------------------|
| `suprnova new`, `suprnova make:*`, `suprnova serve`, `suprnova key:generate`, … | Binaire CLI global (installé via `cargo install --git`) | Générateurs et scaffolders qui ne produisent que des fichiers ; n'ont pas besoin du code utilisateur. |
| `suprnova migrate`, `suprnova migrate:status`, `suprnova schedule:run`, `suprnova schedule:work`, `suprnova schedule:list`, `suprnova workflow:work` | Le CLI global shelle vers `cargo run --quiet -- <name>` contre le binaire app/serveur | Daemons de longue durée et travail de schéma que possède le même analyseur clap `Application::run`. Le `queue:work` du binaire serveur vit ici aussi - `cargo run --bin <app> -- queue:work`. |
| `console db:seed`, `console model:prune`, `console <your-command>` | Binaire `console` par projet (`src/bin/console.rs`) | Commandes ponctuelles qui ont besoin de types utilisateur (seeders, commandes, modèles élagables) compilés dans la crate de l'utilisateur. |

La séparation est intentionnelle. Le binaire serveur a déjà besoin
d'un analyseur clap pour choisir entre `serve`, `migrate`,
`queue:work`, etc. ; les daemons qui partagent son cycle de vie vivent
là. Le binaire console existe pour tout le reste - de courte durée,
défini par l'utilisateur, riche en types. Les nouvelles commandes à
l'exécution ont leur place dans `#[command]` / `#[derive(Command)]`,
dispatchées par le binaire `console` du projet.

## Bonnes pratiques

### Gardez les handlers petits ; recourez aux services partagés via le conteneur

Un `#[command]` est le wrapper en forme de CLI ; la logique métier
devrait vivre dans une `Action`, un service, ou une méthode sur un
modèle. Le handler analyse les args, résout le service depuis le
conteneur, et transmet. Cela garde la même logique testable depuis un
test unitaire, une route HTTP, et la console.

```rust
#[command(name = "users:purge")]
pub async fn users_purge(args: Vec<String>) -> Result<(), FrameworkError> {
    let action = App::resolve::<PurgeStaleUsers>()?;
    action.execute(parse(args)?).await
}
```

`App::resolve` retourne
`Result<T, FrameworkError::ServiceUnresolved(_)>` - la saveur `?` de
`App::get` (qui retourne `Option`). Voir [Conteneur de
service](container.md) pour la surface complète.

### Utilisez des espaces de noms pour les commandes apparentées

Regroupez avec `:` : `mail:send`, `mail:retry`, `mail:queue:work`. Le
dispatcher le traite comme opaque, mais les humains repèrent `mail:*`
mieux que `send-mail`, `retry-mail`, `mail-queue-work`.

### N'imprimez pas de données structurées - retournez-les

Les handlers de console impriment sur stdout pour une sortie lisible
par un humain. Si un outil en aval a besoin de consommer la sortie,
écrivez une variante `console <name> --json` qui émet du JSON lisible
par une machine sur stdout et une ligne de statut sur stderr. Ne
rendez pas le chemin lisible par un humain responsable des deux
publics.

### Traitez les codes de sortie comme le contrat

`FrameworkError` → `ExitCode::FAILURE` est le seul chemin d'échec. Ne
faites pas de `std::process::exit(custom_code)` depuis l'intérieur
d'un handler - retournez `Err(...)` et laissez le `main` du binaire
traduire. Le tooling futur (gates CI, workers supervisés) n'a qu'à
lire le code de sortie.

## Référence

| Symbole                                    | Objectif                                       |
|-------------------------------------------|-----------------------------------------------|
| `suprnova::Command` (derive)              | Enregistre une struct qui dérive `clap::Parser` comme commande console typée. S'associe à `TypedCommand`. |
| `suprnova::TypedCommand` (trait)          | Trait avec `async fn run(self) -> Result<(), FrameworkError>` - le corps d'une commande typée. |
| `suprnova::command` (attribut)           | Enregistre une fn async prenant `Vec<String>` comme commande console à args bruts. |
| `suprnova::console::dispatch_argv(argv)`  | Construit l'arbre d'analyseur clap à partir de chaque entrée enregistrée, analyse argv, route vers le handler. Pas d'init paresseuse - pratique pour les tests et les appelants programmatiques. |
| `suprnova::console::dispatch_argv_with_init(argv, init)` | Identique à `dispatch_argv` mais exécute la closure `init` entre l'analyse d'argv par clap et le handler trouvé. L'init ne se déclenche que quand une vraie sous-commande correspond - les chemins `--help` / `--version` / erreur d'analyse la sautent. C'est ce qu'utilise le binaire `console` scaffoldé. |
| `suprnova::console::set_version(&'static str)` | Enregistre la chaîne de version exposée via `--version` et dans `--help`. À appeler une fois au début de `main`. Le premier enregistrement l'emporte. |
| `suprnova::console::find(name)`           | Recherche une commande enregistrée par nom exact.   |
| `suprnova::console::list()`               | Toutes les commandes enregistrées, triées par nom.      |
| `suprnova::CommandEntry`                  | Enregistrement d'inventory : `{ name, description, clap_builder, handler }`. Soumis par les deux macros. |
| `suprnova::CommandHandler`                | Le type pointeur de fonction du handler : `fn(&clap::ArgMatches) -> Pin<Box<dyn Future<...>>>`. |
| `FrameworkError::silent()` / `.is_silent()` | Construit / détecte une erreur que le dispatcher n'imprimera PAS sur stderr. Utilisé en interne pour supprimer les doubles impressions quand clap a déjà écrit une erreur d'analyse dans le terminal. |

## Suivant

- [Amorçage de l'application](bootstrap.md) - ce qui s'exécute à
  l'intérieur de la closure `dispatch_argv_with_init`
- [Conteneur de service](container.md) - `App::resolve` vs `App::get`,
  et comment un handler atteint les services partagés
- [Ensemencement](seeding.md) - ce que `db:seed` invoque réellement
- [Eloquent](eloquent.md) - `Prunable`, `MassPrunable`, et comment
  `model:prune` parcourt le registre
- [Planification](scheduling.md) - l'asymétrie : les daemons de
  planificateur vivent sur le binaire app, pas sur la console
