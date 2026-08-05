# Planification de tâches

Les tâches planifiées sont des fonctions async que le framework exécute
selon une expression cron - chaque minute, chaque heure, chaque jour,
chaque semaine, ou n'importe quelle expression cron personnalisée à 5
champs. Les tâches vivent à l'intérieur du binaire de votre application ;
`schedule:run` évalue les tâches échues une fois (appelez-la depuis le
cron système) et `schedule:work` exécute le même évaluateur comme un
daemon de longue durée.

## Générer des tâches

La façon la plus rapide de créer une nouvelle tâche planifiée est
d'utiliser le CLI suprnova :

```bash
suprnova make:task CleanupLogs
```

Cette commande va :
1. Créer `src/tasks/cleanup_logs_task.rs` avec un stub de tâche fonctionnel
2. Créer `src/tasks/mod.rs` s'il n'existe pas, en réexportant la tâche
3. Créer `src/schedule.rs` pour enregistrer les tâches, s'il n'existe pas
4. Déclarer `pub mod schedule;` et `pub mod tasks;` dans `src/lib.rs`
5. Câbler `.schedule(<crate>::schedule::register)` dans le builder de votre
   application dans `cmd/main.rs` (ou `src/main.rs` pour le starter API)

Les étapes 2–5 sont idempotentes, si bien que réexécuter `make:task`
répare le câblage qui a été retiré à la main. Le planificateur s'exécute à
l'intérieur du binaire de votre application - il n'y a pas d'exécutable de
planificateur séparé à construire ou déployer.

```bash Examples
# Crée CleanupLogsTask dans src/tasks/cleanup_logs_task.rs
suprnova make:task CleanupLogs

# Crée SendRemindersTask dans src/tasks/send_reminders_task.rs
suprnova make:task SendReminders

# Vous pouvez aussi inclure le suffixe "Task" (même résultat)
suprnova make:task BackupDatabaseTask
```

```rust Generated File
//! CleanupLogsTask scheduled task
//!
//! Created with `suprnova make:task cleanup_logs_task`.

use std::time::Instant;

use async_trait::async_trait;
use suprnova::{Task, TaskResult};

/// CleanupLogsTask - A scheduled task.
///
/// Register the task in `src/schedule.rs` with the fluent API; the skeleton
/// below times its own run and prints a structured log line on each
/// invocation so it works end-to-end the first time you wire it up.
pub struct CleanupLogsTask;

impl CleanupLogsTask {
    /// Create a new instance of this task.
    pub fn new() -> Self {
        Self
    }
}

impl Default for CleanupLogsTask {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Task for CleanupLogsTask {
    async fn handle(&self) -> TaskResult {
        let started_at = Instant::now();
        println!("[CleanupLogsTask] task started");

        // Replace this with the real job. The skeleton ships as a
        // no-op success so the task can be scheduled and observed
        // before the implementation is filled in.

        println!(
            "[CleanupLogsTask] task finished in {} ms",
            started_at.elapsed().as_millis(),
        );
        Ok(())
    }
}
```

## Définir des planifications

suprnova prend en charge deux approches pour définir des tâches planifiées
:

### 1. Tâches basées sur un trait (recommandé)

Pour les tâches complexes qui ont besoin de dépendances ou de logique
réutilisable, implémentez le trait `Task` et configurez la planification
pendant l'enregistrement :

```rust
// src/tasks/cleanup_logs_task.rs
use async_trait::async_trait;
use chrono::{Duration, Utc};
use suprnova::{Task, TaskResult};
use crate::models::Log;

pub struct CleanupLogsTask;

impl CleanupLogsTask {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Task for CleanupLogsTask {
    async fn handle(&self) -> TaskResult {
        // Eloquent fonctionne exactement comme à l'intérieur d'un contrôleur ; les
        // tâches voient les mêmes liaisons du conteneur (`DB::connection()`,
        // `App::get::<T>()`) qu'un handler de requête - voir Amorçage de
        // l'application ci-dessous.
        let cutoff = Utc::now() - Duration::days(30);
        Log::query()
            .filter_op("created_at", "<", cutoff)
            .delete_all()
            .await?;

        println!("Old logs cleaned up successfully");
        Ok(())
    }
}
```

