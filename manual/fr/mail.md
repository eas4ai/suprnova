# E-mail

Le sous-système mail de Suprnova reflète l'API
`Mail::to(...)->send(...)` de Laravel sur Tokio. Une façade `Mail`,
huit transports (log et en mémoire pour le dev/les tests, SMTP, et
cinq fournisseurs HTTP - Postmark, SES, SendGrid, Mailgun, Resend),
des templates rendus par Tera avec les champs sérialisés du Mailable
comme contexte, mise en file d'attente + livraison différée sur
l'enveloppe durable au moins une fois, et une garde de test
`Mail::fake()` taillée dans le même tissu que `Bus::fake()` et
`Cache::fake()`.

## Démarrage rapide

```rust
use serde::{Deserialize, Serialize};
use suprnova::async_trait;
use suprnova::mail::{Address, Mail, Mailable};

#[derive(Serialize, Deserialize)]
struct Welcome {
    name: String,
}

#[async_trait]
impl Mailable for Welcome {
    fn mailable_name() -> &'static str { "Welcome" }
    fn subject(&self) -> String { format!("Welcome, {}", self.name) }
    fn text_template_source(&self) -> Option<String> {
        Some("Hi {{ name }}, welcome aboard.".into())
    }
    fn from(&self) -> Option<Address> {
        Some(Address::new("hello@example.com").with_name("Suprnova"))
    }
}

async fn greet(name: String) -> Result<(), suprnova::FrameworkError> {
    Mail::to("alice@example.org")
        .send(Welcome { name })
        .await
}
```

Le Mailable se sérialise en JSON, qui devient le contexte Tera pour
le template ; chaque champ `pub` est accessible comme `{{ field_name
}}`.

## Configuration

`Server::serve` appelle `suprnova::mail::boot::bootstrap_from_env()`
une fois au démarrage. Elle lit `MAIL_DRIVER` et lie le transport
correspondant. Retombe sur le driver `log` quand non défini.

| `MAIL_DRIVER` | Comportement |
|---------------|--------------|
| `log`         | Émet un `tracing::info!` par envoi - enveloppe et corps complets, comme le fait Laravel - puis abandonne. Défaut hors production. |
| `memory`      | Capture chaque message dans le process. Voir `suprnova::mail::boot::captured_in_memory()`. |
| `smtp`        | Se connecte à un serveur SMTP (STARTTLS quand des identifiants sont définis, TCP simple sinon). |
| `postmark`    | POST du JSON vers le endpoint `/email` de Postmark. |
| `ses`         | POST des requêtes signées SigV4 vers `SendEmail` d'Amazon SES. |
| `sendgrid`    | POST du JSON vers `/v3/mail/send` de SendGrid. |
| `mailgun`     | POST en `application/x-www-form-urlencoded` (ou `multipart/form-data` en présence de pièces jointes) vers `/v3/{domain}/messages` de Mailgun. |
| `resend`      | POST du JSON vers `/emails` de Resend. |

### La production échoue fermée sur un driver qui abandonne le mail

`log` et `memory` rendent un message et l'abandonnent. Sous
`APP_ENV=production`, l'amorçage **refuse** de démarrer sur l'un ou
l'autre - et de même sur un `MAIL_DRIVER` non défini ou une valeur
que le build ne reconnaît pas, parce que les deux atterrissent sur ce
même transport `log` :

```
refusing to boot in production: MAIL_DRIVER is unset, which defaults to the `log`
transport. Password resets and email verifications would report success while
nothing is delivered. Set MAIL_DRIVER to a delivering driver (smtp | postmark |
ses | sendgrid | mailgun | resend), or set
MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true to acknowledge that outgoing mail is
intentionally discarded.
```

L'échec que ceci prévient est silencieux : avec l'ancien défaut, un
déploiement qui avait oublié `MAIL_DRIVER` - ou écrit
`MAIL_DRIVER=SMTP` dans la mauvaise casse - rapportait chaque
réinitialisation de mot de passe comme envoyée alors que rien ne
quittait jamais le process, et personne ne s'en apercevait jusqu'à ce
qu'un utilisateur se retrouve verrouillé hors de son compte.

Si un déploiement de production veut vraiment n'envoyer aucun mail
sortant (un miroir en lecture seule, un dark launch), reconnaissez-le
explicitement :

```env
MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true
```

Seuls `1`, `true`, `yes`, ou `on` comptent comme un consentement -
`=false` ou une faute de frappe laisse le garde-fou armé. Avec la
dérogation définie, chaque amorçage avertit que le mail sortant ne
sera pas livré.

Rien ne change hors production : `local`, `development`, `testing`,
et `staging` gardent le défaut `log` et gardent le comportement
avertir-puis-retomber pour les drivers inconnus.

### La production échoue fermée sur une connexion SMTP non chiffrée

