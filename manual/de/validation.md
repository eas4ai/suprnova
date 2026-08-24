# Validierung

Suprnova validiert Request-Eingaben auf zwei komplementären Wegen:

1. **Derive-Validierung** - `#[validate(...)]`-Attribute auf einer
   `FormRequest`-Struktur, die `extract()` automatisch ausführt. Das ist
   der alltägliche Weg, und er ist in [Anfragen](requests.md)
   beschrieben. Er behandelt Regeln pro Feld (`email`, `length`,
   `range`, …) deklarativ.
2. **Regelobjekte + das `validate!`-Makro** - schlichte Werte, die
   [`Rule`](#regelobjekte) / `ContextualRule` / `AsyncRule`
   implementieren und imperativ komponiert werden. Dazu greifen Sie,
   wenn Sie feldübergreifende Logik brauchen, Regeln, die die Datenbank
   berühren, oder Regeln, die Sie speichern und herumreichen wollen.

Beide Wege sammeln sich in derselben
[`ValidationErrors`](error-model.md)-Bag und rendern dieselbe
Laravel-/Inertia-Form `{ "message", "errors": { field: [...] } }` (HTTP
422).

## Regelobjekte

Eine Regel ist ein Wert, der eines von vier Traits implementiert:

| Trait | Form | Verwendung |
|-------|-------|-----|
| `Rule` | `passes(&self, value: &str)` | reine Prüfung eines einzelnen Werts |
| `ValueRule` | `passes(&self, value: &serde_json::Value)` | Prüfung eines JSON-geformten Werts (Array/Objekt) |
| `ContextualRule` | `passes(&self, value, ctx)` | Prüfung, die Nachbarfelder liest |
| `AsyncRule` | `async passes(&self, value)` | Prüfung, die `.await`et (DB, HTTP) |

Eingebaute `Rule`s: `Required`, `Email`, `Min`, `Max`, `Between`, `In`,
`NotIn`, `Integer`, `Numeric`, `Boolean`, `Alpha`, `AlphaNum`, `Url`,
`UrlProtocols`, `HttpUrl`, `Uuid`. Eingebaute `ValueRule`s: `ArrayKeys`,
`Distinct`. Eingebaute `ContextualRule`s: `RequiredIf`, `RequiredWith`,
`RequiredUnless`, `Same`, `Different`, `Confirmed`. Eingebaute
`AsyncRule`: [`Unique`](#die-unique-regel).

```rust
use suprnova::{Rule, rules::Email};

Email.passes("user@example.com")?; // Ok(())
```

> **Hinweis:** `Numeric` akzeptiert eine **endliche** Zahl - `NaN`, `inf`
> und Größenordnungen, die zu Unendlich überlaufen, werden abgelehnt,
> auch wenn Rusts Parser die Zeichenketten akzeptieren würde.

### URL-Schemata

`Url` akzeptiert einen Wert, der als URL parst, dessen Schema auf
Laravels Allowlist steht - derselben Liste, die
`Illuminate\Support\Str::isUrl` verwendet -, von `://` gefolgt wird
**und** darauf wiederum von einem nicht leeren Host, was Laravels Muster
`^(PROTOCOLS)://HOST` in der Form entspricht (Laravels Host-Gruppe hat
kein `?` - ein fehlender oder leerer Host matcht nie). Die Schema-Liste
und die Anforderung aus `://` plus Host sind wörtlich Laravels; der Host
wird vom `url`-Crate geparst statt von Laravels Regex, sodass hier ein
Port außerhalb des gültigen Bereichs abgelehnt wird, den Laravel
akzeptieren würde. Alle drei Bedingungen müssen gelten: `mailto:`,
`tel:` und `data:` stehen namentlich auf der Allowlist, tragen aber
überhaupt keine Authority-Komponente, also lehnt `Url` sie ab; und
`file:///etc/passwd` scheitert aus dem dritten Grund - es hat `://`,
aber zwischen dem dritten und dem vierten `/` steht nichts, und nichts
ist auch kein Host. `javascript:` und `vbscript:` werden rundweg
abgelehnt; sie stehen gar nicht erst auf der Allowlist.

`ftp://host/x` und `ssh://host` - echte Hosts, nur eben keine
Web-Schemata - kommen weiterhin durch, `Url` ist also keine Prüfung
„das ist eine Webseite“, und es sagt nichts darüber aus, wohin die URL
auflöst. Dass `javascript:` abgelehnt wird, macht einen validierten Wert
sicher genug, um ihn in ein `href` zu setzen, nicht sicher genug, um ihn
abzurufen. Ein Webhook- oder Callback-Ziel braucht weiterhin `HttpUrl`
(oder Ihre eigenen Schema- und SSRF-Prüfungen); `Url` allein deckt das
nicht ab.

Für eine engere Menge benennen Sie die gewünschten Schemata:

```rust
use suprnova::{Rule, rules::Url};

// Laravels `url:http,https`
Url::protocols(&["https"]).passes("https://example.com")?;   // Ok
Url::protocols(&["https"]).passes("http://example.com");     // Err

// Dasselbe, unter einem Namen
use suprnova::rules::HttpUrl;
HttpUrl.passes("https://example.com")?;
```

`Url::protocols(...)` **ersetzt** die Allowlist, statt sie einzuengen,
sodass eine App ihr eigenes Deep-Link-Schema (`myapp://…`) akzeptieren
kann, ohne dass das Framework dazu eine Meinung hat - die Anforderung
aus `://` plus Host gilt auch für dieses eigene Schema. Verwenden Sie
`HttpUrl` (oder `Url::protocols(&["https"])`) für Callback-, Webhook-
und Avatar-Eingaben - ein Webhook-Ziel, das zu `ftp://internal-host/`
auflöst, parst weiterhin als `Url`, und ein `ftp:`-Ziel ist kein
Webhook-Ziel.

### Eine eigene Regel schreiben

Eine eigene Regel ist eine Unit-Struktur (oder eine datentragende) mit
einer einzigen Impl. Der Trait gibt Ihnen `check()` kostenlos - es legt
jede Fehlermeldung unter dem benannten Feld in eine
`ValidationErrors`-Bag - sodass sich die Regel unverändert in
`validate!` und die `after_validation`-Hooks einfügt:

```rust
use suprnova::{Rule, ValidationMessage};

pub struct StartsWith(pub &'static str);

impl Rule for StartsWith {
    fn passes(&self, value: &str) -> Result<(), ValidationMessage> {
        if value.starts_with(self.0) {
            Ok(())
        } else {
            Err(format!("must start with {}", self.0).into())
        }
    }
}

// Jetzt überall verwendbar:
StartsWith("acct_").passes("acct_1234")?;
// oder, in einer validate!-Zeile:
//   stripe_id => Required, StartsWith("acct_");
```

Ein `String` konvertiert in eine `ValidationMessage`, die wörtlich
gerendert wird, und mehr braucht eine einsprachige App nicht. Damit die
Meldung pro Locale übersetzt wird, liefern Sie stattdessen eine
*geschlüsselte* Meldung zurück -
`ValidationMessage::keyed("validation-starts-with").arg("prefix", self.0).fallback(…)` -
und definieren die ID in `lang/<locale>/validation.ftl`. Siehe
[Lokalisierung](localization.md); dort steht auch, wie sich die
Meldungen der eingebauten Regeln überschreiben lassen und wie die
Namenskonvention `field-<name>` funktioniert.

Für feldübergreifende Logik implementieren Sie stattdessen
[`ContextualRule`] - die Methode `passes` bekommt neben dem geprüften
Wert einen `&FormContext` (eine `HashMap<String, String>` mit den Werten
der Nachbarfelder). Für datenbankgestützte Prüfungen implementieren Sie
[`AsyncRule`] und verwenden es aus `after_validation_async`.
### Regeln mit Wertform

`Rule` sieht immer nur `&str`. Zwei eingebaute Regeln brauchen mehr
Struktur als ein String trägt, daher implementieren sie stattdessen
`ValueRule` über `&serde_json::Value`:

```rust
use suprnova::{ValueRule, rules::{ArrayKeys, Distinct}};

// Laravels array:keys - weist Schlüssel außerhalb der erlaubten Menge
// zurück. Aufgelistete Schlüssel müssen nicht alle vorhanden sein; eine
// leere erlaubte Liste ist ein Programmierfehler, der als Meldung ohne
// Feld gemeldet wird.
ArrayKeys(&["name", "email"]).passes(&serde_json::json!({"name": "Ada"}))?;

// Laravels distinct / distinct:ignore_case / distinct:strict.
Distinct { ignore_case: false, strict: false }
    .passes(&serde_json::json!(["a", "b", "c"]))?;
```

Ein durch eine `ValueRule` validiertes Feld muss selbst ein
`serde_json::Value` enthalten (oder `Option<serde_json::Value>` für eine
`?:`-/`?=>`-Zeile) - typischerweise ein Request-Feld, das direkt aus dem
JSON-Body stammt. `validate!`-Zeilen akzeptieren `Rule` und `ValueRule`
in derselben Feldliste; welcher Trait läuft, wird dadurch aufgelöst,
welchen Trait der Typ der Regel implementiert, nicht durch etwas, das
Sie in der Zeile schreiben.

### Warum Suprnova abweicht

Laravels `distinct:strict` stützt sich auf PHPs erzwungenes `==`.
JSON-Werte sind bereits typisiert, daher ändert `strict` bei Suprnova
nur, ob zwei Zahlen mit unterschiedlicher interner Darstellung
(`1` gegenüber `1.0`) als gleich gelten - ein String und eine Zahl
werden in keinem Modus als „dasselbe“ betrachtet.


## Das `validate!`-Makro

`validate!` lässt eine Kette von Regeln über die Felder einer Struktur
laufen und sammelt jeden Fehlschlag in einem einzigen
`ValidationErrors`. Es ist die idiomatische Heimat des synchronen
feldübergreifenden Hooks [`after_validation`](#feldübergreifende-hooks).

```rust
use suprnova::{validate, ValidationErrors, rules::{Required, Email, Min, Max, RequiredIf}};

fn after_validation(&self) -> Result<(), ValidationErrors> {
    // Kontextuelle Regeln lesen die Werte der Nachbarfelder aus einem
    // `FormContext`, den Sie bauen - eine Map von Feldname auf String-Wert.
    let mut ctx = std::collections::HashMap::new();
    ctx.insert("billing_type".to_string(), self.billing_type.clone());
    validate! { self =>
        email       => Required, Email;          // Zeile in Pflichtform
        bio         ?: Min(10), Max(500);        // optional: nur validieren, wenn Some
        card_number ?=> RequiredIf {             // bedingtes Vorhandensein (siehe unten)
            other: "billing_type",
            value: "card",
        } => with ctx;
    }
}
```

Jede Zeile hat eine von drei Formen:

- **`field => Rule1, Rule2;`** - Pflichtform. Die Regeln laufen direkt
  auf `&self.field` (für `String`, `i64` oder alles, was sich auf die
  von der Regel erwartete Referenz dereferenzieren lässt) - oder bei
  einer `ValueRule` direkt auf einem `serde_json::Value`-Feld. Welchen
  Trait die jeweilige Regel verwendet, wird automatisch abgeleitet.
- **`field ?: Rule1, Rule2;`** - optional. Das Feld ist `Option<T>`; die
  Regeln laufen nur, wenn es `Some` ist, und werden bei `None`
  **vollständig übersprungen**. Das ist Laravels Semantik "falls
  vorhanden, validieren" (`sometimes`).
- **`field ?=> Rule1, Rule2;`** - bedingtes Vorhandensein. Ebenfalls für
  ein `Option<String>`-Feld, aber die Regeln laufen **auch bei `None`**
  (Abwesenheit wird als leerer String behandelt). Das ist die Zeile für
  Regeln, deren Bedingung am Vorhandensein hängt, etwa `RequiredIf` -
  die *ein fehlendes Feld durchfallen lassen* können müssen. Genau das
  kann `?:` nicht ausdrücken, weil es bei `None` überspringt.

Auf eine kontextuelle Regel folgt `=> with $ctx` (eine
`&HashMap<String, String>` mit den Werten der Nachbarfelder). Das Makro
ist **synchron** - für asynchrone Regeln nehmen Sie den
[Hook](#asynchrone-regeln-in-requests) weiter unten.

> **Warnung:** Eine häufige Falle ist es, `card_number ?: RequiredIf {...} => with ctx;`
> zu schreiben. In einer `?:`-Zeile überspringt `None` alle Regeln, sodass
> `RequiredIf` ein fehlendes Feld nie durchfallen lassen kann. Verwenden Sie
> `?=>` für jede Regel, die bei Abwesenheit feuern muss.

## Feldübergreifende Hooks

`FormRequest` führt nach den abgeleiteten Regeln pro Feld zwei
feldübergreifende Hooks aus, sowohl im normalen als auch im
Precognition-Flow. `extract()` durchläuft die Stufen der Reihe nach -
abgeleitetes `validate()`, dann `after_validation`, dann
`after_validation_async` - und **steigt bei der ersten fehlschlagenden
Stufe aus**.

```rust
use suprnova::{FormRequest, ValidationErrors};
use serde::Deserialize;
use validator::Validate;

#[derive(Deserialize, Validate)]
pub struct UpdatePassword {
    #[validate(length(min = 8))]
    pub new_password: String,
    pub confirmation: String,
}

impl FormRequest for UpdatePassword {
    fn after_validation(&self) -> Result<(), ValidationErrors> {
        let mut errs = ValidationErrors::new();
        if self.new_password != self.confirmation {
            errs.add("confirmation", "passwords do not match");
        }
        errs.into_result()
    }
}
```

> **Hinweis:** Überschriebene Hooks brauchen ein von Hand geschriebenes
> `impl FormRequest` - das `#[request]`-Attribut und
> `#[derive(FormRequest)]` geben ihre eigene (leere) Impl aus und sind damit
> nur für den häufigen Fall ohne Überschreibung gedacht.

### Asynchrone Regeln in Requests

Das `validate!`-Makro kann kein `.await` einweben, deshalb laufen
datenbankgestützte Regeln in `after_validation_async` - der letzten
Validierungsstufe, die `extract()` automatisch aufruft. Hier nehmen
[`Unique`](#die-unique-regel) und jede eigene `AsyncRule` an der
automatischen Request-Validierung teil; es ist keine Verdrahtung pro
Handler nötig.

```rust
use suprnova::{FormRequest, ValidationErrors, Unique, async_trait};
use serde::Deserialize;
use validator::Validate;

#[derive(Deserialize, Validate)]
pub struct CreateUser {
    #[validate(email)]
    pub email: String,
}

#[async_trait]
impl FormRequest for CreateUser {
    async fn after_validation_async(&self) -> Result<(), ValidationErrors> {
        let mut errs = ValidationErrors::new();
        Unique::new("users", "email")
            .check_async(&self.email, &mut errs, "email")
            .await;
        errs.into_result()
    }
}
```

Weil die asynchrone Stufe erst läuft, wenn die synchronen Stufen
bestanden sind, erreicht ein fehlerhafter Wert (eine syntaktisch
ungültige E-Mail-Adresse) nie die `Unique`-Abfrage in der Datenbank.

## Die `Unique`-Regel

`Unique` prüft, dass ein Wert in einer Tabelle noch nicht existiert.
Bauen Sie sie mit `Unique::new(table, column)` und verfeinern Sie sie
über die Fluent-API:

```rust
use suprnova::Unique;

// email muss eindeutig sein, die gerade bearbeitete Zeile ausgenommen
Unique::new("users", "email").ignore(current_user_id)

// email eindeutig *pro Tenant*, ohne Unterscheidung von Groß- und Kleinschreibung
Unique::new("users", "email")
    .where_eq("tenant_id", tenant_id)
    .case_insensitive()
```

| Builder-Methode | Wirkung |
|----------------|--------|
| `.ignore(id)` | schließt die Zeile aus, deren `id` gleich `id` ist (Fall: sich selbst bearbeiten) |
| `.ignore_with_column(col, id)` | schließt über eine Schlüsselspalte aus, die nicht `id` ist |
| `.where_eq(col, value)` | beschränkt die Prüfung auf Zeilen mit `col = value`; mehrere Aufrufe werden mit AND verknüpft |
| `.case_insensitive()` | vergleicht mit `LOWER(col) = LOWER(?)` |

Tabelle, Spalte, der Ausschlussschlüssel und jede `where_eq`-Spalte
werden gegen eine Allowlist erlaubter Bezeichner geprüft, bevor sie den
SQL-String erreichen; der geprüfte Wert und alle Werte der Einschränkung
sind gebundene Parameter.

### `Unique` ist unverbindlich - die Datenbank-Constraint ist die Garantie

`Unique` führt ein `SELECT COUNT(*)` **vor** dem Schreiben aus und trägt
damit unvermeidlich ein Race zwischen Prüfzeitpunkt und
Nutzungszeitpunkt in sich: Zwei gleichzeitige Anfragen können beide die
Prüfung bestehen und danach beide einfügen. Laravels `unique`-Regel hat
exakt dieselbe Eigenschaft. Die **einzige** echte Garantie ist eine
`UNIQUE`-Constraint (oder ein eindeutiger Index) auf der Spalte in Ihrer
Migration.

Verwenden Sie alle drei zusammen:

1. **Die unverbindliche Regel** - eine schnelle, freundliche Meldung
   "diese E-Mail-Adresse ist schon vergeben" vor dem Absenden (und damit
   Precognition das Feld validieren kann).
2. **Die `UNIQUE`-Constraint** - der maßgebliche Schutz gegen das Race.
3. **`FrameworkError::from_unique_violation`** - an der Schreibstelle:
   Die Constraint-Verletzung, die der Verlierer eines Races bekommt,
   wird auf dasselbe saubere 422 abgebildet, statt ein 500 nach außen
   dringen zu lassen:

```rust
use suprnova::FrameworkError;

// `users.email` trägt in der Migration eine UNIQUE-Constraint.
let user = new_user
    .insert(db)
    .await
    .map_err(|e| FrameworkError::from_unique_violation(
        "email",
        "That email address is already registered.",
        e,
    ))?;
```

`from_unique_violation` liefert einen 422-`Validation`-Fehler, wenn der
Datenbankfehler die Verletzung einer Unique-Constraint ist, und reicht
jeden anderen Fehler unverändert durch (MySQL, Postgres und SQLite
werden alle erkannt).

## Asynchrone Autorisierung

`FormRequest::authorize(&Request) -> bool` läuft **vor** dem Parsen des
Bodys und kann unautorisierte Anfragen daher abweisen, ohne den Payload
zu lesen. Das ist bewusst synchron: Zu diesem Zeitpunkt hält die Anfrage
noch den streamenden Body, der Hook kann also kein `.await` verwenden.
Autorisierung, die die Datenbank oder eine asynchrone Policy braucht,
gehört an eine dieser Stellen und nicht in `authorize`:

- **Middleware** - läuft vor `extract()`, ist `async` und schließt kurz,
  indem sie `Err(response)` zurückgibt (siehe
  [Middleware](middleware.md)). Der richtige Ort für "darf dieser
  Benutzer diese Route überhaupt erreichen".
- **Das Gate** - rufen Sie `Gate::allows_async` /
  `Gate::authorize_async` im Handler auf, sobald Sie den
  authentifizierten Benutzer und die Ressource haben (siehe
  [Autorisierung](authorization.md)).
- **`after_validation_async`** - für eine Autorisierungsprüfung, die vom
  geparsten Request-Body abhängt: Führen Sie sie im asynchronen Hook
  neben Ihren übrigen asynchronen Regeln aus.

## Inertia-Formularübermittlungen

Ein Validierungsfehler beantwortet zwei Zielgruppen unterschiedlich. Ein
REST-Client erhält den `422` mit `{ message, errors }`. Ein Inertia-Besuch
erhält einen `303` zurück zur Formularseite, wobei die Fehler in die
Session geflasht werden, weil der Inertia-Client für jede Response, die
er nicht als Inertia-Response erkennt, ein Fehler-Modal anzeigt - ein
`422` würde `form.errors` nie befüllen.

Im Handler ändert sich nichts. Auf der Zielseite trägt jedes Feld seine
erste Meldung als String:

```svelte
{#if errors?.email}
  <p class="text-red-600">{errors.email}</p>
{/if}
```

Siehe [Inertia-Responses](frontend-inertia-responses.md#validation-failures)
für Error-Bags, `with_all_errors` und das Ziel des Redirects.

## Anmerkungen zum Design

- **Teilweise Validierung.** Ein `FormRequest` deserialisiert vor der
  Validierung in eine typisierte Struktur, die Struktur *ist* also das
  Schema: Ein Feld, das fehlen darf, muss `Option<T>` sein. Genau das
  erlaubt es Precognition auch, einen unvollständigen Payload zu
  validieren - machen Sie die Felder optional, die ein Entwurf weglassen
  darf.
- **Regelmeldungen.** Eingebaute Regeln liefern Meldungen mit Schlüssel
  (`validation-min` plus seine Argumente und ein englischer Fallback),
  die an der Serialisierungsgrenze über den Katalog aufgelöst werden.
  Übersetzen oder formulieren Sie jede davon um, indem Sie dieselbe ID
  in `lang/<locale>/validation.ftl` definieren - kein Wrappen von
  Regeln. Siehe [Lokalisierung](localization.md).
- **`Min` / `Max` / `Between`** sind Regeln für die String-Länge
  (gezählt in Unicode-Skalarwerten). Für numerische Grenzen validieren
  Sie mit `#[validate(range(...))]` am Derive oder mit einer eigenen
  Regel - die Längenregeln sind keine Wertvergleiche.

## Zusammenfassung

| Aufgabe | API |
|------|-----|
| Regeln pro Feld | `#[validate(...)]` auf dem `FormRequest` (siehe Anfragen) |
| Komponierte / feldübergreifende Regeln | `validate! { self => ... }` |
| JSON-geformte Regel (Array/Objekt) | `field => ArrayKeys(&[...]);` / `field => Distinct { .. };` |
| Optional, "falls vorhanden" | `field ?: Rule;` |
| Bedingt erforderliches optionales Feld | `field ?=> Rule => with ctx;` |
| Asynchrone / datenbankgestützte Regel | `after_validation_async` + `AsyncRule::check_async` |
| Eindeutigkeit | `Unique::new(t, c)` + `UNIQUE`-Constraint + `from_unique_violation` |
| Asynchrone Autorisierung | Middleware / `Gate::*_async` / `after_validation_async` |

## Nächste Schritte

- [Anfragen](requests.md) - die Oberfläche aus `#[request]` /
  `#[derive(FormRequest)]`, der alltägliche Weg der abgeleiteten
  Validierung
- [Datenobjekte](data.md) - `#[derive(Data, Validate)]` für eine
  Struktur, die zugleich eingehender Request und ausgehendes DTO ist
- [Fehlermodell](error-model.md) - wie aus `ValidationErrors` der
  422-JSON-Body wird, neben jedem anderen Fehlerpfad
- [Lokalisierung](localization.md) - Regelmeldungen übersetzen, die
  `field-<name>`-Konvention und `ValidationMessage`s mit Schlüssel
- [Autorisierung](authorization.md) - `Gate`, `Policy` und wo die
  Autorisierung im Verhältnis zur Validierung hingehört
- [Middleware](middleware.md) - der richtige Ort für Prüfungen der Art
  "darf diese Anfrage überhaupt durch", die `.await` brauchen
