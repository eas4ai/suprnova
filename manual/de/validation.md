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

Eine Regel ist ein Wert, der einen von drei Traits implementiert:

| Trait | Form | Einsatz |
|-------|-------|-----|
| `Rule` | `passes(&self, value: &str)` | reine Prüfung eines einzelnen Werts |
| `ContextualRule` | `passes(&self, value, ctx)` | Prüfung, die Nachbarfelder liest |
| `AsyncRule` | `async passes(&self, value)` | Prüfung mit `.await` (DB, HTTP) |

Eingebaute `Rule`s: `Required`, `Email`, `Min`, `Max`, `Between`, `In`,
`NotIn`, `Integer`, `Numeric`, `Boolean`, `Alpha`, `AlphaNum`, `Url`,
`HttpUrl`, `Uuid`. Eingebaute `ContextualRule`s: `RequiredIf`,
`RequiredWith`, `RequiredUnless`, `Same`, `Different`, `Confirmed`.
Eingebaute `AsyncRule`: [`Unique`](#die-unique-regel).

```rust
use suprnova::{Rule, rules::Email};

Email.passes("user@example.com")?; // Ok(())
```

> **Hinweis:** `Numeric` akzeptiert eine **endliche** Zahl - `NaN`, `inf` und
> Größenordnungen, die zu Unendlich überlaufen, werden abgewiesen, obwohl
> Rusts Parser die Strings akzeptieren würde. Verwenden Sie `HttpUrl` (nicht
> `Url`) für Callback-, Webhook- und Avatar-Eingaben: `Url` parst jedes
> Schema, das `url::Url` akzeptiert (`file:`, `javascript:`, eigene URIs),
> während `HttpUrl` `http`/`https` verlangt.

### Eine eigene Regel schreiben

Eine eigene Regel ist eine Unit-Struktur (oder eine, die Daten trägt)
mit genau einer Impl. Der Trait schenkt Ihnen `check()` dazu; es legt
jede Fehlermeldung unter dem benannten Feld auf einer
`ValidationErrors`-Bag ab. So fügt sich die Regel unverändert in
`validate!` und die `after_validation`-Hooks ein:

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
gerendert wird - mehr braucht eine einsprachige Anwendung nicht. Damit
die Meldung pro Locale übersetzt wird, geben Sie stattdessen eine
Meldung *mit Schlüssel* zurück -
`ValidationMessage::keyed("validation-starts-with").arg("prefix", self.0).fallback(…)` -
und definieren die ID in `lang/<locale>/validation.ftl`. Siehe
[Lokalisierung](localization.md); dort steht auch, wie Sie die Meldungen
der eingebauten Regeln überschreiben, und die Namenskonvention
`field-<name>`.

Für feldübergreifende Logik implementieren Sie stattdessen
[`ContextualRule`] - die `passes`-Methode bekommt neben dem geprüften
Wert ein `&FormContext` (eine `HashMap<String, String>` mit den Werten
der Nachbarfelder). Für datenbankgestützte Prüfungen implementieren Sie
[`AsyncRule`] und verwenden es aus `after_validation_async`.

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
  von der Regel erwartete Referenz dereferenzieren lässt).
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