La même règle, appliquée à la façon dont la connexion est protégée
plutôt qu'à si elle livre. `MAIL_DRIVER=smtp` en production doit se
résoudre vers un transport chiffré, sinon l'amorçage échoue.

`MAIL_SMTP_ENCRYPTION` prend `starttls`, `tls`, ou `none` (`ssl` et
`null` sont acceptés comme alias compatibles Laravel). Laissé non
défini, elle se dérive des identifiants :

| `MAIL_SMTP_USER` / `MAIL_SMTP_PASS` | Se résout en | Parce que |
|---|---|---|
| les deux définis | `starttls` | Des identifiants impliquent un vrai relais sur le port de soumission. |
| ni l'un ni l'autre | `none` | Le chemin catcher local. Mailpit, MailHog et maildev écoutent sans authentification sur 1025 et ne parlent aucun TLS. |

Ainsi un scaffold neuf continue de fonctionner avec zéro
configuration, et un déploiement de production qui n'a jamais câblé
les identifiants s'arrête au lieu d'envoyer silencieusement en
clair. Définissez `MAIL_SMTP_ENCRYPTION=tls` pour un relais qui
attend du TLS implicite sur 465 - un mode que le transport a toujours
supporté mais qu'aucune combinaison de variables d'environnement ne
pouvait atteindre avant.

Une valeur non reconnue fait échouer l'amorçage dans *tout*
environnement, pas seulement en production.
`MAIL_SMTP_ENCRYPTION=tsl` est une transposition d'un mode qui
chiffre, donc le traiter silencieusement comme « pas de chiffrement »
serait exactement l'échec que la variable existe pour prévenir -
mieux vaut échouer sur la machine du développeur que dans le
déploiement.

L'échappatoire reflète celle du dessus :

```env
MAIL_ALLOW_INSECURE_SMTP_IN_PRODUCTION=true
```

Défendable seulement quand le relais n'est atteignable que sur un
réseau privé - un sidecar, ou un Postfix à l'intérieur du VPC. Sur
tout le reste, le SMTP en texte en clair met les identifiants et
chaque lien de réinitialisation de mot de passe sur le réseau, et ça
y reste pour quiconque écoute sur le chemin.

### Le driver `log` journalise le message complet

Comme le mailer `log` de Laravel : enveloppe *et* corps rendus.

```
mail (log driver): would send from=noreply@app.test to=["alice@example.org"]
  subject=Reset your password
  text=Reset your password: https://app.test/password/reset?token=9f3a…&signature=…
  html=<a href="https://app.test/password/reset?token=9f3a…&signature=…">Reset</a>
```

Ce lien est tout l'intérêt. En développement, la console est
l'endroit où vous lisez le lien de vérification ou de
réinitialisation de mot de passe que l'app vient « d'envoyer », et un
driver qui le cache est un driver que personne ne peut utiliser.

C'est sûr ici parce que le driver ne peut pas atteindre la
production - l'amorçage refuse de démarrer sur `MAIL_DRIVER=log` sous
`APP_ENV=production` (voir ci-dessus). Les corps n'existent jamais
que sur la machine d'un développeur.

Si vous définissez `MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true`
pour faire tourner le driver `log` dans un environnement déployé,
vous choisissez de mettre des liens bearer à usage unique dans vos
logs. Quiconque peut lire ces fichiers - les opérateurs, le log
shipper, le bucket de rétention, l'agrégateur - peut les utiliser, et
l'expiration du lien n'aide pas parce que le log shipping est plus
rapide qu'une personne qui lit sa boîte de réception. Dimensionnez
votre politique de rétention et d'accès en conséquence, ou utilisez
un driver qui n'imprime pas :

```env
# Capture dans le process - suprnova::mail::boot::captured_in_memory(), ou Mail::fake() dans les tests
MAIL_DRIVER=memory

# Ou un catcher local (mailpit / maildev / mailhog), qui affiche le vrai mail dans une UI
MAIL_DRIVER=smtp
MAIL_SMTP_HOST=127.0.0.1
MAIL_SMTP_PORT=1025
```

### Environnement par driver

```env
# SMTP
MAIL_DRIVER=smtp
MAIL_SMTP_HOST=smtp.mailtrap.io
MAIL_SMTP_PORT=587
MAIL_SMTP_USER=...
MAIL_SMTP_PASS=...
MAIL_SMTP_ENCRYPTION=starttls   # ou `tls` pour du TLS implicite sur 465, ou `none`

# Postmark
MAIL_DRIVER=postmark
MAIL_POSTMARK_TOKEN=...

# Amazon SES
MAIL_DRIVER=ses
MAIL_SES_ACCESS_KEY=...
MAIL_SES_SECRET_KEY=...
MAIL_SES_REGION=us-east-1

# SendGrid
MAIL_DRIVER=sendgrid
MAIL_SENDGRID_API_KEY=...

# Mailgun
MAIL_DRIVER=mailgun
MAIL_MAILGUN_API_KEY=...
MAIL_MAILGUN_DOMAIN=mg.example.com

# Resend
MAIL_DRIVER=resend
MAIL_RESEND_API_KEY=...
```