Puis enregistrez-la avec l'API de planification fluide dans
`src/schedule.rs` :

```rust
// src/schedule.rs
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

### 2. Tâches basées sur une closure

Pour des tâches rapides, en ligne, sans fichiers séparés :

```rust
// src/schedule.rs
use suprnova::Schedule;

pub fn register(schedule: &mut Schedule) {
    // Tâche simple par closure
    schedule.add(
        schedule.call(|| async {
            println!("Ping! Running every minute");
            Ok(())
        })
        .every_minute()
        .name("heartbeat")
    );

    // Tâche par closure configurée
    schedule.add(
        schedule.call(|| async {
            // Votre logique de tâche
            Ok(())
        })
        .daily()
        .at("09:00")
        .name("morning-report")
        .description("Sends daily morning report")
    );
}
```

## Enregistrer des tâches

Enregistrez vos tâches dans `src/schedule.rs` :

```rust
// src/schedule.rs
use suprnova::Schedule;
use crate::tasks;

pub fn register(schedule: &mut Schedule) {
    // Tâches basées sur un trait avec configuration de planification fluide
    schedule.add(
        schedule.task(tasks::CleanupLogsTask::new())
            .daily()
            .at("03:00")
            .name("cleanup:logs")
            .description("Removes logs older than 30 days")
    );

    schedule.add(
        schedule.task(tasks::SendRemindersTask::new())
            .daily()
            .at("09:00")
            .name("send:reminders")
            .description("Sends daily reminder emails")
    );

    schedule.add(
        schedule.task(tasks::BackupDatabaseTask::new())
            .weekly()
            .at("00:00")
            .name("backup:database")
            .description("Weekly database backup")
            .without_overlapping()
    );

    // Tâches basées sur une closure
    schedule.add(
        schedule.call(|| async {
            println!("Quick task!");
            Ok(())
        })
        .hourly()
        .name("quick-task")
    );
}
```

## Options de fréquence de planification

suprnova fournit une API fluide pour définir quand les tâches doivent
s'exécuter :

### Intervalles courants

| Méthode | Description |
|--------|-------------|
| `.every_minute()` | S'exécute chaque minute |
| `.every_two_minutes()` | S'exécute toutes les 2 minutes |
| `.every_five_minutes()` | S'exécute toutes les 5 minutes |
| `.every_ten_minutes()` | S'exécute toutes les 10 minutes |
| `.every_fifteen_minutes()` | S'exécute toutes les 15 minutes |
| `.every_thirty_minutes()` | S'exécute toutes les 30 minutes |
| `.hourly()` | S'exécute chaque heure à la minute 0 |
| `.hourly_at(30)` | S'exécute chaque heure à la minute 30 |
| `.every_two_hours()` / `.every_three_hours()` / `.every_four_hours()` / `.every_six_hours()` | S'exécute à l'heure pile toutes les N heures |
| `.daily()` | S'exécute chaque jour à minuit |
| `.daily_at("03:00")` | S'exécute chaque jour à 3h00 |
| `.twice_daily(1, 13)` | S'exécute deux fois par jour (par ex. 1h00 et 13h00) |
| `.weekly()` | S'exécute chaque semaine le dimanche à minuit |
| `.monthly()` | S'exécute chaque mois le 1er à minuit |
| `.monthly_on(15)` | S'exécute chaque mois à un jour spécifique |
| `.quarterly()` | S'exécute le 1er de janvier/avril/juillet/octobre à minuit |
| `.yearly()` | S'exécute le 1er janvier à minuit |

### Planifications pour des jours spécifiques

```rust
use suprnova::DayOfWeek;

// S'exécute à des jours spécifiques
.weekly_on(DayOfWeek::Monday)
.weekly_on(DayOfWeek::Friday)

// Méthodes raccourcies par jour
.sundays()
.mondays()
.tuesdays()
.wednesdays()
.thursdays()
.fridays()
.saturdays()

