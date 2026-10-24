# Développement

La boucle quotidienne de Suprnova se résume à une seule commande : `suprnova serve`.  Elle exécute
le backend Rust, le frontend Vite et un régénérateur de types TypeScript
dans un seul processus, chacun surveillant les bons fichiers.  Ce chapitre couvre
le serveur de développement, le fonctionnement des éléments du rechargement à chaud, et les
commandes que vous utiliserez quotidiennement.  Pour la configuration initiale, voir
[Installation](installation.md) ; pour la visite des répertoires, voir
[Structure des répertoires](structure.md).

## Le serveur de développement

Depuis la racine d'un projet scaffolé :

```bash
suprnova serve
```

L'interface CLI affiche deux URL et puis un flux continu de sorties
préfixées provenant de chaque processus enfant :

```
Backend  http://127.0.0.1:8765
Frontend http://127.0.0.1:5765

[backend]  Compiling links v0.1.0
[backend]  Finished `dev` profile [unoptimized + debuginfo] target(s) in 4.21s
[backend]  Running `target/debug/links`
[frontend] VITE v6.0.1  ready in 312 ms
[frontend]   ➜  Local:   http://localhost:5765/
[types]    Watching for Rust file changes to regenerate types
```

Vous accédez à l'URL du backend (`127.0.0.1:8765`).  Vite sert votre JS/CSS
via l'intégration de développement d'Inertia - vous ne visitez pas `:5765` directement.
Appuyez sur `Ctrl+C` une fois et l'interface CLI arrête les deux processus proprement.

### Flags