Chaque fournisseur HTTP honore aussi une dérogation
`MAIL_<PROVIDER>_ENDPOINT` correspondante qui pointe vers une URL
régionale ou un serveur mock (utile pour les tests d'intégration
contre `wiremock`).

### Expéditeur des flux d'authentification : `MAIL_FROM` et `MAIL_FROM_NAME`

Les mailables de flux d'authentification intégrés - vérification
d'e-mail, réinitialisation de mot de passe, et l'avis de changement
de mot de passe - résolvent leur `From` d'enveloppe depuis
l'environnement plutôt que depuis un `from()` codé en dur :

```env
MAIL_FROM=no-reply@example.com        # adresse nue (exigée par les flux d'auth ; échoue fermé si non défini)
MAIL_FROM_NAME=Acme Support           # nom d'affichage optionnel (depuis la 0.5.9)
```

- `MAIL_FROM` **doit être une adresse nue.** Elle est reprise telle
  quelle dans le `From` du message, si bien qu'une valeur `"Nom
  <adresse>"` serait traitée comme l'adresse entière et rejetée par
  le transport.
- `MAIL_FROM_NAME` (optionnel, ajouté en **0.5.9**) attache un nom
  d'affichage, si bien que l'en-tête se rend en `Acme Support
  <no-reply@example.com>`. Non défini ou vide garde le comportement
  précédent d'adresse nue. Elle est lue au moment de l'envoi, si bien
  qu'elle s'applique aussi au mail de flux d'auth mis en file
  d'attente.

Ces deux variables n'affectent que les mailables de flux d'auth du
framework lui-même. Vos propres `Mailable` fixent leur expéditeur via
`from()` (ou le défaut global `always_from`) - voir plus bas.

## Le trait Mailable

Les mailables sont des structs sérialisables qui savent se rendre
elles-mêmes. Les défauts du trait rendent avec
`tera::Tera::one_off` contre les champs sérialisés du mailable :

```rust
use suprnova::async_trait;
use suprnova::mail::{Address, Attachment, Mailable};

#[async_trait]
impl Mailable for OrderShipped {
    fn mailable_name() -> &'static str { "OrderShipped" }
    fn subject(&self) -> String {
        format!("Order #{} shipped", self.order_id)
    }
    fn html_template_source(&self) -> Option<String> {
        Some("<p>Tracking: <code>{{ tracking }}</code></p>".into())
    }
    fn text_template_source(&self) -> Option<String> {
        Some("Tracking: {{ tracking }}".into())
    }
    fn from(&self) -> Option<Address> {
        Some(Address::new("orders@example.com").with_name("Acme Orders"))
    }
    fn attachments(&self) -> Vec<Attachment> {
        vec![Attachment::new("invoice.pdf", self.invoice_bytes.clone(), "application/pdf")]
    }
}
```

| Méthode | Requise ? | But |
|--------|-----------|-----|
| `mailable_name()` | oui | Nom stable persisté dans l'enveloppe de la file d'attente - le renommer casse le mail en file d'attente en vol. |
| `subject(&self)` | oui | Sujet calculé. Utilisé tel quel quand `subject_template_source` renvoie `None`. |
| `subject_template_source(&self)` | optionnel | Template Tera pour le sujet - quand `Some`, prend le pas sur `subject()` et se rend avec `self` comme contexte. Même sémantique que les sources de template du corps. |
| `html_template_source(&self)` | optionnel | Template Tera du corps HTML. Renvoyer `None` pour sauter le HTML. |
| `text_template_source(&self)` | optionnel | Template Tera du corps texte brut. Renvoyer `None` pour sauter le texte. |
| `from(&self)` | optionnel | Surcharge le défaut global `noreply@localhost`. |
| `attachments(&self)` | optionnel | Fichiers à joindre. Chacun est `name + bytes + mime`. |
| `render_subject(&self)` / `render_html(&self)` / `render_text(&self)` | optionnel | Surchargez si vous voulez contourner Tera (Markdown → HTML, contenu pré-rendu, logique de sujet personnalisée, etc.). |

Au moins l'un de `html_template_source` ou `text_template_source`
doit renvoyer `Some` (ou `render_html`/`render_text` doit produire du
contenu). Un mailable à corps vide est refusé aussi bien au dispatch
(`Mail::send`) qu'à la mise en file (`Mail::queue`).

### Autoescape de Tera

