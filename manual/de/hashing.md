# Hashing

Das Modul `suprnova::hashing` ist die Passwort-Hashing-Oberfläche
des Frameworks, mit drei erstklassigen Treibern - **bcrypt**
(Standard, kompatibel zu Laravel), **Argon2i** (speicherintensiv,
seitenkanalresistent) und **Argon2id** (OWASP-2024-Empfehlung).
Verwenden Sie es beim Speichern von Benutzerpasswörtern, beim
Hashen von Remember-me-Verifier-Tokens oder überall dort, wo eine
Einwegfunktion die richtige Primitive ist. Die Treiberauswahl
erfolgt über die Umgebung, und die Facade ist durchgängig
algorithmusbewusst (`info`, `is_hashed`, `needs_rehash`, `verify`),
sodass ein gespeicherter bcrypt-Hash auch nach dem Umschalten auf
`HASH_DRIVER=argon2id` weiterhin verifiziert.

## Überblick

```rust
use suprnova::hashing;

// Async (bevorzugt innerhalb von Tokio-Request-Handlern - führt den
// CPU-gebundenen Hash über spawn_blocking aus, damit der
// Worker-Thread frei bleibt):
let hashed = hashing::hash_async("my_password").await?;
let valid = hashing::verify_async("my_password", &hashed).await?;

// Sync (Tests, CLI-Tools, nicht-asynchrone Kontexte):
let hashed = hashing::hash("my_password")?;
let valid = hashing::verify("my_password", &hashed)?;
```

Die freistehende Funktions-Facade liest den aktiven Treiber aus
`HASH_DRIVER` (oder fällt auf bcrypt zurück). Für Aufrufe mit
explizitem Treiber konstruieren Sie den Treiber-Typ direkt und
übergeben ihn an `hash_with` / `verify_with` / `needs_rehash_with`.

## Konfiguration

| Variable | Beschreibung | Standard | Bereich |
|----------|-------------|---------|-------|
| `HASH_DRIVER` | Aktiver Algorithmus | `bcrypt` | `bcrypt` \| `argon` \| `argon2i` \| `argon2id` |
| `HASH_ROUNDS` | Bcrypt-Kostenfaktor | `12` | `4..=31` (nur bcrypt) |
| `HASH_MEMORY` | Argon-Speicherkosten in KiB | `65536` (64 MiB) | `>= 8` (nur argon) |
| `HASH_TIME` | Argon-Zeit-Iterationen | `4` | `>= 1` (nur argon) |
| `HASH_THREADS` | Argon-Parallelität / Lanes | `1` | `>= 1` (nur argon) |
| `HASH_VERIFY` | Bei `true` lehnt `verify()` algorithmusübergreifende Hashes ab | `false` | `true` / `false` |

Eine Fehlkonfiguration (ungültiger Wert, Parameter außerhalb des
Bereichs) tritt beim ersten Aufruf von `hash` / `verify` /
`needs_rehash` als `FrameworkError::param` zutage - nicht als
stiller Standardwert.

### Beispiel-`.env` für argon2id

```env
HASH_DRIVER=argon2id
HASH_MEMORY=65536
HASH_TIME=4
HASH_THREADS=1
```

### Warum Suprnovas Argon2-Standardwerte stärker sind als die von Laravel

| Parameter | Laravel-Standard | Suprnova-Standard | Quelle |
|-------|-----------------|------------------|--------|
| Speicher | 1 024 KiB (1 MiB) | 65 536 KiB (64 MiB) | OWASP 2024 |
| Zeit | 2 Iterationen | 4 Iterationen | OWASP 2024 |
| Threads | 2 | 1 | OWASP 2024 / an libsodium ausgerichtet |

Laravels Standardwerte setzen PHPs Modell eines Prozesses pro
Anfrage voraus - ein Worker kann für jeden Passwort-Hash nur so
viel Zeit aufwenden, bevor die Maschine überlastet ist. Tokios
`spawn_blocking` lässt Suprnova den Hash an einen blockierenden
Thread-Pool abgeben, ohne die Anfrage-Schleife zu blockieren,
sodass die OWASP-2024-Werte auf echter Produktionshardware
realistisch sind.

