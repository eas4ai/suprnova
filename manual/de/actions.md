# Aktionen

Eine Aktion in Suprnova ist eine Struktur mit einer einzigen Aufgabe: ein
einzelnes Stück Geschäftslogik hinter einer Methode zu halten. Sie ist
das Rust-Äquivalent zu Laravels aufrufbaren Single-Action-Controllern -
`RegisterUser`, `PublishPost`, `ChargeInvoice`. Die Aktion lebt in
`src/actions/`, trägt das `#[injectable]`-Attribut, damit der Container
sie auflösen kann, und stellt eine `execute(...)`-Methode bereit, die
Controller (und Jobs und andere Aktionen) aufrufen. Es gibt kein
`#[action]`-Makro und keine framework-seitige Durchsetzung von „einer
Methode“ - die Form ist eine Konvention, und `#[injectable]` ist die
Maschinerie, die diese Konvention schmerzlos macht.

```rust
use suprnova::{injectable, FrameworkError};

#[injectable]
pub struct RegisterUserAction {
    // Abhängigkeiten als Felder injizieren - siehe „Abhängigkeiten“ unten
}

impl RegisterUserAction {
    pub async fn execute(&self, email: &str) -> Result<String, FrameworkError> {
        tracing::info!(action = "RegisterUser", email, "executed");
        Ok(format!("registered: {email}"))
    }
}
```

Lösen Sie sie aus einem Handler mit `App::resolve::<RegisterUserAction>()?`
auf, und Sie haben Ihre Domänenlogik von der HTTP-Schicht getrennt, ohne
eine Basisklasse für eine Service-Schicht zu erfinden. Das ist das ganze
Muster.

## Eine Aktion generieren

```bash
suprnova make:action RegisterUser
```

Die CLI normalisiert den Namen zu PascalCase, hängt `Action` an, falls
das Suffix fehlt, und wandelt den Dateinamen dann in snake_case um.
Also:

| `make:action <Name>` | Struktur-Name | Datei |
|---|---|---|
| `RegisterUser` | `RegisterUserAction` | `src/actions/register_user_action.rs` |
| `SendNotification` | `SendNotificationAction` | `src/actions/send_notification_action.rs` |
| `ProcessPayment` | `ProcessPaymentAction` | `src/actions/process_payment_action.rs` |
| `ChargeInvoiceAction` | `ChargeInvoiceAction` | `src/actions/charge_invoice_action.rs` |

Der Generator schreibt die Datei und fügt `src/actions/mod.rs` eine
Zeile `pub mod register_user_action;` hinzu. Der ausgegebene Stub
kompiliert sofort:

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

Die Signatur - `async fn execute(&self) -> Result<_, FrameworkError>` -
ist die produktionssichere Form: asynchron, mit einem `Result`, das sich
über `?` direkt am Aufrufort in eine `HttpResponse` umwandelt. Der
Rumpf ist ein Platzhalter; tauschen Sie ihn gegen den echten Workflow
aus.

## Das `#[injectable]`-Attribut

`#[injectable]` ist das einzige Stück Framework-Maschinerie, auf das
sich das Aktions-Muster stützt. Es expandiert zu drei Dingen:

1. Einem `#[derive(Clone)]` auf der Struktur (und `Default`, wenn es
   keine `#[inject]`-Felder gibt).
2. Einem `inventory::submit!`-Eintrag, damit der Boot den Typ entdecken
   kann.
3. Einer Auto-Registrierungs-Closure, die `App::singleton_if_absent`
   einmal während `boot_services()` ausführt.

Der Vertrag des Makros:

| Struktur-Form | Verhalten |
|---|---|
| Unit-Struktur (`pub struct Foo;`) | Leitet `Default + Clone` ab, registriert `Default::default()` |
| Benannte Felder, keines `#[inject]` | Leitet `Default + Clone` ab, registriert `Default::default()` |
| Benannte Felder mit `#[inject]` | Leitet nur `Clone` ab; jedes `#[inject]`-Feld wird beim Boot aus dem Container aufgelöst, Nicht-Inject-Felder erhalten ihren Standardwert |
| Tuple-Struktur | Zur Compile-Zeit abgelehnt - „use named fields instead“ |