L'autoescape est **DÉSACTIVÉ** parce que les corps de mail sont
typiquement du HTML écrit à la main, où l'échappement `<>&` de Tera
sur-échapperait. Si votre corps littéral contient `{{` pour des
raisons non liées au template (par exemple, un texte marketing qui
cite la syntaxe Mustache), échappez-le : `{% raw %}{{ literal
}}{% endraw %}`.

## Construire des messages

Le builder `Mail::to(...)` fait passer les destinataires, CC/BCC,
reply-to, et une surcharge d'expéditeur par message, dans le
dispatch :

```rust
Mail::to("alice@example.org")
    .cc("manager@example.com")
    .bcc("audit@example.com")
    .reply_to("support@example.com")
    .from(("Operations", "ops@example.com"))   // (nom d'affichage, e-mail)
    .send(OrderShipped { order_id: 42, /* ... */ })
    .await?;
```

`Address` accepte `&str`, `String`, et des tuples `(name, email)` ;
`Mail::to(...)` accepte tout ce qui est `Into<Address>`.

## Pièces jointes

```rust
use suprnova::mail::Attachment;

let attachment = Attachment::new(
    "report.csv",
    csv_bytes,
    "text/csv",
);
```

Les pièces jointes voyagent via la méthode `Mailable::attachments`.
Les cinq fournisseurs HTTP les gèrent - Postmark/SendGrid/Resend en
JSON (encodé en base64), SES via du MIME brut (puisque
`Content.Simple` ne supporte pas les pièces jointes), et Mailgun via
`multipart/form-data` (le chemin form-encodé est utilisé quand il n'y
a pas de pièces jointes).

## Mise en file d'attente

`Mail::queue(...)` construit un `SendMailJob` et le pousse sur la
file d'attente du framework. Le worker reconstruit le mailable depuis
la factory enregistrée et dispatche à travers le transport lié :

```rust
// Une fois : enregistrez chaque type Mailable que le worker verra.
suprnova::mail::register_mailable_factory::<Welcome>()?;

// Au moment de l'envoi :
Mail::to("alice@example.org").queue(Welcome { name: "Alice".into() }).await?;

// Différé :
use std::time::Duration;
Mail::to("alice@example.org")
    .later(Duration::from_secs(60), Welcome { name: "Alice".into() })
    .await?;
```

Le même garde-fou de corps vide s'exécute sur le chemin de la file
d'attente, si bien qu'un Mailable mal configuré est rejeté au moment
du push, avant qu'aucune enveloppe ne soit créée.

## Télémétrie

Chaque envoi passe par `suprnova::mail::dispatch_with_telemetry`, qui
ouvre un `tracing::info_span!` `mail.send` portant :

- `transport` - nom du driver (`"postmark"`, `"smtp"`, `"in-memory"`, …)
- `to_count`, `cc_count`, `bcc_count` - comptes de destinataires
- `has_html`, `has_text` - forme du corps
- `attachment_count` - nombre de pièces jointes
- `tag_count`, `metadata_count` - comptes d'indices fournisseur
- `priority` - `1..=5`, ou `0` si non défini

À la complétion, le span émet `mail sent` (info) ou `mail send
failed` (warn) avec `duration_ms`. Le même wrapper couvre
`Mail::send`, le worker de file `SendMailJob`, et le `MailChannel` de
notification, si bien que le schéma du span est identique quelle que
soit la façon dont le message a été produit.

## Tester avec `Mail::fake()`

`Mail::fake()` installe un transport de capture en mémoire pour la
durée de la garde RAII retournée. Reflète `Bus::fake()` /
`Queue::fake()` / `Cache::fake()` :

```rust
use suprnova::mail::Mail;

#[tokio::test]
async fn welcome_mail_is_sent_on_signup() {
    let fake = Mail::fake();

    sign_up("alice@example.org").await.unwrap();

    fake.assert_sent_count(1);
    fake.assert_sent(|m| m.to.iter().any(|a| a.email == "alice@example.org"));
    fake.assert_sent(|m| m.subject.starts_with("Welcome"));
    fake.assert_not_sent(|m| m.subject.contains("Password reset"));
}
```

Quand la garde se drop, le transport précédemment lié (s'il y en
avait un) est restauré. Les tests qui mélangent `Mail::fake()` avec
une liaison de transport explicite ne fuient pas d'état.

`Mail::fake()` est `Send + Sync` ; partagez-le entre des awaits ou
des threads selon vos besoins.

## Transports personnalisés

Le trait `MailTransport` est le point d'intégration :