## Treiber

### Bcrypt (Standard)

```rust
use suprnova::hashing::{BcryptHasher, BcryptOptions, hash_with, verify_with};

let driver = BcryptHasher::new(BcryptOptions { rounds: 14 });
let hashed = hash_with(&driver, "my_password")?;
assert!(verify_with(&driver, "my_password", &hashed)?);
```

Bcrypt hat eine **Obergrenze von 72 Byte** für die
Passwort-Eingabe - die zugrunde liegende Primitive kürzt längere
Eingaben stillschweigend, was bedeutet, dass zwei verschiedene
Passphrasen mit denselben ersten 72 Byte auf denselben Wert hashen.
Suprnova lehnt solche Eingaben von vornherein ab (der bcrypt-Pfad
des Frameworks liefert bei `hash()` einen Fehler und bei `verify()`
für überlange Passwörter `Ok(false)`, damit die "ungültige
Credentials"-Antwort des Auth-Flows einheitlich bleibt). Argon2
kennt keine solche Obergrenze.

Die bcrypt-Obergrenze ist als
`suprnova::hashing::MAX_BCRYPT_PASSWORD_BYTES` zugänglich (71 - die
nutzbare Grenze nach dem bcrypt-Nullterminator).

### Argon2id (OWASP-2024-Empfehlung)

```rust
use suprnova::hashing::{Argon2idHasher, Argon2Options, hash_with, verify_with};

let driver = Argon2idHasher::new(Argon2Options {
    memory: 65_536,  // 64 MiB
    time: 4,
    threads: 1,
})?;

let hashed = hash_with(&driver, "my_password")?;
assert!(verify_with(&driver, "my_password", &hashed)?);

// Argon2 akzeptiert Passphrasen beliebiger Länge - die
// 72-Byte-Obergrenze von bcrypt gilt hier nicht.
let long = "x".repeat(500);
let h = hash_with(&driver, &long)?;
assert!(verify_with(&driver, &long, &h)?);
```

### Argon2i

Gleiche Form wie Argon2id; `Argon2iHasher::new(opts)`. Verwenden Sie
für neue Projekte Argon2id - Argon2i wird aus
Kompatibilitätsgründen unterstützt, aber Argon2id ist die moderne
Empfehlung.

## Bcrypt mit explizitem Kostenfaktor (`hash_with_cost`)

`hash_with_cost(password, cost)` und
`hash_with_cost_async(password, cost)` prägen einen bcrypt-Hash mit
einem vom Aufrufer vorgegebenen Kostenfaktor, unabhängig von
`HASH_DRIVER`. Verwenden Sie diese, wenn eine Richtlinie oder eine
mandantenspezifische Konfiguration einen Kostenfaktor direkt an die
Aufrufstelle statt in die Prozessumgebung einspeist - zum Beispiel
eine Hochsicherheits-Kontoklasse, die Kostenfaktor 14 verwendet,
während der Rest der App mit dem Standard 12 läuft.

```rust
use suprnova::hashing::{hash_with_cost, hash_with_cost_async};

// Sync - Tests, CLI-Tools.
let h = hash_with_cost("my_password", 14)?;

// Async - innerhalb von Tokio-Request-Handlern.
let h = hash_with_cost_async("my_password", 14).await?;
```

Beide Einstiegspunkte lehnen `cost` außerhalb von
`MIN_BCRYPT_COST..=MAX_BCRYPT_COST` (`4..=31`) mit
`FrameworkError::param` ab und spiegeln damit die
umgebungsseitige `HASH_ROUNDS`-Validierung:

```rust
use suprnova::hashing::{hash_with_cost, MIN_BCRYPT_COST, MAX_BCRYPT_COST};

assert!(hash_with_cost("pw", MIN_BCRYPT_COST - 1).is_err()); // < 4
assert!(hash_with_cost("pw", MAX_BCRYPT_COST + 1).is_err()); // > 31
```

Die Bereichsprüfung ist wichtig, weil jede Erhöhung des
Kostenfaktors die CPU-Zeit verdoppelt. Bei Kostenfaktor 31 dauert
ein einzelner bcrypt-Hash auf üblicher Hardware Stunden - die
Bereichsprüfung im Framework verhindert, dass ein Tippfehler in
einer Richtlinie oder Konfiguration einen Worker-Thread versehentlich
für den Rest des Tages blockiert. Die asynchrone Variante läuft
über `spawn_blocking`, sodass selbst ein berechtigt hoher
Kostenfaktor die Anfrage-Schleife nicht blockiert.

## Algorithmusbewusstes needs_rehash

`needs_rehash` liefert `true`, wenn der gespeicherte Hash unter dem
aktiven Treiber neu gehasht werden sollte. Es deckt drei Fälle ab:

1. **Algorithmus-Diskrepanz** - ein bcrypt-Hash ist gespeichert,
   während `HASH_DRIVER=argon2id` gilt (oder umgekehrt). Löst bei
   der nächsten erfolgreichen Verifizierung eine Rotation aus.
2. **Parameter-Schwäche** - bcrypt-Kostenfaktor unter
   `HASH_ROUNDS`, oder argon `m`/`t`/`p` unter
   `HASH_MEMORY`/`HASH_TIME`/`HASH_THREADS`.
3. **Bcrypt-Legacy-Varianten** - `$2a$`, `$2x$`, `$2y$` rotieren
   auch beim konfigurierten Kostenfaktor zum kanonischen `$2b$`.

```rust
if hashing::needs_rehash(&stored_hash) {
    let fresh = hashing::hash_async("plaintext_at_login").await?;
    // `fresh` persistieren. Das Standard-Laravel-Muster "erneut
    // hashen nach erfolgreichem Login"; funktioniert über
    // Algorithmen hinweg.
}
```

Fehlerhafte Eingaben liefern `true` - der Aufrufer rotiert auf
natürliche Weise alles, was er nicht parsen kann.

## Hash-Inspektion (`info` + `is_hashed`)

```rust
use suprnova::hashing::{info, is_hashed};

let h = hashing::hash_async("my_password").await?;
let i = info(&h);
println!("algo: {}", i.algo.as_str());
println!("bcrypt cost: {:?}", i.rounds);
println!("argon memory KiB: {:?}", i.memory);

// Gilt für jeden erkannten Algorithmus-Hash; false für Klartext /
// Müll.
assert!(is_hashed(&h));
assert!(!is_hashed("plaintext"));
```

`info().algo` ist eines von: `Bcrypt`, `Argon2i`, `Argon2id`,
`Argon2d` (erkannt, aber nie geprägt), `Unknown`.

`is_hashed` ist das, was der eloquente Cast `AsHashed` verwendet,
um das erneute Hashen einer bereits gehashten Spalte zu
überspringen - funktioniert über alle drei Treiber hinweg, sodass
ein Wechsel von `HASH_DRIVER` mitten im Projekt beim nächsten
Speichern keine Hash-des-Hashes-Schleife verursacht.

## Algorithmusübergreifendes Verifizierungs-Gate (`HASH_VERIFY`)

Standardmäßig prüft `verify()` das Passwort gegen den Hash,
unabhängig davon, welcher Algorithmus den Hash erzeugt hat - das
ist es, was Legacy-bcrypt-Hashes auch nach dem Umschalten auf
`HASH_DRIVER=argon2id` weiter verifizieren lässt (sodass Sie sie
beim Login rotieren können). Setzen Sie `HASH_VERIFY=true`, sobald
jeder Benutzer rotiert ist, um den aktiven Algorithmus strikt
durchzusetzen:

```env
HASH_VERIFY=true
```

Mit aktiviertem Gate liefert `verify()` für jeden Hash, dessen
Algorithmus vom aktiven Treiber abweicht, `Ok(false)` - dieselbe
Form wie Laravels `RuntimeException`, aber Suprnova liefert false
zurück statt zu werfen, weil der Aufrufer im Auth-Flow ohnehin ein
`Result<bool>` erwartet.

## Async vs. Sync

Sowohl bcrypt mit Kostenfaktor 12 (~250 ms) als auch Argon2id mit
memory=64 MiB (~80 ms) sind absichtlich CPU-gebunden - das ist der
ganze Sinn langsamen Hashings. Der direkte Aufruf des synchronen
`hash` / `verify` aus einem Tokio-Request-Handler blockiert den
Worker-Thread für die gesamte Hash-Dauer und hungert dabei andere
Anfragen auf demselben Worker aus.

Verwenden Sie die `*_async`-Geschwister innerhalb von
`async fn`-Handlern. Sie umschließen den CPU-gebundenen Aufruf mit
`tokio::task::spawn_blocking`, sodass der Worker für andere
Anfragen frei bleibt:

```rust
// GUT - innerhalb eines asynchronen Handlers
let hashed = hashing::hash_async(&form.password).await?;

// SCHLECHT - blockiert den Worker für ~250 ms
let hashed = hashing::hash(&form.password)?;
```

Die synchronen Varianten sind für Tests, CLI-Tools und andere
nicht-asynchrone Kontexte gedacht, in denen Blockieren
unproblematisch ist.

## Eloquent-Integration: der Cast `AsHashed`

Der eloquente Cast `#[cast(AsHashed)]` hasht ein Klartext-Feld beim
Schreiben mit dem aktiven Treiber und ist **über alle Treiber
hinweg idempotent** - das Speichern eines Modells, dessen
`password`-Spalte bereits einen erkannten Hash (bcrypt oder argon)
enthält, lässt den Wert unverändert durchlaufen. Ohne diese
Absicherung würde `User::find(id).await?.save().await?` bei jedem
Speichern den bereits vorhandenen Hash erneut hashen und die
Authentifizierung brechen.

```rust
use suprnova::eloquent::casts::AsHashed;

#[suprnova::model]
struct User {
    #[cast(AsHashed)]
    pub password: String,
    // ...
}
```

Die Idempotenzprüfung verwendet `hashing::is_hashed`, sodass ein
Wechsel von `HASH_DRIVER` mitten im Projekt sicher ist - sowohl die
Legacy-bcrypt-Hashes als auch die frischen argon2id-Hashes werden
erkannt und beim erneuten Speichern übersprungen.

## Verwendung mit `Auth::attempt`

`Auth::attempt(&credentials)` ruft
`UserProvider::validate_credentials` auf, was wiederum
`hashing::verify_async` gegen den gespeicherten Hash des Benutzers
aufruft. Die Verifizierung richtet sich nach dem Algorithmus des
*gespeicherten* Hashes, nicht nach dem konfigurierten Treiber - nach
dem Umschalten auf `HASH_DRIVER=argon2id` verifiziert also jeder
bestehende bcrypt-Hash weiterhin, und `needs_rehash` liefert
`true`, sodass das übliche Rotate-on-Login-Muster die Nutzerbasis
Login für Login zum neuen Algorithmus überführt.

## Den Treiber in Tests überschreiben

`set_default_driver(Box<dyn Hasher>)` installiert einen Treiber
programmatisch für Tests und eingebettete CLI-Tools, die den
Treiber bauen, ohne über `HASH_DRIVER` zu gehen. Es ist einmalig -
der erste Aufruf gewinnt, und ein zweiter Aufruf liefert
`FrameworkError::internal`, statt den Treiber mitten im Prozess
auszutauschen. Verwenden Sie es beim Start der Testsuite, bevor
irgendein Codepfad den Standard auflöst.

## Nächste Schritte

- [Authentifizierung](authentication.md) - `Auth::attempt`, der
  User-Provider-Trait und wie Hashing in den Login integriert
- [Auth-Flows](auth-flows.md) - `PasswordReset::complete` rotiert
  den gespeicherten Passwort-Hash durch den aktiven Treiber;
  Remember-me-Tokens werden vor der Speicherung über `hash_async`
  gehasht
- [Eloquent](eloquent.md) - Referenz zu `#[cast(AsHashed)]` und die
  breitere Cast-Oberfläche
- [Verschlüsselung](encryption.md) - zweiseitige authentifizierte
  Verschlüsselung für ruhende Daten; die Ergänzung zum einseitigen
  Hashing
- [Fehlermodell](error-model.md) - wie `FrameworkError::param`
  aussieht, wenn ein Hashing-Konfigurationswert abgelehnt wird