Eine aufgelöste Aktion ist ein Klon des gespeicherten Singletons. Die
Kosten sind ein `Clone` pro `App::resolve::<Action>()?`-Aufruf, was für
eine Unit-Struktur oder eine Struktur aus `Arc`-umschlossenen Services
eine Handvoll Referenzzähler-Erhöhungen ist. Schwerer Zustand gehört
hinter `Arc<dyn …>`-Services, die die Aktion injiziert, nicht in die
Aktion selbst.

### `#[inject]` passiert beim Boot, nicht bei jedem Aufruf

Wenn das Framework bootet, durchläuft `App::boot_services()` jede
`#[injectable]`-Registrierung und führt sie in einer
Fixpunkt-Retry-Schleife aus. Jeder Eintrag versucht, seine
`#[inject]`-Felder aus dem Container aufzulösen. Ist eine Abhängigkeit
noch nicht registriert, verschiebt sich der Eintrag auf die nächste
Iteration. Die Schleife läuft, bis entweder jeder Eintrag erfolgreich
ist oder kein Fortschritt mehr gemacht wird - und im Fehlerfall liefert
das Framework einen strukturierten Fehler, der den nicht auflösbaren
Typ oder den Zyklus benennt.

Die praktische Konsequenz: **`App::resolve::<MyAction>()` klont das
bereits konstruierte Singleton.** Es führt bei jedem Aufruf keine
`#[inject]`-Auflösung aus. Alles Injizierbare, von dem eine Aktion
abhängt, muss selbst vor der Aktion registriert sein - entweder über
sein eigenes `#[injectable]`-Attribut oder durch ein manuelles
`App::bind` / `App::singleton` in Ihrer `bootstrap()`-Funktion. Die
Retry-Schleife übernimmt die Reihenfolge im Inventory für Sie; sie
erfindet keine fehlenden Services.

## Eine Aktion aus einem Controller verwenden

Die Standard-Handler-Form: auflösen, ausführen, rendern.

```rust
use suprnova::{App, Request, Response, ResponseExt, json_response};

use crate::actions::register_user_action::RegisterUserAction;

pub async fn store(_req: Request) -> Response {
    let action = App::resolve::<RegisterUserAction>()?;
    let result = action.execute("alice@example.com").await?;

    json_response!({ "ok": true, "result": result }).status(201)
}
```

Beide `?`-Stellen funktionieren, weil sich beide Fehlertypen über
`From`-Implementierungen in `HttpResponse` umwandeln -
`App::resolve` gibt `Result<T, FrameworkError>` zurück, und der
Framework-Fehlerkonverter erledigt den Rest. Eine fehlende
Service-Registrierung erscheint als 500 mit dem Servicenamen im
strukturierten Log, nicht als Panic. Siehe
[Fehlermodell](error-model.md) für das vollständige Bild.

Wenn Sie das `?` beim Resolve lieber vermeiden möchten - zum
Beispiel in einem Pfad, der beim Boot hart fehlschlagen soll -
gibt `App::get::<RegisterUserAction>()` ein `Option<T>` zurück, und
Sie können mit `.expect("registered at boot")` sichtbar scheitern,
falls Sie die Verdrahtung falsch gemacht haben.

## Asynchrone Aktionen, die die Datenbank berühren

Das ist der Pfad, den die meisten Aktionen tatsächlich nehmen - laden
oder schreiben über ein Eloquent-Modell. Heben Sie den Rumpf aus Ihrer
Domäne; die Oberfläche ist dieselbe.

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

`Todo::create(attrs!{...})` und `Todo::all()` stammen aus dem
`#[suprnova::model]`-Makro. Siehe [Eloquent](eloquent.md) für die
Modell-Oberfläche. Beachten Sie, dass `Model::all()` eine
`Collection<Todo>` zurückgibt - das Beispiel ruft `.into_vec()` auf, um
dem Controller einen einfachen `Vec` zu übergeben; Sie können die
`Collection` auch direkt zurückgeben und sie vom Serialisierer rendern
lassen.