```rust
use suprnova::async_trait;
use suprnova::mail::{MailTransport, OutgoingMessage};
use suprnova::FrameworkError;

pub struct StdoutTransport;

#[async_trait]
impl MailTransport for StdoutTransport {
    async fn send(&self, msg: &OutgoingMessage) -> Result<(), FrameworkError> {
        println!("--- mail ---\n{}\n--- end ---", msg.subject);
        Ok(())
    }
    fn name(&self) -> &'static str { "stdout" }
}

// À l'amorçage :
use std::sync::Arc;
suprnova::mail::Mail::set_transport(Arc::new(StdoutTransport))?;
```

Les transports tournent sur le runtime de Tokio - IO async, pooling
de connexions, et envoi concurrent sont de premier ordre. Il n'y a
pas de pénalité de fork par requête.

### Pourquoi Suprnova diverge

La couche Mailable de Laravel est construite sur Symfony Mailer, qui
tourne de façon synchrone à l'intérieur du cycle de vie de la
requête. Le `MailTransport` de Suprnova est `async fn send(&self,
msg: &OutgoingMessage)` de bout en bout : les fournisseurs HTTP
utilisent `reqwest`, le chemin SMTP utilise un adaptateur lettre
async, et `dispatch_with_telemetry` enveloppe chaque envoi dans un
span `tracing` de Tokio. Les fournisseurs à longue distance ne
bloquent pas le thread du handler, les pools de connexions survivent
à travers les requêtes, et des envois concurrents dans un seul
handler sont triviaux -
`tokio::try_join!(Mail::to(a).send(m), Mail::to(b).send(n))` fait ce
à quoi vous vous attendriez.

L'autre divergence est l'annulation d'événement. Laravel modélise un
écouteur `MessageSending` qui peut renvoyer `false` et supprimer
l'envoi (`events->until()`). Le dispatcher de Suprnova n'expose pas
de canal de retour à court-circuit - `MessageSending` est observation
seule. Pour filtrer un envoi, refusez au niveau de la couche Mailable
(surchargez `render_html` / `render_text` pour renvoyer une erreur)
ou enveloppez l'appel `MailBuilder::send` avec votre propre
garde-fou. Le compromis est réel : nous perdons un hook Laravel pour
garder le contrat du dispatcher simple.