// Plusieurs jours
.days(&[DayOfWeek::Monday, DayOfWeek::Wednesday, DayOfWeek::Friday])

// Jours de semaine/Week-ends
.weekdays()  // Lundi-vendredi
.weekends()  // Samedi-dimanche
```

### Modificateurs d'heure

Chaînez `.at()` avec n'importe quelle planification pour définir une heure
spécifique :

```rust
.daily().at("14:30")           // Chaque jour à 14h30
.weekly().at("09:00")          // Chaque semaine à 9h00
.mondays().at("08:00")         // Chaque lundi à 8h00
.monthly().at("00:00")         // Premier du mois à minuit
```

### Expressions cron personnalisées

Pour un contrôle total, utilisez la syntaxe cron :

```rust
// Format cron standard : minute heure jour-du-mois mois jour-de-la-semaine
.cron("0 */2 * * *")    // Toutes les 2 heures
.cron("30 4 * * 1-5")   // 4h30 les jours de semaine
.cron("0 0 1,15 * *")   // Le 1er et le 15 de chaque mois
```

`.cron(...)` **panique** si l'expression est malformée (mauvais nombre de
champs, step/range/list non analysable). Utilisez `.try_cron(expr)` quand
l'expression est fournie à l'exécution (configuration, saisie utilisateur)
et que vous préférez propager l'erreur d'analyse :

```rust
schedule.add(
    schedule.task(MyTask::new())
        .try_cron(env_expr)?   // retourne Err(String) sur une expression invalide
        .name("from-config")
);
```

La même paire `panic` / `try_*` existe sur chaque méthode de builder à
plage numérique : `try_hourly_at`, `try_daily_at`, `try_twice_daily`,
`try_monthly_on`. Les variantes infaillibles paniquent sur des valeurs
numériques hors limites (par ex. `daily_at("25:00")` ou `monthly_on(40)`)
; leurs homologues faillibles retournent `Err(String)`.

## Configuration des tâches

### Empêcher le chevauchement

Sautez un tick quand une exécution précédente de la même tâche est encore
en vol :

```rust
schedule.add(
    schedule.task(LongRunningTask::new())
        .daily()
        .name("long-task")
        .without_overlapping()
);
```

**Comment le verrou fonctionne.** Quand le flag est activé, suprnova
essaie d'acquérir un mutex distribué via le backend [`Cache`](cache.md)
configuré (`schedule:lock:<task-name>`). Une acquisition réussie exécute
la tâche et libère le verrou ; une acquisition en contention est rapportée
comme un saut réussi - `Ok(())`, avec le compteur de sauts de la tâche
incrémenté afin que les surfaces d'observabilité puissent le voir sans
fausser le code de sortie de `schedule:run`.

**Le Cache est requis pour la protection inter-processus.** Si vous
exécutez plusieurs processus qui planifient la même tâche (par ex.,
plusieurs machines invoquant `suprnova schedule:run` depuis le cron
système, ou des daemons `schedule:work` derrière un load balancer), le
backend Cache est ce qui les coordonne. **Sans Cache configuré,
`without_overlapping()` se dégrade silencieusement vers un `AtomicBool`
par processus** - deux processus séparés ne verront pas les verrous l'un
de l'autre. Le framework émet un `WARN` unique (`suprnova::schedule`) la
première fois que ce repli se déclenche, afin que les opérateurs
remarquent la garantie plus faible :

> `without_overlapping() falling back to in-process AtomicBool protection - Cache is not bootstrapped. Multi-process deployments will NOT see each other's locks. Configure Cache (CACHE_DRIVER=memory|redis) before relying on cross-process overlap protection.`

**TTL de verrou personnalisé.** Le TTL du verrou vaut 30 minutes par
défaut - assez long pour que la plupart des tâches se terminent, assez
court pour qu'une tâche plantée qui détient le verrou débloque le tick
suivant sans intervention de l'opérateur. Surchargez par tâche avec
`.without_overlapping_for(Duration)`. `Duration::ZERO` n'est pas défini de
façon uniforme selon les backends de cache (Redis renvoie une erreur, la
mémoire expire instantanément, Memcached le traite comme « n'expire jamais
»), donc le builder le force au défaut de 30 minutes avec un `WARN` unique
afin que l'opérateur puisse corriger le site d'appel.