Das in einen Controller verdrahten:

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

Zwei `?` pro Handler; der Controller bleibt ein dünner Adapter zwischen
HTTP und der Domäne.

## Abhängigkeiten über `#[inject]`

Wenn eine Aktion Mitwirkende braucht - einen Mailer, einen Logger, einen
Domänen-Service - deklarieren Sie sie als Felder und markieren Sie jedes
mit `#[inject]`:

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

Sowohl `MailerService` als auch `LoggerService` müssen selbst
container-registriert sein, bevor diese Aktion bootet - entweder mit
ihrem eigenen `#[injectable]`-Attribut oder durch einen
`bootstrap()`-Aufruf:

```rust
// In src/bootstrap.rs
App::singleton(MailerService::from_env()?);
App::singleton(LoggerService::default());
```

Fehlt eine der beiden Abhängigkeiten, wenn der Boot die
Fixpunkt-Schleife ausführt, liefert der Boot einen Fehler, der den
nicht aufgelösten Typ benennt, und das Framework beendet sich mit
einem Fehlercode, statt mit einem halb verdrahteten Container zu
starten.

Nicht-`#[inject]`-Felder fallen auf `Default::default()` zurück, sodass
Sie injizierte Abhängigkeiten mit einfachem Zustand mischen können, ohne
einen Konstruktor zu schreiben.

## Wann eine Aktion verwenden

Die Faustregel: Eine Aktion existiert, wenn dasselbe Stück Arbeit von
mehr als einem Einstiegspunkt ausgelöst wird (oder werden könnte). Ein
Registrierungs-Flow, der sowohl von einer HTTP-Route als auch von einem
eingereihten Job läuft, gehört in `RegisterUserAction`. Ein einmaliger
„Rendere diese Index-Seite“-Handler braucht keine Aktion - lassen Sie
ihn im Controller.

| Guter Fit | Beispiel |
|---|---|
| Mehrstufige Geschäftsoperationen | `RegisterUserAction`, `CheckoutAction` |
| Zwischen HTTP + Queue geteilte Arbeit | `IssueRefundAction` (auf beiden Wegen dispatcht) |
| Logik, die es wert ist, ohne Request getestet zu werden | `CalculateTotalsAction` |
| Externe Integrationen | `SendEmailAction`, `SyncInventoryAction` |
| Alles, was der Controller sonst inline duplizieren würde | Rule-of-three-Auslöser |

Im Vergleich zu einem Controller ist eine Aktion wiederverwendbar, hat
keine `Request`-Bindung und ist trivial aus einem Test aufzurufen
(`App::resolve` + `await`). Ein Controller bleibt eine HTTP-bewusste
Grenze, die weiß, wie sich das Ergebnis einer Aktion in eine `Response`
übersetzen lässt.

| Controller | Aktion |
|---|---|
| Behandelt eine Route | Wiederverwendbar über Routen, Jobs, Zeitpläne hinweg |
| Kennt `Request` / `Response` | Kennt Ihre Domänentypen |
| Gibt `Response` zurück | Gibt `Result<T, FrameworkError>` zurück |
| Ruft Aktionen auf | Wird von Controllern (und anderen) aufgerufen |

## Aktionen, der Bus und Queues

Aktionen sind nicht der einzige Ort, an dem Geschäftslogik leben kann -
der [Bus](bus.md) behandelt dispatchte Commands mit typisierten
Ausgaben, und die [Warteschlange](queues.md) behandelt Arbeit, die auf einem
Worker laufen soll. Wählen Sie danach, wie die Arbeit aufgerufen wird:

| Sie wollen … | Greifen Sie zu |
|---|---|
| Synchrone Geschäftslogik, aufrufbar aus einem Controller oder einem Job | **Aktion** (`#[injectable]` + `execute`) |
| Ein typisiertes Command mit registriertem Handler, aufrufbar über `Bus::dispatch` | [Bus](bus.md) |
| Dauerhafte, wiederholte Arbeit außerhalb des Requests | [Warteschlange](queues.md) |