Une divergence plus petite est un durcissement délibéré. Laravel se
contente de laisser `MAIL_MAILER=log` tourner en production ;
Suprnova refuse d'y démarrer sans une reconnaissance explicite, parce
qu'un sous-système mail qui rapporte un succès et ne livre rien est
le genre de panne que personne ne remarque pendant des semaines. Le
driver `log` lui-même se comporte exactement comme celui de Laravel -
message complet, corps et liens inclus - ce qui est ce qui le rend
utile en développement, et le refus en production est ce qui garde
ça sûr (voir [Le driver `log` journalise le message
complet](#le-driver-log-journalise-le-message-complet)).

## Bonnes pratiques

### Enregistrez les factories à l'amorçage, pas par requête

`Mail::queue` et `Mail::later` poussent un `SendMailJob` portant le
nom du mailable et le payload JSON - le worker reconstruit le type
concret via `mailable_registry`. Enregistrez chaque `Mailable` qui
peut être mis en file, une fois, au moment de `Server::serve` :

```rust
// bootstrap.rs
pub fn register() -> Result<(), suprnova::FrameworkError> {
    suprnova::mail::register_mailable_factory::<WelcomeEmail>()?;
    suprnova::mail::register_mailable_factory::<PasswordReset>()?;
    suprnova::mail::register_mailable_factory::<InvoiceShipped>()?;
    Ok(())
}
```

Un `Mail::queue` pour un mailable non enregistré atterrit sur la
file d'attente, tourne une fois, tombe sur « unknown mailable »,
réessaie selon la politique de backoff de l'enveloppe, et finit en
lettre morte - coûtant un temps d'observabilité que vous n'auriez pas
dépensé si la factory avait été liée à l'amorçage.

### Mettez en file d'attente le mail pour tout rendu lent ou peu fiable

Envoyer du mail dans un handler de requête couple la latence de
réponse de l'utilisateur à votre serveur SMTP (ou à l'API HTTP de
quelque fournisseur que ce soit). Utilisez `Mail::queue` pour tout ce
qui dépasse un rendu synchrone en dev local, et `Mail::later` quand
vous voulez que le dispatch soit différé - relances d'onboarding,
e-mails de rappel, digests planifiés.

```rust
// Mauvais : lie le temps de réponse au fournisseur de mail
Mail::to(&user.email).send(Welcome { ... }).await?;
return json_response!({ "ok": true });

// Bon : le 200 OK retourne immédiatement ; le worker livre le mail.
Mail::to(&user.email).queue(Welcome { ... }).await?;
return json_response!({ "ok": true });
```

### Fixez toujours `from` sur un Mailable

L'expéditeur par défaut du framework est `noreply@localhost` - utile
pour repérer les expéditeurs manquants en développement, pas un
expéditeur qu'un quelconque fournisseur acceptera en production.
Surchargez `Mailable::from(&self)` (ou définissez `from = "..."` dans
l'attribut `#[mail(...)]` sur un `NotificationMailable`) pour que
chaque message dispatché ait une véritable identité d'expéditeur :

```rust
fn from(&self) -> Option<Address> {
    Some(Address::new("orders@example.com").with_name("Acme Orders"))
}
```

La surcharge par message sur `MailBuilder`
(`.from(("Operations", "ops@example.com"))`) prend le pas sur le
défaut du mailable - utile pour des envois transactionnels ponctuels.

### Utilisez la file d'attente pour une livraison au moins une fois, pas le chemin direct

`MailBuilder::send` est au plus une fois : si le transport échoue à
mi-chemin en dispatchant vers deux fournisseurs, vous ne pouvez pas
réessayer sans risquer un double envoi. `MailBuilder::queue`
chevauche l'enveloppe durable de la file d'attente, qui supporte des
clés d'idempotence et un réessai au niveau worker. Pour tout mail que
vous ne devez ni perdre NI envoyer deux fois, mettez-le en file
d'attente avec une clé d'idempotence stable, liée à l'événement
d'origine.

## Messages ponctuels : `Mail::raw` et `Mail::html`

Quand le mail est un simple ping transactionnel qui ne justifie pas
une struct `Mailable` complète, deux raccourcis sautent le
boilerplate :

```rust
use suprnova::mail::Mail;

// Texte brut
Mail::raw("Your code is 12345", |b| {
    b.to("alice@example.org")
        .subject("Verification code")
        .from("auth@example.com")
}).await?;

// HTML
Mail::html("<p>Hello, <b>world</b></p>", |b| {
    b.to("alice@example.org")
        .subject("Hi")
        .from("hello@example.com")
}).await?;
```

La closure reçoit un [`MailBuilder`] pré-chargé avec le corps et vous
laisse superposer destinataires, sujet, expéditeur, tags, métadonnées,
priorité, et toute autre méthode fluide de [`MailBuilder`]. Ces
chemins contournent entièrement le trait `Mailable` - utile pour des
pings de test ponctuels et de courtes notes transactionnelles.

## Défauts globaux : `always_from`, `always_reply_to`, `always_to`, `always_return_path`

En miroir de `Mailer::alwaysFrom` / `alwaysReplyTo` / `alwaysTo` /
`alwaysReturnPath` de Laravel, la façade Mail expose quatre setters
globaux :

```rust
use suprnova::mail::{Address, Mail};

// À l'amorçage :
Mail::always_from(Address::new("noreply@example.com").with_name("Acme"))?;
Mail::always_reply_to(Address::new("support@example.com"))?;
Mail::always_return_path(Address::new("bounce@example.com"))?;

// « Boîte unique » en dev local - route TOUT le mail vers une seule adresse, abandonne CC/BCC :
Mail::always_to(Address::new("dev-inbox@example.com"))?;

// Annule tout (les tests appellent typiquement ceci au teardown) :
Mail::forget_always()?;
```

La précédence est conservatrice - les défauts ne s'appliquent que
quand le message dispatché manque d'une valeur explicite :

| Champ | Le défaut s'applique quand |
|-------|------------------------------|
| `always_from` | Le `from` du message est le défaut du framework `noreply@localhost` |
| `always_reply_to` | Le message n'a pas de `reply_to` explicite |
| `always_to` | Toujours - route chaque message vers cette adresse, vide CC/BCC |
| `always_return_path` | Le message n'a pas de `return_path` explicite |

La même précédence s'applique sur le chemin de la file d'attente :
les mailables mis en file passent par `apply_always_defaults` au
moment du dispatch par le worker, si bien que les envois directs et
les envois en file convergent vers des formes d'enveloppe identiques.

## Tags, métadonnées, priorité, en-têtes, Return-Path

Chaque message dispatché peut porter des indications pour le
fournisseur à la manière de Laravel - tags, paires clé/valeur de
métadonnées, priorité RFC-2076, en-têtes MIME personnalisés, et une
adresse Sender / de retour des rebonds. Elles sont transmises aux
champs natifs des fournisseurs HTTP (`Tag` / `Metadata` / `Headers`
de Postmark, `EmailTags` plus `Content.Simple.Headers` de SES,
`categories` / `custom_args` / `headers` de SendGrid, `o:tag` / `v:`
/ `h:` de Mailgun, `tags` / `headers` de Resend) et à SMTP sous
forme d'en-têtes RFC 5322.

Sur SES en particulier, les en-têtes voyagent dans la forme de
contenu que le message utilise : `Content.Simple.Headers` pour un
message simple, de vraies lignes d'en-tête MIME pour un message avec
pièces jointes (que SES n'accepte qu'en MIME brut). Un nom d'en-tête
est validé de la même façon quelle que soit la forme que le message
finit par utiliser - CR, LF et NUL sont rejetés (c'est ainsi qu'une
chaîne fournie par l'appelant se transforme en un second en-tête),
et le sont aussi un nom vide, un nom de plus de 76 octets, un octet
non ASCII, ou un `:` ou une espace dans le nom, ce qui correspond à
ce qu'exige le constructeur MIME brut lui-même. Un nom d'en-tête
répété plus d'une fois conserve toutes les valeurs sur le chemin du
message simple mais seulement la dernière valeur sur le chemin avec
pièces jointes - la même limite que celle de SMTP.

Deux façons de les attacher - au niveau du Mailable pour des défauts
par type, ou par message sur le builder :

```rust
use suprnova::async_trait;
use suprnova::mail::{Mailable, PRIORITY_HIGH};
use std::collections::BTreeMap;

#[async_trait]
impl Mailable for OrderShipped {
    fn mailable_name() -> &'static str { "OrderShipped" }
    fn subject(&self) -> String { format!("Order #{} shipped", self.order_id) }
    fn text_template_source(&self) -> Option<String> { Some("...".into()) }

    fn tags(&self) -> Vec<String> { vec!["transactional".into(), "order".into()] }
    fn metadata(&self) -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        m.insert("order_id".into(), self.order_id.to_string());
        m
    }
    fn priority(&self) -> Option<u8> { Some(PRIORITY_HIGH) }
    fn headers(&self) -> Vec<(String, String)> {
        vec![("X-Origin".into(), "warehouse".into())]
    }
}
```

```rust
// Par message sur le builder. Le builder l'emporte en cas de collision de clé de métadonnée ; les tags et en-têtes fusionnent.
Mail::to(&user.email)
    .tag("campaign-spring")
    .metadata("ab_variant", "B")
    .priority(1)
    .header("X-Source", "promo-feed")
    .return_path("bounce@example.com")
    .send(WelcomeEmail { name: user.name.clone() })
    .await?;
```

Les constantes des cinq niveaux de priorité vivent dans
`suprnova::mail::{PRIORITY_HIGHEST, PRIORITY_HIGH, PRIORITY_NORMAL,
PRIORITY_LOW, PRIORITY_LOWEST}` - la même échelle entière `1..=5`
qu'utilise Laravel.

## Inspecter les messages capturés

`OutgoingMessage` porte des helpers d'inspection à la Laravel -
utiles à la fois pour les assertions de test et pour la
journalisation d'audit à l'exécution :

```rust
fn audit_outgoing(m: &suprnova::mail::OutgoingMessage) {
    if m.has_tag("transactional") && m.has_to("alice@example.org") { /* ... */ }
    if m.has_metadata("order_id") { /* ... */ }
    if m.has_subject("Welcome") { /* ... */ }
    if m.has_attachment("invoice.pdf") { /* ... */ }
    if m.has_header("X-Source", "promo-feed") { /* ... */ }
}
```

Les vérifications de destinataire sont insensibles à la casse sur
l'e-mail ; les vérifications de métadonnées, de tag, de sujet, et de
nom de fichier de pièce jointe sont exactes.

## Fake de test : surface étendue

`Mail::fake()` couvre À LA FOIS la piste des envoyés et celle des mis
en file. Le mail envoyé (via `MailBuilder::send`) atterrit dans le
transport en mémoire ; le mail mis en file (via `.queue` / `.later`)
atterrit dans le buffer de file d'attente du fake.

```rust
use suprnova::mail::Mail;

#[tokio::test]
async fn boot_dispatches_welcome() {
    let fake = Mail::fake();

    onboard_user("alice@example.org").await.unwrap();

    // Côté envoyé
    fake.assert_sent_count(1);
    fake.assert_sent(|m| m.has_to("alice@example.org") && m.subject.starts_with("Welcome"));
    fake.assert_sent_to("alice@example.org");
    fake.assert_not_sent(|m| m.subject.contains("Password reset"));

    // Côté mis en file (pour les mails différés)
    fake.assert_queued("WelcomeFollowup");
    fake.assert_queued_to("alice@example.org");
    fake.assert_queued_count(1);

    // Composite
    fake.assert_outgoing_count(2);   // envoyé + mis en file
    fake.assert_not_outgoing("PasswordReset");
}
```

Helpers supplémentaires :

| Helper | But |
|--------|-----|
| `fake.captured()` | Tous les messages envoyés |
| `fake.count()` | Compte des envoyés |
| `fake.queued()` | Tous les `QueuedSnapshot` mis en file |
| `fake.queued_count()` | Compte des mis en file |
| `fake.outgoing_count()` | Envoyés + mis en file |
| `fake.sent(predicate)` | Filtre les envoyés par prédicat |
| `fake.sent_to(email)` | Filtre les envoyés par destinataire |
| `fake.queued_named(name)` | Mailables mis en file d'un nom donné |
| `fake.queued_to(email)` | Mailables mis en file vers un destinataire |
| `fake.assert_sent_count(n)` | Compte exact des envoyés |
| `fake.assert_queued_count(n)` | Compte exact des mis en file |
| `fake.assert_outgoing_count(n)` | Total exact |
| `fake.assert_nothing_sent()` | Buffer des envoyés vide |
| `fake.assert_nothing_queued()` | Buffer des mis en file vide |
| `fake.assert_nothing_outgoing()` | Les deux vides |
| `fake.assert_sent_to(email)` | Au moins un envoyé vers le destinataire |
| `fake.assert_not_sent_to(email)` | Aucun envoyé vers le destinataire |
| `fake.assert_queued(name)` | Au moins un mis en file de ce nom |
| `fake.assert_queued_with(name, fn)` | Au moins un mis en file de ce nom correspondant au prédicat |
| `fake.assert_queued_to(email)` | Au moins un mis en file vers le destinataire |
| `fake.assert_not_queued(name)` | Aucun mis en file de ce nom |

`QueuedSnapshot::decode::<M>()` désérialise le payload dans le `M`
concret, si bien que les prédicats typés fonctionnent sans
boilerplate de décodage sur mesure.

## Événements : `MessageSending` et `MessageSent`

Chaque dispatch réussi déclenche deux événements du framework :

- `MessageSending` - immédiatement AVANT l'appel au transport. Les
  écouteurs observent la forme du message (destinataires, sujet,
  tags, flags de forme du corps).
- `MessageSent` - immédiatement APRÈS un appel au transport réussi.
  Les écouteurs observent la même forme ; les envois échoués
  n'émettent pas cet événement.

```rust
use std::sync::Arc;
use suprnova::events::EventFacade;
use suprnova::mail::MessageSent;

EventFacade::listen::<MessageSent, _>(Arc::new(MyAuditListener)).await;
```

Les deux événements sont observation seule - le dispatcher ne
modélise pas de canal d'annulation à la Laravel. Voir [Pourquoi
Suprnova diverge](#pourquoi-suprnova-diverge) plus haut pour le
contournement de filtrage.

## Commodité multi-destinataires : `Mail::cc` et `Mail::bcc`

La façade Mail expose trois points d'entrée - `to`, `cc`, `bcc` - qui
renvoient tous un `MailBuilder` neuf. Utilisez celui qui correspond à
l'intention de routage dominante :

```rust
// Commencez par un cc / bcc quand le message est avant tout une copie d'audit.
Mail::cc("manager@example.com")
    .to("alice@example.org")
    .send(OrderShipped { /* ... */ })
    .await?;
```

La même surface fluide s'applique, quel que soit le point d'entrée
par lequel vous commencez.

### Testez contre `Mail::fake()`, pas contre le transport lié

`Mail::fake()` installe un transport de capture local au process
pendant la durée de vie de la garde RAII, et restaure ce qui était
lié avant. Les tests qui l'utilisent n'ont pas besoin de nettoyer des
globales à chaque entrée/sortie - la sémantique du drop s'en occupe.
Combinez `#[serial_test::serial]` avec `Mail::fake()` pour les tests
qui modifient la globale de transport ; des tests concurrents
s'écraseraient mutuellement sinon.

## Suivant

- [Notifications](notifications.md) - `Notify::send` se propage à
  travers les canaux mail, base de données, et webpush ;
  `#[derive(NotificationMailable)]` est le raccourci piloté par macro
  par-dessus le trait `Mailable`
- [File d'attente](queues.md) - l'enveloppe durable sur laquelle
  `Mail::queue` et `Mail::later` s'appuient
- [Événements](events.md) - écouter `MessageSending` / `MessageSent`
  plus le modèle de dispatcher plus large
- [Tests](testing.md) - `Mail::fake()` aux côtés des autres gardes
  `*::fake()`
- [Configuration](configuration.md) - l'enregistrement de config
  typée pour les identifiants de service

## Référence

- Trait : `suprnova::mail::Mailable`
- Façade : `suprnova::mail::Mail`
- Amorçage : `suprnova::mail::boot::bootstrap_from_env()`
- Transports : `LogMailTransport`, `InMemoryMailTransport`,
  `SmtpMailTransport`, `PostmarkMailTransport`, `SesMailTransport`,
  `SendGridMailTransport`, `MailgunMailTransport`,
  `ResendMailTransport`
- Job de file d'attente : `suprnova::mail::SendMailJob`
- Garde de test : `suprnova::mail::MailFake`
- Helper de télémétrie : `suprnova::mail::dispatch_with_telemetry`