| Flag | Par défaut | Ce qu'il fait |
|---|---|---|
| `-p`, `--port <N>` | `8765` | Port du backend |
| `--frontend-port <N>` | `5765` | Port Vite |
| `--backend-only` | désactivé | Ignorer le processus Vite (travail API uniquement) |
| `--frontend-only` | désactivé | Ignorer le processus backend (travail de composant contre un backend en cours d'exécution ailleurs) |
| `--skip-types` | désactivé | Ignorer le générateur de types TypeScript et sa surveillance |

Les mêmes ports peuvent être définis dans `.env` via `SERVER_PORT` et `VITE_PORT`.
Un flag sur la ligne de commande prime sur `.env`.

### Ce qu'il pré-vérifie

Avant de démarrer quoi que ce soit, `suprnova serve` :

1. **Vérifie que vous êtes dans un projet.**  Abandonne avec une erreur claire s'il n'y a
   pas de `Cargo.toml` (ou pas de `frontend/` lors de l'exécution du frontend).
2. **Génère les types TypeScript une fois.**  Scanne `src/` pour
   `#[derive(InertiaProps)]` et écrit
   `frontend/src/types/inertia-props.ts`.  Ignoré par `--skip-types` ou
   `--frontend-only`.
3. **Installe `cargo-watch` s'il manque.**  La première exécution sur une nouvelle machine
   exécute `cargo install cargo-watch` pour vous, puis continue.
4. **Exécute `npm install` si `frontend/node_modules` manque.**  Aucune
   étape d'installation manuelle sur un clone frais.

## Rechargement à chaud

Trois surveilleurs s'exécutent concurremment dans `suprnova serve` :

- **`cargo watch -x 'run --bin <pkg>'`** conduit le backend.  Toute modification `.rs`
  dans le projet déclenche une recompilation et un
  redémarrage en cours de processus.  Les erreurs de compilation s'affichent sur le flux `[backend]` et le
  binaire précédent reste actif jusqu'au prochain build réussi.
- **Vite** conduit le frontend.  Les modifications de composants, styles et ressources
  se remplacent par module à chaud dans l'onglet du navigateur ouvert sans rechargement complet.
- **Surveilleur de types basé sur `notify`** réexécute le scanner InertiaProps
  chaque fois qu'un fichier `.rs` change.  Il débounce à 500 ms pour qu'une salve
  de sauvegardes régénère `inertia-props.ts` une fois.  La sortie s'affiche sous le
  préfixe `[types]`.

Ce troisième élément est celui auquel vous n'avez pas à penser : renommez un champ
sur une structure `#[derive(InertiaProps)]` et l'interface TypeScript correspondante
suit lors de la sauvegarde suivante.  La page Svelte/React/Vue récupère
immédiatement le nouveau type.  Aucune invocation `suprnova generate-types`
nécessaire pendant le développement normal.

### Pourquoi Suprnova diverge

La plupart des stacks web Rust rendent le rechargement à chaud votre problème - choisissez votre propre
surveillance de fichiers, écrivez votre propre wrapper de redémarrage, exécutez Vite dans un
terminal séparé.  La plupart des stacks Laravel rendent les types TypeScript votre problème -
déclarez-les en deux endroits (PHP et TS) et gardez-les synchronisés.
`suprnova serve` exécute les deux surveilleurs, plus le générateur de types qui
garde vos types frontend honnêtes, en un seul processus supervisé.  Le
runtime Tokio rend « faire plusieurs choses à la fois » assez bon marché pour qu'une boucle
de développement puisse l'utiliser librement.

## Commandes quotidiennes

Quelques-unes que vous exécuterez toutes les heures :

```bash
suprnova serve                    # démarre le dev (backend + Vite + surveilleur de types)
suprnova make:controller orders   # scaffolde un contrôleur
suprnova make:migration add_idx   # scaffolde une migration
suprnova db:sync                  # exécute les migrations, régénère les entités SeaORM
suprnova migrate:status           # affiche ce qui est appliqué
suprnova migrate:fresh            # supprime les tables + relance de zéro
suprnova key:generate --show      # effectue la rotation d'APP_KEY
cargo run --bin console <cmd>     # n'importe quel handler de console annoté `#[command]`
cargo test                        # exécute la suite de tests
```

`db:sync` est le raccourci de développement pour « migration + régénération d'entité en une
étape ».  En production, vous utilisez simplement `suprnova migrate` car vous
ne voulez pas que la régénération se produise sur une machine de release.  La surface complète du générateur
est dans [Générateurs de code](cli-generators.md) et les
verbes de migration sont dans [Migrations](migrations.md).

## Débogage

### Journalisation

Suprnova utilise `tracing` bout en bout.  Filtrez ce qui s'affiche avec
`LOG_LEVEL` (la même syntaxe que `EnvFilter` de `tracing-subscriber`) :

```bash
# Sortie verbeuse du framework
LOG_LEVEL=debug suprnova serve

# hyper silencieux mais votre crate verbeuse
LOG_LEVEL=info,my_app=debug,hyper=warn suprnova serve
```

Le format de sortie est contrôlé par `LOG_FORMAT` (`pretty` pour lisible par l'homme,
`json` pour parsable par machine).  La valeur par défaut de développement est `pretty`.  Voir
[Observabilité](observability.md) pour la surface complète de la journalisation.

### Requêtes SQL

Activez la journalisation par requête avec une variable d'environnement :

```env
DB_LOGGING=true
```

Cela achemine chaque requête SeaORM via `tracing` au niveau `info` pour que vous puissiez
voir exactement ce qui s'exécute.  Laissez-le désactivé en production à moins que vous ne
traciez une requête lente spécifique - le volume devient bruyant rapidement.

### Traces d'exécution

Rust standard :

```bash
RUST_BACKTRACE=1 suprnova serve
```

Une panique dans un handler est capturée et transformée en une réponse 500
structurée ; la trace d'exécution atterrit dans vos journaux sans arrêter le serveur.
Voir [Modèle d'erreur](error-model.md) pour savoir comment ce contrat fonctionne.

## Tests en boucle

```bash
cargo test                        # tout l'espace de travail
cargo test -p my_app              # seulement la crate de votre app
cargo test some_test_name         # filtre par nom
cargo test -- --nocapture         # affiche la sortie println!/tracing
```

L'exécution des tests est du Cargo simple.  Les assistants côté framework
(`#[suprnova_test]`, `TestDatabase`, `expect!`, fakes pour Mail/Queue/
Storage/etc.) sont documentés dans [Tests](testing.md) et
[Tests de base de données](database-testing.md).  Ils s'exécutent sous le même
`cargo test` que vous connaissez déjà.

## Travail avec le worker SSR

Si votre application utilise le rendu côté serveur Inertia, vous voudrez le worker SSR
à côté de `suprnova serve` pendant le développement :

```bash
# Terminal 1
suprnova serve

# Terminal 2
suprnova ssr:start
```

`ssr:start` exécute le worker SSR fourni sous Node, Bun ou Deno
(`--runtime`).  `ssr:check` vérifie qu'un worker en cours d'exécution est accessible.
Les deux sont documentés dans le chapitre frontend - voir
[Présentation du frontend](frontend.md).

## Quand quelque chose semble mal

Une courte liste de triage pour les ratés les plus courants de la boucle de développement :

- **Port déjà utilisé.**  Un autre `suprnova serve` est encore actif, ou un
  backend précédent s'est bloqué.  `lsof -i :8765` pour le trouver, ou passez simplement
  `--port 8001`.
- **`cargo-watch` continue de recompiler.**  Un éditeur réécrit les fichiers
  à la sauvegarde (formateurs, linters avec correction automatique).  Désactivez le formatage à la sauvegarde
  pour le projet, ou délimitez votre surveilleur avec des modèles
  `CARGO_WATCH_IGNORE`.
- **Les types TypeScript ne se mettent pas à jour.**  Soit `--skip-types` a été passé,
  soit le surveilleur a trébuché sur une erreur d'analyse `.rs`.  Regardez les
  lignes `[types]` - elle affiche un avertissement et continue plutôt que
  d'échouer tout le serve.
- **Erreurs Vite mais le backend est bien.**  Exécutez `npm install` dans
  `frontend/` une fois (l'interface CLI fait ceci au premier serve, mais si vous
  supprimez `node_modules`, elle ne le refera pas tant que ce répertoire est
  manquant à nouveau au démarrage frais).

N'importe quoi d'autre, le chapitre [Gestion des erreurs](errors.md) couvre des modèles de triage plus profonds.

## Suivant

- [Installation](installation.md) - configuration initiale de l'interface CLI et d'un
  projet
- [Démarrage rapide](quickstart.md) - construire une petite application bout en bout
- [Structure des répertoires](structure.md) - ce que chaque répertoire contient
- [Générateurs de code](cli-generators.md) - chaque commande `make:*`
- [Tests](testing.md) - `#[suprnova_test]`, fakes et la base de données
  de test