Mischen ist in Ordnung: Ein `BusHandler` oder ein `Job` löst oft einfach
eine Aktion auf und ruft deren `execute` auf. Die Aktion hält die
Domänenlogik; der Bus oder die Queue hält die Dispatch-Metadaten.

## Dateilayout

Was `make:action` ausgibt, plus der Raum zum Gruppieren:

```
src/
├── actions/
│   ├── mod.rs                          // pub mod register_user_action;
│   ├── register_user_action.rs
│   ├── send_welcome_email_action.rs
│   └── billing/                        // nach Domäne gruppieren, wenn das Verzeichnis wächst
│       ├── mod.rs
│       ├── charge_invoice_action.rs
│       └── issue_refund_action.rs
├── controllers/
└── main.rs
```

Nichts im Framework verlangt dieses Layout; der Generator schreibt
nach `src/actions/`, weil das die Konvention ist. Verschieben Sie eine
Aktion nach `src/billing/actions/`, und sie funktioniert weiter -
`#[injectable]` ist ortsunabhängig.

## Eine Aktion testen

Weil eine Aktion nur eine container-auflösbare Struktur mit einer
`async`-Methode ist, ist die Test-Oberfläche `App::resolve` + `await`.
Dieselbe `TestDatabase`-Test-Fixture, die anderswo verwendet wird,
funktioniert auch hier:

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

Siehe [Testen](testing.md) für die vollständige `describe!` / `test!` /
`expect!`-Oberfläche und für `TestContainer::fake`, wenn Sie einen
Mailer- oder Gateway-Fake in eine getestete Aktion injizieren möchten.

## Warum Suprnova abweicht

Laravels Single-Action-Controller - Klassen mit einer
`__invoke`-Methode in `App\Actions\` - werden pro Anfrage konstruiert.
Der Container löst die Klasse auf, führt Konstruktor-Injection aus, und
die Instanz wird verworfen, sobald die Response abgeht. PHPs
Prozess-pro-Anfrage-Modell macht das praktisch kostenlos.

Suprnova-Aktionen sind container-residente Singletons: einmal beim
Boot gebaut, mit `#[inject]`-Feldern, die dann aufgelöst werden, und
bei jedem `App::resolve` herausgeklont. Das Muster passt zu Rust, weil
das Klonen einer Struktur aus `Arc`-umschlossenen Services eine
Handvoll Referenzzähler-Erhöhungen kostet, während das
Konstruieren-und-Verwerfen einer Struktur bei jeder Anfrage jedes Feld
durch eine Allokation zwingen würde. Die Laravel-förmige Konvention -
eine Struktur, eine Methode, benannt nach der Operation - überlebt
intakt; die Verdrahtung darunter ist für Tokio geformt.

Die andere bewusste Trennung: Controller bleiben freie Funktionen
(siehe [Controller](controllers.md)), sodass die HTTP-Schicht eine
reine Request-zu-Response-Transformation ohne eigene DI-Oberfläche
ist. Konstruktor-artige Injection passiert an der
`#[injectable]`-Grenze, innerhalb der Aktion, wo sie hingehört.

## Nächste Schritte

- [Controller](controllers.md) - die HTTP-seitigen freien Funktionen, die Aktionen auflösen und aufrufen
- [Service Container](container.md) - was `App::resolve`, `App::singleton` und das Drei-Ebenen-Lookup tatsächlich tun
- [Bus](bus.md) - typisierter Command-Dispatch, wenn Sie einen registrierten Handler statt einer aufgelösten Aktion wollen
- [Testen](testing.md) - `App::resolve` + `TestContainer::fake` für hermetische Aktions-Tests
- [Fehlermodell](error-model.md) - wie `?` bei `App::resolve::<Action>()?` und `action.execute().await?` in eine saubere Response kollabiert
