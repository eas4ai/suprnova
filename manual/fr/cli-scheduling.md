# Commandes de planification

Surface CLI pour le planificateur de tâches par minute. Les trois
sous-commandes `schedule:*` délèguent toutes au dispatch
`Application::run()` du binaire de votre application, si bien
qu'elles voient la même config, les mêmes services, observateurs, et
écouteurs qu'un handler de requête. Le modèle complet du
planificateur - le trait `Task`, l'API cron fluide,
`without_overlapping`, `run_in_background` - vit dans
[Planification](scheduling.md) ; ce chapitre est la référence
opérateur pour les commandes elles-mêmes.

## Comment les commandes s'exécutent

`suprnova schedule:run`, `suprnova schedule:work`, et `suprnova
schedule:list` sont de fins wrappers qui invoquent `cargo run --
schedule:<subcommand>` contre le projet dans le répertoire courant.
Les mêmes sous-commandes sont aussi accessibles directement sur le
binaire d'application en production :

```bash
# En développement (depuis la racine du projet, build source) :
suprnova schedule:run

# En production (binaire sur le PATH) :
/usr/local/bin/myapp schedule:run
```

Les drivers runtime (Cache, Queue, RateLimit, Mail) et votre
`bootstrap_fn` sont amorcés avant qu'aucune tâche ne s'exécute, si
bien qu'une tâche planifiée peut résoudre des services depuis le
conteneur exactement comme un contrôleur - voir [Amorçage de
l'application](bootstrap.md).

Vous devez câbler le planificateur dans le builder d'application pour
que les sous-commandes trouvent des tâches :

```rust
// cmd/main.rs (starter backend) ou src/main.rs (starter API)
Application::new()
    .config(my_app::config::register)
    .bootstrap(my_app::bootstrap::bootstrap)
    .routes(my_app::routes::register)
    .schedule(my_app::schedule::register)   // <-- le hook du planificateur
    .migrations::<my_app::migrations::Migrator>()
    .run()
    .await
```

`suprnova make:task <Name>` câble cela automatiquement ; si vous
construisez la chaîne à la main, ajoutez l'appel `.schedule(...)`
vous-même.

## schedule:run

Évalue chaque tâche enregistrée une fois et exécute celles dont
l'expression cron correspond à la minute courante. Conçue pour être
invoquée par le cron système chaque minute. Quitte avec un code non
nul si une tâche a échoué ; quitte avec zéro (avec `No tasks were
due.`) si rien n'était échu cette minute.

```bash
suprnova schedule:run
```

### Exemple de sortie

```
Running due scheduled tasks...
  ✓ cleanup:logs
  ✓ send:reminders
```

Quand une tâche retourne une erreur, sa ligne est préfixée par `✗` et
le message d'erreur est ajouté :

```
Running due scheduled tasks...
  ✓ cleanup:logs
  ✗ backup:database: connection refused
```

Quand aucune tâche n'est échue cette minute :

```
Running due scheduled tasks...
No tasks were due.
```

### Entrée crontab

Une seule entrée exécute le planificateur chaque minute. Le binaire
d'application évalue lui-même toutes les tâches échues, donc c'est la
seule ligne crontab dont un hôte de production a besoin :

```cron
* * * * * cd /path/to/your/project && /usr/local/bin/myapp schedule:run >> /var/log/myapp/schedule.log 2>&1
```

Si vous exécutez `schedule:run` depuis le cron système sur plus d'un
hôte (ou aux côtés d'un daemon `schedule:work`), les tâches marquées
`.without_overlapping()` ont besoin d'un backend Cache configuré
(`CACHE_DRIVER=redis` est le choix de qualité production) pour se
coordonner entre les processus - voir [Empêcher le
chevauchement](scheduling.md#preventing-overlapping) pour la
sémantique des verrous.

## schedule:work

Exécute le planificateur comme un daemon de longue durée. Le premier
tick est aligné sur la prochaine limite de minute, puis la boucle
évalue les tâches échues une fois par minute jusqu'à ce qu'elle
reçoive `SIGINT` (Ctrl-C) ou `SIGTERM`. À l'arrêt, toute tâche
`run_in_background` encore en vol est attendue avant de quitter pour
qu'elle ne soit pas interrompue en pleine écriture.

```bash
suprnova schedule:work
```

### Exemple de sortie

```
Starting scheduler daemon...
Press Ctrl+C to stop

==============================================
  suprnova Scheduler Daemon
==============================================
  3 task(s) registered. Press Ctrl+C to stop.
==============================================
```

Chaque tick est silencieux - seuls les échecs sont journalisés. À
l'arrêt :

```
suprnova: scheduler shutting down.
suprnova: waiting for 1 background task(s) to finish…

Scheduler daemon stopped.
```

### Cas d'usage

- **Développement.** Pas de crontab nécessaire - démarrez le daemon
  dans un terminal et observez ses ticks.
- **Docker.** Utilisez-le comme processus principal du conteneur
  quand vous voulez qu'une image joue le rôle de planificateur.
- **Systemd.** Gérez-le comme une unité de longue durée (voir [unité
  systemd](#unité-systemd) ci-dessous).

### Unité systemd

```ini
# /etc/systemd/system/myapp-scheduler.service
[Unit]
Description=MyApp Scheduler
After=network.target

[Service]
Type=simple
User=www-data
WorkingDirectory=/path/to/your/project
ExecStart=/usr/local/bin/myapp schedule:work
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable myapp-scheduler
sudo systemctl start myapp-scheduler
```

`Restart=always` fait redémarrer le daemon s'il plante ;
`RestartSec=5` amortit une boucle de plantage. Comme la limite de
panique du framework attrape les tâches qui paniquent et les
convertit en `FrameworkError`, une seule tâche défaillante ne devrait
pas faire planter le daemon - `Restart=always` est pour la rare
défaillance à l'échelle du processus (OOM, arrêt forcé par le
parent).

## schedule:list

Affiche chaque tâche enregistrée avec son expression cron, sa prochaine
exécution et sa description.

```bash
suprnova schedule:list
suprnova schedule:list --timezone=Asia/Tokyo
```

### Exemple de sortie

```
Registered scheduled tasks:
  cleanup:logs [0 3 * * *] next: 2026-05-29 03:00 UTC
  send:reminders [0 9 * * *] next: 2026-05-28 09:00 UTC
  heartbeat [* * * * *] next: 2026-05-28 12:01 UTC
  report:generate [0 6 * * *] (UTC) next: 2026-05-29 06:00 UTC
```

Les tâches qui portent un `.description(...)` enchaîné sur le builder
affichent la description après l'heure de la prochaine exécution ; celles
qui n'ont pas de description ne montrent que le cron et la prochaine
exécution.

`next:` est la première minute après maintenant à laquelle l'expression
correspond ; une expression qui ne peut jamais correspondre affiche
`next: never`. Les heures sont données en UTC, sauf si `--timezone` nomme
une autre zone IANA, et un nom de zone inconnu quitte en erreur avant que
quoi que ce soit ne soit affiché.

Une tâche qui a épinglé sa propre zone avec `.timezone(...)` voit son
expression réécrite dans la zone du listing et étiquetée avec celle-ci -
`report:generate` ci-dessus demandait `02:00 America/New_York`. Les tâches
sans zone épinglée sont affichées telles qu'elles ont été écrites et ne
portent aucune étiquette. Voir [Planification](scheduling.md) pour les
règles de fuseau horaire au complet, y compris les cas où une réécriture
est refusée et où une tâche peut occuper plusieurs lignes.

Quand rien n'est enregistré (l'appel de builder `.schedule(...)` est
absent, ou `schedule::register` est sans effet) :

```
No scheduled tasks registered.
Define tasks in src/schedule.rs and wire it with `Application::schedule(schedule::register)`.
```

## Générer une tâche

Le framework livre un générateur qui crée la tâche, la câble dans le
projet, et ajoute l'appel du planificateur à votre `main.rs` :

```bash
suprnova make:task CleanupLogs
```

Ceci :

1. Crée `src/tasks/cleanup_logs_task.rs` (un stub `Task` fonctionnel
   qui journalise sa propre durée)
2. Crée `src/tasks/mod.rs` (réexportant `CleanupLogsTask`) s'il
   n'existe pas déjà
3. Crée `src/schedule.rs` (avec une fonction
   `register(&mut Schedule)`) s'il n'existe pas déjà
4. Déclare `pub mod schedule;` et `pub mod tasks;` dans `src/lib.rs`
5. Ajoute `.schedule(<crate>::schedule::register)` à la chaîne
   `Application` dans `cmd/main.rs` (ou `src/main.rs` pour le starter
   API)

Les étapes 2 à 5 sont idempotentes, donc réexécuter `make:task`
répare le câblage qui a été retiré à la main. Voir [Générateurs de
code](cli-generators.md) pour la famille `make:*` plus large.

Après la génération, enregistrez la tâche dans `src/schedule.rs` :

```rust
use suprnova::Schedule;
use crate::tasks::CleanupLogsTask;

pub fn register(schedule: &mut Schedule) {
    schedule.add(
        schedule.task(CleanupLogsTask::new())
            .daily()
            .at("03:00")
            .name("cleanup:logs")
            .description("Removes logs older than 30 days")
    );
}
```

L'API fluide du builder (`.daily()`, `.cron(...)`,
`.without_overlapping()`, `.run_in_background()`, les modificateurs
spécifiques au jour) est entièrement couverte dans
[Planification](scheduling.md).

## Codes de sortie

| Commande | Quitte avec zéro | Quitte avec un code non nul |
|---|---|---|
| `schedule:run` | chaque tâche échue a retourné `Ok(())`, ou aucune tâche n'était échue | au moins une tâche a retourné `Err(_)` ou a paniqué |
| `schedule:work` | arrêt propre via `SIGINT` / `SIGTERM` (le wrapper traite le code de sortie 130 comme un Ctrl-C propre) | échec de bootstrap, ou le processus daemon a été interrompu |
| `schedule:list` | le listing a réussi (y compris le message « aucune tâche enregistrée ») | l'application a échoué à démarrer |

Les échecs de tâche en arrière-plan à l'intérieur de `schedule:work`
sont journalisés sur stderr mais ne font pas quitter le daemon - la
limite `catch_unwind` du `JoinSet` les remonte en tant que
`FrameworkError` et la boucle de tick continue.

### Pourquoi Suprnova diverge

Le `schedule:run` de Laravel est le seul point d'entrée de première
classe ; la forme daemon (`schedule:work`) est un backport pour les
hôtes sans crontab. PHP n'a pas de processus de longue durée, donc
chaque minute est un runtime neuf qui doit réamorcer le framework, le
conteneur, et chaque liaison de service.

Dans Suprnova, le daemon est de premier ordre. `schedule:work`
s'exécute à l'intérieur du même runtime Tokio qui sert HTTP, donc :

- **Les tâches en arrière-plan se combinent avec la boucle de tick.**
  Une tâche `.run_in_background()` est spawnée dans un `JoinSet` ; la
  boucle sonde celles qui sont terminées avant le tick suivant et
  vide le reste à l'arrêt. Laravel spawne un processus enfant par
  tâche en arrière-plan.
- **L'arrêt gracieux vide le travail en vol.** Ctrl-C / SIGTERM
  laisse les tâches en ligne terminer leur appel courant et attend
  chaque spawn en arrière-plan avant de sortir. Laravel s'appuie sur
  l'OS pour tuer l'enfant cron.
- **Le coût de l'amorçage n'est payé qu'une fois.** Le conteneur, les
  drivers, et votre `bootstrap_fn` s'amorcent au démarrage du daemon,
  pas à chaque tick. `schedule:run` paie toujours le coût d'amorçage
  par invocation (c'est une sous-commande ponctuelle), mais le chemin
  du daemon est là où le modèle runtime porte ses fruits.

`schedule:run` fonctionne toujours (et est le bon choix quand le cron
système est déjà la source de vérité de l'opérateur). Choisissez
celui qui correspond à la forme de votre déploiement - les deux
partagent les mêmes définitions de tâche.

## Suivant

- [Planification](scheduling.md) - le trait `Task`, l'API cron
  fluide, `without_overlapping`, `run_in_background`, et le
  dédoublonnage à la même minute
- [Générateurs de code](cli-generators.md) - la famille complète
  `make:*`, y compris `make:task`
- [Console](console.md) - les tâches opérateur ponctuelles annotées
  `#[command]` (pas planifiées)
- [Files d'attente](queues.md) - pour le travail qui devrait être
  récupéré par un worker plutôt que cadencé par une horloge
- [Amorçage de l'application](bootstrap.md) - comment `.schedule(...)`
  se branche dans le builder, et ce que les tâches peuvent résoudre
  depuis le conteneur
