# Actions

Une action dans Suprnova est une struct qui a un seul travail : porter un
unique morceau de logique métier derrière une seule méthode. C'est
l'analogue Rust des contrôleurs invocables à une seule action de Laravel -
`RegisterUser`, `PublishPost`, `ChargeInvoice`. L'action vit dans
`src/actions/`, porte l'attribut `#[injectable]` pour que le conteneur
puisse la résoudre, et expose une méthode `execute(...)` que les
contrôleurs (et les jobs, et d'autres actions) appellent. Il n'existe pas
de macro `#[action]` ni d'application côté framework d'une règle « une
seule méthode » - la forme est une convention, et `#[injectable]` est la
machinerie qui rend cette convention indolore.

```rust
use suprnova::{injectable, FrameworkError};

#[injectable]
pub struct RegisterUserAction {
    // Injectez les dépendances comme champs - voir « Dépendances » ci-dessous
}

impl RegisterUserAction {
    pub async fn execute(&self, email: &str) -> Result<String, FrameworkError> {
        tracing::info!(action = "RegisterUser", email, "executed");
        Ok(format!("registered: {email}"))
    }
}
```

Résolvez-la depuis un handler avec `App::resolve::<RegisterUserAction>()?`
et vous avez séparé votre logique de domaine de la couche HTTP sans
inventer de classe de base pour une couche de service. C'est tout le
motif.

## Générer une action

```bash
suprnova make:action RegisterUser
```

Le CLI normalise le nom en PascalCase, ajoute `Action` si le suffixe est
absent, puis met le nom de fichier en snake_case. Donc :

| `make:action <Name>` | Nom de la struct | Fichier |
|---|---|---|
| `RegisterUser` | `RegisterUserAction` | `src/actions/register_user_action.rs` |
| `SendNotification` | `SendNotificationAction` | `src/actions/send_notification_action.rs` |
| `ProcessPayment` | `ProcessPaymentAction` | `src/actions/process_payment_action.rs` |
| `ChargeInvoiceAction` | `ChargeInvoiceAction` | `src/actions/charge_invoice_action.rs` |

Le générateur écrit le fichier et ajoute une ligne
`pub mod register_user_action;` à `src/actions/mod.rs`. Le stub émis
compile immédiatement :

```rust
//! register_user_action action

use suprnova::{injectable, FrameworkError};

/// RegisterUserAction
///
/// Single-responsibility command resolved from the container. Inject any
/// dependencies as fields and the `#[injectable]` macro wires them at
/// resolve time.
#[injectable]
pub struct RegisterUserAction {
    // Add injected dependencies as fields here, e.g.
    // db: suprnova::DbConnection,
}

impl RegisterUserAction {
    /// Execute the action.
    pub async fn execute(&self) -> Result<String, FrameworkError> {
        Ok("RegisterUserAction executed".to_string())
    }
}
```

La signature - `async fn execute(&self) -> Result<_, FrameworkError>` -
est la forme prête pour la production : async, retournant un `Result`
qui se convertit via `?` directement en `HttpResponse` au site d'appel.
Le corps est un placeholder ; remplacez-le par le vrai workflow.

## L'attribut `#[injectable]`

`#[injectable]` est le seul morceau de machinerie du framework dont
dépend le motif d'action. Il se développe en trois choses :

1. Un `#[derive(Clone)]` sur la struct (et `Default` quand il n'y a
   aucun champ `#[inject]`).
2. Une entrée `inventory::submit!` pour que l'amorçage puisse découvrir
   le type.
3. Une closure d'auto-enregistrement que `App::singleton_if_absent`
   exécute une fois durant `boot_services()`.

Le contrat de la macro :

| Forme de la struct | Comportement |
|---|---|
| Struct unitaire (`pub struct Foo;`) | Dérive `Default + Clone`, enregistre `Default::default()` |
| Champs nommés, aucun `#[inject]` | Dérive `Default + Clone`, enregistre `Default::default()` |
| Champs nommés avec `#[inject]` | Dérive uniquement `Clone` ; chaque champ `#[inject]` est résolu depuis le conteneur à l'amorçage, les champs non injectés prennent leur valeur par défaut |
| Struct tuple | Rejetée à la compilation - « utilisez des champs nommés à la place » |

Une action résolue est un clone du singleton stocké. Le coût est un
`Clone` par appel à `App::resolve::<Action>()?`, ce qui pour une struct
unitaire ou une struct de services enveloppés dans `Arc` représente une
poignée d'augmentations du compteur de références. L'état lourd doit se
trouver derrière des services `Arc<dyn …>` que l'action injecte, pas à
l'intérieur de l'action elle-même.

### `#[inject]` se produit à l'amorçage, pas à chaque appel

Quand le framework s'amorce, `App::boot_services()` parcourt chaque
enregistrement `#[injectable]` et les exécute dans une boucle de réessai
à point fixe. Chaque entrée essaie de résoudre ses champs `#[inject]`
depuis le conteneur. Si une dépendance n'a pas encore été enregistrée,
l'entrée reporte à l'itération suivante. La boucle s'exécute jusqu'à ce
que chaque entrée réussisse ou qu'aucun progrès ne soit fait - et en cas
d'échec, le framework retourne une erreur structurée nommant le type non
résolu ou le cycle.

La conséquence pratique : **`App::resolve::<MyAction>()` clone le
singleton déjà construit**. Cela n'exécute pas la résolution `#[inject]`
à chaque appel. Tout ce qui est injectable dont dépend une action doit
lui-même être enregistré avant l'action - soit via son propre attribut
`#[injectable]`, soit par un `App::bind` / `App::singleton` manuel dans
votre fonction `bootstrap()`. La boucle de réessai gère l'ordonnancement
de l'inventaire pour vous ; elle n'invente pas les services manquants.

## Utiliser une action depuis un contrôleur

La forme standard d'un handler : résoudre, exécuter, rendre.

```rust
use suprnova::{App, Request, Response, ResponseExt, json_response};

use crate::actions::register_user_action::RegisterUserAction;

pub async fn store(_req: Request) -> Response {
    let action = App::resolve::<RegisterUserAction>()?;
    let result = action.execute("alice@example.com").await?;

    json_response!({ "ok": true, "result": result }).status(201)
}
```

Les deux `?` fonctionnent parce que les deux types d'erreur se
convertissent en `HttpResponse` via des impls `From` - `App::resolve`
retourne `Result<T, FrameworkError>` et le convertisseur d'erreurs du
framework fait le reste. Un enregistrement de service manquant se
manifeste comme un 500 avec le nom du service dans le journal structuré,
pas une panique. Voir [Modèle d'erreur](error-model.md) pour la vue
d'ensemble complète.

Si vous préférez éviter le `?` sur la résolution - par exemple dans un
chemin qui devrait échouer explicitement dès l'amorçage -
`App::get::<RegisterUserAction>()` retourne `Option<T>` et vous pouvez
faire `.expect("registered at boot")` pour échouer explicitement si le
câblage est mauvais.

## Actions async qui touchent la base de données

C'est le chemin qu'empruntent la plupart des actions en pratique -
charger ou écrire via un modèle Eloquent. Reprenez le corps depuis votre
domaine ; la surface est la même.

```rust
use suprnova::{attrs, injectable, FrameworkError, Model};

use crate::models::todos::Todo;

#[injectable]
pub struct CreateRandomTodoAction;

impl CreateRandomTodoAction {
    pub async fn execute(&self) -> Result<Todo, FrameworkError> {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
            % 10000;

        Todo::create(attrs! {
            title: format!("Todo #{}", n),
            description: format!("created at {}", n),
            done: false,
        })
        .await
    }
}

#[injectable]
pub struct ListTodosAction;

impl ListTodosAction {
    pub async fn execute(&self) -> Result<Vec<Todo>, FrameworkError> {
        Ok(<Todo as suprnova::eloquent::Model>::all().await?.into_vec())
    }
}
```

`Todo::create(attrs!{...})` et `Todo::all()` proviennent de la macro
`#[suprnova::model]`. Voir [Eloquent](eloquent.md) pour la surface du
modèle. Notez que `Model::all()` retourne une `Collection<Todo>` -
l'exemple appelle `.into_vec()` pour remettre un `Vec` simple au
contrôleur ; vous pouvez aussi retourner directement la `Collection` et
laisser le sérialiseur la rendre.

Câbler tout ça dans un contrôleur :

```rust
use suprnova::{App, Request, Response, ResponseExt, json_response};

use crate::actions::todo_action::{CreateRandomTodoAction, ListTodosAction};

pub async fn create_random(_req: Request) -> Response {
    let action = App::resolve::<CreateRandomTodoAction>()?;
    let todo = action.execute().await?;
    json_response!({ "ok": true, "todo": todo }).status(201)
}

pub async fn list(_req: Request) -> Response {
    let action = App::resolve::<ListTodosAction>()?;
    let todos = action.execute().await?;
    json_response!({ "ok": true, "todos": todos })
}
```

Deux `?` par handler ; le contrôleur reste un adaptateur mince entre HTTP
et le domaine.

## Dépendances via `#[inject]`

Quand une action a besoin de collaborateurs - un mailer, un logger, un
service de domaine - déclarez-les comme champs et étiquetez chacun avec
`#[inject]` :

```rust
use suprnova::{injectable, FrameworkError};

use crate::services::{MailerService, LoggerService};

#[injectable]
pub struct SendWelcomeEmailAction {
    #[inject]
    mailer: MailerService,
    #[inject]
    logger: LoggerService,
}

impl SendWelcomeEmailAction {
    pub async fn execute(&self, to: &str) -> Result<(), FrameworkError> {
        self.logger.info(&format!("welcome → {to}"));
        self.mailer.send_welcome(to).await
    }
}
```

`MailerService` et `LoggerService` doivent tous deux être eux-mêmes
enregistrés dans le conteneur avant que cette action ne s'amorce - soit
avec leur propre attribut `#[injectable]`, soit par un appel dans
`bootstrap()` :

```rust
// Dans src/bootstrap.rs
App::singleton(MailerService::from_env()?);
App::singleton(LoggerService::default());
```

Si l'une ou l'autre dépendance est manquante quand l'amorçage exécute la
boucle à point fixe, l'amorçage retourne une erreur nommant le type non
résolu et le framework quitte avec un code non nul plutôt que de
démarrer avec un conteneur à moitié câblé.

Les champs non `#[inject]` retombent sur `Default::default()`, donc vous
pouvez mélanger dépendances injectées et état simple sans écrire de
constructeur.

## Quand utiliser une action

La règle empirique : une action existe quand le même travail est (ou
pourrait être) déclenché depuis plus d'un point d'entrée. Un flux
d'inscription qui s'exécute à la fois depuis une route HTTP et un job en
file d'attente a sa place dans `RegisterUserAction`. Un handler ponctuel
« rendre cette page d'index » n'a pas besoin d'une action - gardez-le
dans le contrôleur.

| Bon candidat | Exemple |
|---|---|
| Opérations métier en plusieurs étapes | `RegisterUserAction`, `CheckoutAction` |
| Travail partagé entre HTTP + file d'attente | `IssueRefundAction` (dispatchée dans les deux sens) |
| Logique qui mérite d'être testée sans requête | `CalculateTotalsAction` |
| Intégrations externes | `SendEmailAction`, `SyncInventoryAction` |
| Tout ce que le contrôleur mettrait sinon en ligne + dupliquerait | déclencheur de la règle des trois |

Comparée à un contrôleur, une action est réutilisable, n'a pas de
liaison à `Request`, et est triviale à appeler depuis un test
(`App::resolve` + `await`). Un contrôleur reste une frontière consciente
de HTTP qui sait traduire le résultat d'une action en `Response`.

| Contrôleur | Action |
|---|---|
| Gère une seule route | Réutilisable à travers les routes, jobs, planifications |
| Connaît `Request` / `Response` | Connaît vos types de domaine |
| Retourne `Response` | Retourne `Result<T, FrameworkError>` |
| Appelle des actions | Appelée par les contrôleurs (et d'autres) |

## Actions, le bus, et les files d'attente

Les actions ne sont pas le seul endroit où la logique métier peut
vivre - le [Bus](bus.md) gère les commandes dispatchées avec des
sorties typées, et la [File d'attente](queues.md) gère le travail qui
doit s'exécuter sur un worker. Choisissez selon la façon dont le
travail est invoqué :

| Vous voulez… | Utilisez |
|---|---|
| De la logique métier synchrone, appelable depuis un contrôleur ou un job | **Action** (`#[injectable]` + `execute`) |
| Une commande typée avec un handler enregistré, appelable via `Bus::dispatch` | [Bus](bus.md) |
| Du travail durable, réessayé, en arrière-plan | [File d'attente](queues.md) |

Mélanger les deux est très bien : un `BusHandler` ou un `Job` se
contente souvent de résoudre une action et d'appeler son `execute`.
L'action porte la logique de domaine ; le bus ou la file d'attente porte
les métadonnées de dispatch.

## Disposition des fichiers

Ce que `make:action` émet, plus la place pour regrouper :

```
src/
├── actions/
│   ├── mod.rs                          // pub mod register_user_action;
│   ├── register_user_action.rs
│   ├── send_welcome_email_action.rs
│   └── billing/                        // regrouper par domaine quand le répertoire grossit
│       ├── mod.rs
│       ├── charge_invoice_action.rs
│       └── issue_refund_action.rs
├── controllers/
└── main.rs
```

Rien dans le framework n'exige cette disposition ; le générateur écrit
dans `src/actions/` parce que c'est la convention. Déplacez une action
vers `src/billing/actions/` et elle continuera de fonctionner -
`#[injectable]` est indépendant de l'emplacement.

## Tester une action

Parce qu'une action n'est qu'une struct résoluble par le conteneur avec
une méthode `async`, la surface de test est `App::resolve` + `await`. La
même fixture de test `TestDatabase` utilisée ailleurs fonctionne ici :

```rust
use suprnova::{describe, expect, test, App};
use suprnova::testing::TestDatabase;

use crate::actions::todo_action::ListTodosAction;
use crate::models::todos::Todo;

describe!("ListTodosAction", {
    test!("returns all todos", async fn(_db: TestDatabase) {
        Todo::create(suprnova::attrs! { title: "Test", description: "", done: false })
            .await
            .unwrap();

        let action = App::resolve::<ListTodosAction>().unwrap();
        let todos = action.execute().await.unwrap();

        expect!(todos).to_have_length(1);
    });
});
```

Voir [Tests](testing.md) pour la surface complète de `describe!` /
`test!` / `expect!` et pour `TestContainer::fake` quand vous voulez
injecter un fake-mailer ou un fake-gateway dans une action en cours de
test.

## Pourquoi Suprnova diverge

Les contrôleurs à action unique de Laravel - des classes avec une
méthode `__invoke` dans `App\Actions\` - sont construits par requête. Le
conteneur résout la classe, exécute l'injection par constructeur, et
l'instance est jetée quand la réponse repart. Le modèle
process-per-request de PHP rend cela essentiellement gratuit.

Les actions de Suprnova sont des singletons résidant dans le conteneur :
construites une fois à l'amorçage avec les champs `#[inject]` résolus à
ce moment-là, puis clonées à chaque `App::resolve`. Le motif convient à
Rust parce que cloner une struct de services enveloppés dans `Arc` coûte
quelques augmentations du compteur de références, alors que
construire-puis-jeter une struct à chaque requête forcerait chaque champ
à passer par l'allocation. La convention à la Laravel - une struct, une
méthode, nommée pour l'opération - survit intacte ; le câblage en
dessous est conçu pour Tokio.

L'autre séparation intentionnelle : les contrôleurs restent des
fonctions libres (voir [Contrôleurs](controllers.md)), si bien que la
couche HTTP est une transformation pure requête-vers-réponse sans
surface d'injection de dépendances qui lui soit propre. L'injection
façon constructeur se produit à la frontière `#[injectable]`, à
l'intérieur de l'action, là où est sa place.

## Suivant

- [Contrôleurs](controllers.md) - les fonctions libres orientées HTTP qui résolvent et appellent les actions
- [Conteneur de service](container.md) - ce que font réellement `App::resolve`, `App::singleton`, et la recherche à trois couches
- [Bus](bus.md) - dispatch de commande typée quand vous voulez un handler enregistré plutôt qu'une action résolue
- [Tests](testing.md) - `App::resolve` + `TestContainer::fake` pour des tests d'action hermétiques
- [Modèle d'erreur](error-model.md) - comment le `?` sur `App::resolve::<Action>()?` et `action.execute().await?` s'effondre en une réponse propre