```rust
use std::time::Duration;

schedule.add(
    schedule.task(SlowBackupTask::new())
        .daily()
        .name("backup:full")
        // Cette tâche s'exécute légitimement plus longtemps que le défaut de
        // 30 minutes ; donnez au verrou un TTL de 2 heures pour qu'une
        // exécution lente ne soit pas préemptée par le tick suivant.
        .without_overlapping_for(Duration::from_secs(2 * 3600))
);
```

### S'exécuter sur un seul serveur

Exécutez une tâche exactement une fois par tick échu, quel que soit le
nombre de répliques qui font tourner le planificateur :

```rust
schedule.add(
    schedule.task(NightlyBillingTask::new())
        .daily()
        .at("02:00")
        .name("billing:nightly")
        .on_one_server()
);
```

**Ce qui se passe de travers sans cela.** Chaque réplique qui fait tourner
`schedule:work` évalue la planification indépendamment, et rien n'empêche
toutes ces répliques de décider que le même tick leur appartient. Trois
répliques ont été mesurées produisant trois exécutions de la même tâche,
chaque minute, sans aucune variance. Pour une tâche de facturation
nocturne, cela signifie que chaque client est facturé trois fois.

**Pourquoi `without_overlapping()` ne couvre pas ce cas.** Les deux se
ressemblent mais résolvent des problèmes différents :

| | Clé de verrou | Détenu pendant | Empêche |
|---|---|---|---|
| `without_overlapping()` | la tâche | la durée de la tâche | qu'une exécution lente chevauche son propre tick suivant |
| `on_one_server()` | la tâche **+ le tick** | la fenêtre du tick | qu'une seconde réplique exécute le même tick |

La distinction qui compte est le moment où le verrou est libéré.
`without_overlapping()` se libère dès que le handler retourne - pour une
tâche rapide, avant même qu'une seconde réplique n'ait regardé, si bien
que les N répliques s'exécutent quand même toutes. `on_one_server()`
conserve délibérément son verrou au-delà du handler et le laisse expirer
sur son TTL, car une réplique arrivant plus tard dans le même tick doit le
trouver déjà pris.

Ils se combinent. Une tâche de longue durée qui doit aussi être
mono-serveur prend les deux.

**Nécessite un cache partagé.** L'élection est un verrou
[`Cache`](cache.md), donc « un seul serveur » signifie « un seul processus
parmi ceux qui partagent un backend de cache ». Sous
`CACHE_DRIVER=memory`, le verrou vit dans le tas d'un seul processus,
chaque réplique gagne sa propre élection, et la garantie est
silencieusement absente.

En production, c'est un **échec d'amorçage**, pas un avertissement :

> `refusing to boot in production: 1 task(s) request single-server execution (billing:nightly) but CACHE_DRIVER is memory or unset, so the election lock lives in this process's heap. Every replica would win its own election and run the task, which is what on_one_server() exists to prevent. Set CACHE_DRIVER=redis with REDIS_URL, or set SCHEDULE_ALLOW_MEMORY_LOCK_IN_PRODUCTION=true to acknowledge per-process locking - which is only accurate if you run exactly one scheduler.`

Définissez `SCHEDULE_ALLOW_MEMORY_LOCK_IN_PRODUCTION=true` si votre
déploiement fait réellement tourner un seul planificateur. En dehors de la
production, le driver mémoire reste utilisable et le framework avertit une
seule fois à la place.

**TTL de verrou personnalisé.** Vaut 60 secondes par défaut - un tick
aligné sur la minute. Les deux bords comptent : trop court, et une
réplique dont le tick arrive quelques secondes en retard trouve le verrou
disparu et exécute la tâche à nouveau ; trop long, et le verrou survit à
son tick, si bien que l'exécution échue *suivante* le trouve détenu et est
entièrement sautée. Utilisez `.on_one_server_for(Duration)` pour des
planifications à grain plus grossier.

```rust
use std::time::Duration;

schedule.add(
    schedule.task(HourlyRollupTask::new())
        .hourly()
        .name("rollup:hourly")
        // Une tâche horaire n'a besoin que le verrou survive à la fenêtre
        // pendant laquelle des répliques pourraient encore considérer ce
        // tick comme échu.
        .on_one_server_for(Duration::from_secs(300))
);
```

**Si le cache est injoignable**, le tick est sauté plutôt qu'exécuté.
Perdre la coordination est le pire moment possible pour laisser passer
toutes les répliques : un tick sauté est récupérable au tick suivant, des
effets de bord dupliqués ne le sont généralement pas.

### Pourquoi Suprnova diverge

Le `onOneServer()` de Laravel est le même mécanisme opt-in, et Suprnova le
conserve : les tâches par serveur - rotation de logs, réchauffement d'un
cache local - sont légitimes et restent exprimables.

Là où cela diverge, c'est le mode de défaillance. Laravel exécute sans
problème `onOneServer()` contre un driver de cache incapable de
coordonner. Suprnova refuse plutôt de démarrer en production, selon le
même raisonnement que le limiteur de débit en mémoire : un contrôle qui
fait silencieusement bien moins qu'il ne prétend est pire qu'un contrôle
visiblement absent.

### S'exécuter en arrière-plan

Détachez des tâches du chemin critique par tick afin qu'elles ne bloquent
pas le démarrage d'autres tâches échues :

```rust
schedule.add(
    schedule.task(BackgroundTask::new())
        .hourly()
        .name("background-task")
        .run_in_background()
);
```

**Isolation des paniques.** Les tâches en arrière-plan s'exécutent à
l'intérieur d'un `tokio::task::JoinSet` avec `catch_unwind`, si bien
qu'une tâche qui panique remonte comme une `FrameworkError` portant le
nom de la tâche plutôt que de démolir le planificateur. Le daemon
`schedule:work` vide le JoinSet à l'arrêt (Ctrl-C / SIGTERM) afin que les
tâches en arrière-plan en vol se terminent avant la sortie.

**Combinez avec `without_overlapping`.** Les deux flags se combinent - une
tâche en arrière-plan avec `without_overlapping()` sera spawnée dans le
JoinSet et acquerra le verrou de chevauchement depuis l'intérieur du
future spawné, si bien que la sémantique de verrou décrite plus haut
s'applique toujours.

### Dédoublonnage à la même minute

La résolution de cron est au niveau de la minute, et suprnova l'impose :
si la même tâche se voit demander de s'exécuter deux fois au sein de la
même minute d'horloge à l'intérieur d'un seul processus, le second appel
est un saut sans effet - `Ok(())`, avec le compteur de sauts de la tâche
incrémenté. Cela referme une classe de bug où une boucle de daemon ou une
invocation serrée de `schedule:run` pourrait exécuter une tâche
`.every_minute()` plusieurs fois dans la même minute.

Cette barrière in-process est **toujours active**, indépendamment de
`without_overlapping`. Elle ne s'étend PAS entre processus (chaque
processus a son propre état par tâche). Si vous avez besoin d'une
coordination inter-processus à la même minute, superposez
`without_overlapping` + un backend Cache configuré - ensemble, ils
couvrent les deux directions.

## Exécuter le planificateur

suprnova fournit des commandes CLI pour exécuter les tâches planifiées :

### Exécuter une fois

Exécute une fois toutes les tâches échues (typiquement appelée par cron
chaque minute) :

```bash
suprnova schedule:run
```

### Mode daemon

Tourne en continu, en vérifiant les tâches échues chaque minute :

```bash
suprnova schedule:work
```

C'est idéal pour le développement ou quand vous utilisez un
gestionnaire de processus comme systemd.

### Lister les tâches

Affiche toutes les tâches planifiées enregistrées :

```bash
suprnova schedule:list
```

Sortie :
```
Registered scheduled tasks:
  cleanup:logs [0 3 * * *] - Removes logs older than 30 days
  send:reminders [0 9 * * *] - Sends daily reminder emails
  backup:database [0 0 * * 0] - Weekly database backup
```

## Configuration en production

### Utiliser cron

Ajoutez une seule entrée cron pour exécuter le planificateur chaque minute
:

```bash
* * * * * cd /path/to/your/project && suprnova schedule:run >> /dev/null 2>&1
```

**Coordination inter-processus.** Si vous exécutez `schedule:run` depuis
le cron système sur plus d'un hôte (ou à côté d'un daemon
`schedule:work`), les tâches avec `.without_overlapping()` ont besoin d'un
backend **Cache** configuré (`CACHE_DRIVER=redis` recommandé en
production) pour coordonner entre les processus. Sans cela, le flag de
chevauchement se dégrade en une protection par processus et la même tâche
peut s'exécuter sur plusieurs hôtes à la même minute. Voir [Empêcher le
chevauchement](#empêcher-le-chevauchement) plus haut pour la sémantique
complète du verrou.

### Utiliser Systemd

Créez un service systemd pour le daemon du planificateur :

```ini
# /etc/systemd/system/myapp-scheduler.service
[Unit]
Description=MyApp Scheduler
After=network.target

[Service]
Type=simple
User=www-data
WorkingDirectory=/path/to/your/project
ExecStart=/path/to/suprnova schedule:work
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl enable myapp-scheduler
sudo systemctl start myapp-scheduler
```

## Accéder au contexte de l'application

Les tâches planifiées ont un accès complet au contexte de l'application,
tout comme les contrôleurs :

```rust
use async_trait::async_trait;
use suprnova::{App, Task, TaskResult};
use crate::actions::SendEmailAction;
use crate::models::User;

pub struct SendRemindersTask;

#[async_trait]
impl Task for SendRemindersTask {
    async fn handle(&self) -> TaskResult {
        // Eloquent : `.get()` retourne une `Collection<User>` que vous pouvez itérer.
        let users = User::query()
            .filter("reminder_enabled", true)
            .get()
            .await?;

        // Tout ce qui est lié dans `bootstrap.rs` est également accessible ici.
        let send_email = App::get::<SendEmailAction>()
            .expect("SendEmailAction bound in bootstrap()");

        for user in users.iter() {
            send_email.execute(&user.email, "Daily Reminder").await?;
        }

        Ok(())
    }
}
```

## Disposition des fichiers

La structure de fichiers recommandée pour les tâches planifiées :

```
src/
├── tasks/
│   ├── mod.rs              # Réexporte toutes les tâches (mis à jour automatiquement par make:task)
│   ├── cleanup_logs_task.rs
│   ├── send_reminders_task.rs
│   └── backup_database_task.rs
├── schedule.rs             # Enregistre les tâches (exécuté par les commandes schedule:*)
├── bootstrap.rs
├── routes.rs
└── lib.rs                  # Déclare `pub mod schedule;` + `pub mod tasks;`
cmd/
└── main.rs                 # Appelle `.schedule(<crate>::schedule::register)`
```

**src/tasks/mod.rs:**
```rust
pub mod cleanup_logs_task;
pub mod send_reminders_task;
pub mod backup_database_task;

pub use cleanup_logs_task::CleanupLogsTask;
pub use send_reminders_task::SendRemindersTask;
pub use backup_database_task::BackupDatabaseTask;
```

## Câbler le planificateur dans votre application

`make:task` câble automatiquement `.schedule(<crate>::schedule::register)`
dans votre builder `Application`. Si vous construisez la chaîne à la main,
l'appel pertinent se trouve sur `Application` :

```rust
// cmd/main.rs (or src/main.rs for the api starter)
Application::new()
    .config(my_app::config::register)
    .bootstrap(my_app::bootstrap::bootstrap)
    .routes(my_app::routes::register)
    .schedule(my_app::schedule::register)        // <- cette ligne
    .migrations::<my_app::migrations::Migrator>()
    .run()
    .await;
```

Sans `.schedule(...)`, les sous-commandes `schedule:*` rapportent toutes
qu'aucune tâche n'est enregistrée. `schedule:work` et `schedule:run`
exécutent aussi les mêmes drivers de runtime et le même `bootstrap_fn` que
le serveur HTTP, si bien que les observateurs, les écouteurs, et les
liaisons du conteneur enregistrés à l'amorçage sont visibles pour vos
handlers de tâche exactement comme ils le sont pour les contrôleurs (voir
[Amorçage de l'application](bootstrap.md)).

### Pourquoi Suprnova diverge

Le planificateur de Laravel est lui-même une unique commande Artisan
(`schedule:run`) que PHP-cron déclenche chaque minute. Le runtime PHP
démarre, évalue les tâches échues, les exécute in-process ou via un shell
externe, puis démonte le runtime. PHP n'a pas de processus de longue
durée, donc la forme daemon (`schedule:work`) a été rétroportée par Lumen
et est livrée dans Laravel lui-même comme contournement pour les sites
sans accès à crontab.

Dans Suprnova, le daemon est de premier ordre. `schedule:work` s'exécute à
l'intérieur d'un runtime Tokio déjà de longue durée, donc :

- **Les tâches en arrière-plan (`run_in_background`) se combinent avec la boucle de tick.** Laravel spawne un processus enfant par tâche en arrière-plan ; nous spawnons dans un `JoinSet` et faisons remonter les complétions au tick suivant ou à l'arrêt.
- **L'arrêt gracieux est un bras de `tokio::select!`.** Ctrl-C / SIGTERM vide les tâches en arrière-plan en vol avant la sortie ; les tâches in-process terminent leur appel courant.
- **Le dédoublonnage à la même minute est un état in-process.** Un atomique `last_run_minute` par tâche garantit qu'un seul processus ne peut pas déclencher deux fois une tâche alignée sur la minute, même si la boucle exécute ses ticks rapidement. PHP ne peut pas faire ça - chaque tick de cron est un processus neuf - c'est pourquoi Laravel utilise des verrous de système de fichiers comme seule ligne de défense.

Le `without_overlapping` adossé à `Cache::lock` existe toujours pour le
cas multi-processus (cron système sur plusieurs hôtes, plusieurs daemons
`schedule:work` derrière un load balancer). C'est le même mécanisme,
simplement à une couche dont le planificateur n'a pas toujours besoin.

## Résumé

| Fonctionnalité | Usage |
|---------|-------|
| Créer une tâche | `suprnova make:task TaskName` |
| Basé sur un trait | Implémentez le trait `Task`, configurez la planification à l'enregistrement |
| Basé sur une closure | `schedule.call(\|\| async { ... })` |
| Enregistrer des tâches | `schedule.add(schedule.task(...).daily().name("..."))` |
| Câbler dans l'application | `Application::new().schedule(schedule::register)` |
| Exécuter une fois | `suprnova schedule:run` |
| Exécuter le daemon | `suprnova schedule:work` |
| Lister les tâches | `suprnova schedule:list` |
| Empêcher le chevauchement | `.without_overlapping()` (TTL de verrou par défaut de 30 min via un backend Cache) |
| TTL de chevauchement personnalisé | `.without_overlapping_for(Duration)` |
| Arrière-plan | `.run_in_background()` (isolé des paniques via JoinSet) |
| Dédoublonnage à la même minute | Toujours actif par processus ; les exécutions sautées retournent `Ok(())` |
| Cron validé à l'exécution | `.try_cron(expr)` / `.try_daily_at(s)` / `.try_hourly_at(n)` |

## Suivant

- [Commandes de planification](cli-scheduling.md) - référence CLI `schedule:run` / `schedule:work` / `schedule:list`
- [File d'attente](queues.md) - pour le travail qui devrait être récupéré par un worker plutôt que rythmé par une horloge
- [Console](console.md) - `#[command]` pour les tâches d'opérateur ponctuelles (hors planification)
- [Cache](cache.md) - le backend qui alimente `without_overlapping` inter-processus
- [Amorçage de l'application](bootstrap.md) - comment `.schedule(...)` se branche dans le builder, et ce que les tâches peuvent résoudre depuis le conteneur
