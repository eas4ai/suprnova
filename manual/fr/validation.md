# Validation

Suprnova valide les entrées de requête sur deux voies complémentaires :

1. **Validation par derive** - les attributs `#[validate(...)]` sur une
   struct `FormRequest`, exécutés automatiquement par `extract()`. C'est
   le chemin quotidien, couvert dans [Requêtes](requests.md). Il traite
   les règles par champ (`email`, `length`, `range`, …) de façon
   déclarative.
2. **Objets règle + la macro `validate!`** - de simples valeurs
   implémentant [`Rule`](#objets-règle) / `ContextualRule` / `AsyncRule`,
   composées impérativement. Recourez-y quand vous avez besoin de logique
   inter-champs, de règles qui touchent la base de données, ou de règles
   que vous voulez stocker et faire circuler.

Les deux voies s'accumulent dans le même sac
[`ValidationErrors`](error-model.md) et rendent la même forme
Laravel/Inertia `{ "message", "errors": { field: [...] } }` (HTTP
422).

## Objets règle

Une règle est une valeur qui implémente l'un de quatre traits :

| Trait | Forme | Usage |
|-------|-------|-----|
| `Rule` | `passes(&self, value: &str)` | vérification pure sur une valeur |
| `ValueRule` | `passes(&self, value: &serde_json::Value)` | vérification sur une valeur de forme JSON (tableau/objet) |
| `ContextualRule` | `passes(&self, value, ctx)` | vérification qui lit les champs voisins |
| `AsyncRule` | `async passes(&self, value)` | vérification qui fait un `.await` (BD, HTTP) |

`Rule`s intégrées : `Required`, `Email`, `Min`, `Max`, `Between`, `In`,
`NotIn`, `Integer`, `Numeric`, `Boolean`, `Alpha`, `AlphaNum`, `Url`,
`UrlProtocols`, `HttpUrl`, `Uuid`,
[`Password`](#robustesse-du-mot-de-passe) (vérifications de robustesse
seulement). `ValueRule`s intégrées : `ArrayKeys`, `Distinct`.
`ContextualRule`s intégrées : `RequiredIf`, `RequiredWith`,
`RequiredUnless`, `Same`, `Different`, `Confirmed`. `AsyncRule`s
intégrées : [`Unique`](#la-règle-unique) et
[`Password`](#robustesse-du-mot-de-passe) (robustesse plus sa
vérification HIBP `uncompromised()` - la seule règle intégrée à
implémenter à la fois `Rule` et `AsyncRule`).

```rust
use suprnova::{Rule, rules::Email};

Email.passes("user@example.com")?; // Ok(())
```

> **Remarque :** `Numeric` accepte un nombre **fini** - `NaN`, `inf` et les grandeurs
> qui débordent vers l'infini sont rejetés, alors même que l'analyseur de
> Rust accepterait ces chaînes.

### Schémas d'URL

`Url` accepte une valeur qui s'analyse comme une URL, dont le schéma est
sur la liste blanche de Laravel - la même liste qu'utilise
`Illuminate\Support\Str::isUrl` -, est suivi de `://`, **et** est suivi à
son tour d'un hôte non vide, ce qui correspond en forme au motif
`^(PROTOCOLS)://HOST` de Laravel (le groupe hôte de Laravel n'a pas de
`?` : un hôte absent ou vide ne correspond jamais). La liste des schémas
et l'exigence `://` plus hôte sont celles de Laravel mot pour mot ; l'hôte
est analysé par le crate `url` plutôt que par la regex de Laravel, si bien
qu'un port hors plage est rejeté ici alors que Laravel l'accepterait. Les
trois conditions doivent tenir : `mailto:`, `tel:` et `data:` sont sur la
liste blanche par leur nom mais ne portent aucune composante
d'autorité, donc `Url` les rejette ; et `file:///etc/passwd` échoue pour
la troisième raison - il a bien `://`, mais rien ne se trouve entre le
troisième et le quatrième `/`, et rien n'est pas un hôte. `javascript:` et
`vbscript:` sont rejetés d'emblée ; ils ne sont pas du tout sur la liste
d'autorisation.

`ftp://host/x` et `ssh://host` - de vrais hôtes, simplement pas des
schémas web - passent quand même : `Url` n'est donc pas une vérification
du type « ceci est une page web », et elle ne dit rien de l'endroit où
l'URL se résout. Rejeter `javascript:` rend une valeur validée sûre à
poser dans un `href`, pas sûre à récupérer. Une cible de webhook ou de
callback a encore besoin de `HttpUrl` (ou de vos propres vérifications de
schéma et anti-SSRF) ; `Url` seule ne couvre pas cela.

Pour un ensemble plus étroit, nommez les schémas que vous voulez :

```rust
use suprnova::{Rule, rules::Url};

// Le `url:http,https` de Laravel
Url::protocols(&["https"]).passes("https://example.com")?;   // Ok
Url::protocols(&["https"]).passes("http://example.com");     // Err

// La même chose, sous un nom
use suprnova::rules::HttpUrl;
HttpUrl.passes("https://example.com")?;
```

`Url::protocols(...)` **remplace** la liste blanche au lieu de la
restreindre, si bien qu'une application peut accepter son propre schéma de
lien profond (`myapp://…`) sans que le framework ait un avis là-dessus -
l'exigence `://` plus hôte s'applique aussi à ce schéma personnalisé.
Utilisez `HttpUrl` (ou `Url::protocols(&["https"])`) pour les entrées de
callback, de webhook et d'avatar : une cible de webhook qui se résout en
`ftp://internal-host/` s'analyse encore comme une `Url`, et une cible
`ftp:` n'est pas une cible de webhook.

### Robustesse du mot de passe

`Password` vérifie la longueur et la robustesse par classes de caractères,
plus une vérification Have I Been Pwned `uncompromised()` optionnelle -
l'objet règle `Password` de Laravel, porté. Construisez-le avec
`Password::min(n)` et enchaînez les builders de robustesse :

```rust
use suprnova::{Password, Rule};

let rule = Password::min(8).letters().mixed_case().numbers().symbols();
Rule::passes(&rule, "Str0ng! Pass")?; // Ok(())
Rule::passes(&rule, "weak");          // Err - trop court, sans chiffre, sans symbole
```

| Builder | Exige | Regex Laravel |
|---|---|---|
| `.min(n)` (via `Password::min`) | au moins `n` caractères (plancher à 1) | vérification de longueur |
| `.max(n)` | au plus `n` caractères | vérification de longueur |
| `.letters()` | au moins une lettre Unicode | `/\pL/u` |
| `.mixed_case()` | une majuscule et une minuscule, dans n'importe quel ordre | `/(\p{Ll}+.*\p{Lu})\|(\p{Lu}+.*\p{Ll})/u` |
| `.numbers()` | au moins un chiffre Unicode | `/\pN/u` |
| `.symbols()` | au moins un séparateur, symbole ou signe de ponctuation - **une simple espace compte** | `/\p{Z}\|\p{S}\|\p{P}/u` |

`Password::defaults_with(|| Password::min(12).letters().mixed_case().numbers())`,
appelé une fois depuis `bootstrap::register()`, fixe le défaut valable
pour tout le processus que `Password::defaults()` retourne partout
ailleurs - le `Password::defaults(fn () => ...)` de Laravel. Un second
appel est ignoré (avec un `tracing::warn!`) plutôt que de remplacer
silencieusement la politique choisie par la première application.

#### `uncompromised()` - parce que la robustesse seule ne suffit pas

`.uncompromised()` (ou `.uncompromised_with_threshold(n)`) ajoute une
vérification contre le corpus de fuites Have I Been Pwned, en utilisant
son API de plage à k-anonymat : seuls les **5 premiers caractères** du
hachage SHA-1 en majuscules du mot de passe quittent le processus - `GET
https://api.pwnedpasswords.com/range/{prefix}` - et la comparaison avec le
hachage complet se fait localement, contre les lignes `SUFFIX:COUNT` que
l'API retourne pour ce préfixe. Le service ne voit jamais le mot de passe,
ni même son hachage complet. La comparaison au seuil est stricte
(`count > threshold`), si bien que l'`uncompromised()` par défaut (seuil
`0`) échoue à la moindre apparition, et qu'un échec réseau, un délai
d'attente dépassé ou une réponse non 2xx **échoue en mode ouvert** : le
mot de passe est considéré comme sain plutôt que de bloquer chaque
inscription pendant une panne de Have I Been Pwned. Cela correspond
exactement au `NotPwnedVerifier` de Laravel.

Comme cette vérification est un aller-retour HTTP, `uncompromised()` a
besoin d'`AsyncRule`, et non de la `Rule` synchrone qui suffit aux seules
vérifications de robustesse. Câblez-la à travers
`after_validation_async`, la même recette qu'utilise
[`Unique`](#la-règle-unique) :

```rust
use suprnova::{AsyncRule, FormRequest, Password, ValidationErrors, async_trait};
use serde::Deserialize;
use validator::Validate;

#[derive(Deserialize, Validate)]
pub struct Register {
    pub password: String,
}

#[async_trait]
impl FormRequest for Register {
    async fn after_validation_async(&self) -> Result<(), ValidationErrors> {
        let mut errs = ValidationErrors::new();
        Password::defaults()
            .uncompromised()
            .check_async(&self.password, &mut errs, "password")
            .await;
        errs.into_result()
    }
}
```

Appeler le `Rule::passes` synchrone sur un `Password` doté
d'`uncompromised()` est une **erreur explicite**, pas un saut silencieux -
une vérification de sécurité qui ne fait discrètement rien est pire
qu'une vérification qui n'a jamais existé. Le message d'erreur nomme
`after_validation_async` comme correctif.

`HIBP_TIMEOUT_SECS` (défaut `30`) contrôle le délai d'attente de la
requête - voir [Variables d'environnement](env-vars.md).

Un vérificateur personnalisé qui retourne `Err` est un cas différent d'une
vérification échouée : le texte de son erreur est journalisé au niveau
`error` et n'atteint jamais le client, et la réponse porte à la place la
clé de catalogue `validation-password-unverifiable` (« The { $field }
could not be checked against known data leaks. Please try again. »).
Ajoutez cette clé si vous livrez votre propre catalogue de validation.

### Pourquoi Suprnova diverge : Password

- Le `Password` de Laravel rassemble chaque vérification de robustesse
  échouée dans un seul tableau. Le contrat `Rule` de Suprnova retourne un
  unique `ValidationMessage`, si bien que `Rule::passes` rapporte la
  PREMIÈRE vérification en échec, dans l'ordre min, max, casse mixte,
  lettres, symboles, chiffres - corrigez-en une à la fois plutôt que de
  voir toute la liste d'emblée.
- Le validateur synchrone de Laravel peut appeler `uncompromised()`
  directement ; une requête PHP est déjà à l'intérieur d'une boucle
  d'événements qui tolère un appel HTTP bloquant. Le `Rule::passes` de
  Suprnova est synchrone par contrat : il n'y a donc aucun endroit sûr
  d'où lancer la requête HIBP. Plutôt que de sauter silencieusement la
  vérification - la seule issue inacceptable pour une règle relevant de la
  sécurité -, le `Rule::passes` de Suprnova retourne une erreur explicite,
  destinée au développeur, qui nomme `after_validation_async` comme
  correctif.
- `Password::defaults_with` prend un simple pointeur de `fn`, pas une
  closure, si bien que le défaut configuré reste `Copy` et n'a besoin
  d'aucune allocation sur le tas - une restriction délibérée par rapport
  à la `Closure` de Laravel.

### Écrire votre propre règle

Une règle personnalisée est une struct unitaire (ou porteuse de données)
avec une seule impl. Le trait vous donne `check()` gratuitement - il pousse
tout message d'échec dans un sac `ValidationErrors` sous le champ nommé -
si bien que la règle se branche telle quelle dans `validate!` et dans les
hooks `after_validation` :

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

// Utilisable partout désormais :
StartsWith("acct_").passes("acct_1234")?;
// ou, dans une ligne validate! :
//   stripe_id => Required, StartsWith("acct_");
```

Une `String` se convertit en un `ValidationMessage` qui s'affiche mot pour
mot, et c'est tout ce dont une application monolingue a besoin. Pour que
le message soit traduit selon la locale, retournez plutôt un message
*à clé* - `ValidationMessage::keyed("validation-starts-with").arg("prefix", self.0).fallback(…)` -
et définissez l'id dans `lang/<locale>/validation.ftl`. Voir
[Localisation](localization.md), qui couvre aussi la surcharge des
messages des règles intégrées et la convention de nommage `field-<name>`.

Pour une logique inter-champs, implémentez plutôt [`ContextualRule`] : la
méthode `passes` reçoit un `&FormContext` (une `HashMap<String, String>`
des valeurs des champs voisins) à côté de la valeur testée. Pour les
vérifications adossées à la base de données, implémentez [`AsyncRule`] et
utilisez-la depuis `after_validation_async`.

### Règles de forme valeur

`Rule` ne voit jamais qu'un `&str`. Deux règles intégrées ont besoin de
plus de structure qu'une chaîne n'en porte : elles implémentent donc
`ValueRule`, sur `&serde_json::Value` :

```rust
use suprnova::{ValueRule, rules::{ArrayKeys, Distinct}};

// Le array:keys de Laravel - rejette les clés hors de l'ensemble autorisé.
// Les clés listées n'ont pas toutes à être présentes ; une liste autorisée
// vide est une erreur de programmation, rapportée comme un message sans clé.
ArrayKeys(&["name", "email"]).passes(&serde_json::json!({"name": "Ada"}))?;

// Les distinct / distinct:ignore_case / distinct:strict de Laravel.
Distinct { ignore_case: false, strict: false }
    .passes(&serde_json::json!(["a", "b", "c"]))?;
```

Un champ validé par une `ValueRule` doit contenir un
`serde_json::Value` lui-même (ou `Option<serde_json::Value>` pour une
ligne `?:`/`?=>`) - typiquement un champ de requête tiré directement du
corps JSON. Les lignes `validate!` acceptent des `Rule`s et des
`ValueRule`s dans la même liste de champs ; le trait qui s'exécute est
déterminé par celui que le type de la règle implémente, pas par ce que
vous écrivez dans la ligne.

### Pourquoi Suprnova diverge

Le `distinct:strict` de Laravel s'appuie sur le `==` coercitif de PHP. Les
valeurs JSON sont déjà typées, si bien que le `strict` de Suprnova ne
change que le fait que deux *nombres* de représentations internes
différentes (`1` face à `1.0`) comptent comme égaux - il ne rend jamais
une chaîne et un nombre « identiques », dans aucun des deux modes.

## La macro `validate!`

`validate!` exécute une chaîne de règles sur les champs d'une struct, en
accumulant chaque échec dans une seule `ValidationErrors`. C'est
l'endroit idiomatique du hook inter-champs synchrone,
[`after_validation`](#hooks-inter-champs).

```rust
use suprnova::{validate, ValidationErrors, rules::{Required, Email, Min, Max, RequiredIf}};

fn after_validation(&self) -> Result<(), ValidationErrors> {
    // Les règles contextuelles lisent les valeurs voisines dans un
    // `FormContext` que vous construisez - une map nom de champ → chaîne.
    let mut ctx = std::collections::HashMap::new();
    ctx.insert("billing_type".to_string(), self.billing_type.clone());
    validate! { self =>
        email       => Required, Email;          // ligne de forme requise
        bio         ?: Min(10), Max(500);        // facultatif : valider si Some
        card_number ?=> RequiredIf {             // présence conditionnelle (voir plus bas)
            other: "billing_type",
            value: "card",
        } => with ctx;
    }
}
```

Chaque ligne prend l'une de trois formes :

- **`field => Rule1, Rule2;`** - forme requise. Les règles s'exécutent
  directement sur `&self.field` (pour `String`, `i64`, ou tout ce qui se
  déréférence vers l'emprunt attendu par la règle) - ou, pour une `ValueRule`,
  directement sur un champ `serde_json::Value`. Quel trait chaque règle utilise
  est déduit automatiquement.
- **`field ?: Rule1, Rule2;`** - facultatif. Le champ est un `Option<T>` ;
  les règles ne s'exécutent que lorsqu'il vaut `Some`, et sont
  **entièrement ignorées sur `None`**. C'est la sémantique « si
  présent, valider » (`sometimes`) de Laravel.
- **`field ?=> Rule1, Rule2;`** - présence conditionnelle. Également pour
  un champ `Option<String>`, mais les règles s'exécutent **même sur
  `None`** (l'absence est traitée comme la chaîne vide). C'est la ligne
  des règles conditionnelles à la présence comme `RequiredIf`, qui doivent
  pouvoir *faire échouer un champ absent* - le cas que `?:` ne peut pas
  exprimer, puisqu'il ignore tout sur `None`.

Une règle contextuelle est suivie de `=> with $ctx` (un
`&HashMap<String, String>` de valeurs voisines). La macro est
**synchrone** - pour les règles asynchrones, utilisez le
[hook](#règles-asynchrones-dans-les-requêtes) ci-dessous.

> **Avertissement :** un piège courant, écrire
> `card_number ?: RequiredIf {...} => with ctx;`. Sur une ligne `?:`,
> `None` saute toutes les règles, si bien que `RequiredIf` ne peut jamais
> faire échouer un champ absent. Utilisez `?=>` pour toute règle qui doit
> se déclencher sur l'absence.

## Hooks inter-champs

`FormRequest` exécute deux hooks inter-champs après les règles par champ
issues du derive, aussi bien dans le flux normal que dans celui de
Precognition. `extract()` exécute les étapes dans l'ordre - le
`validate()` dérivé, puis `after_validation`, puis
`after_validation_async` - et **s'arrête à la première étape en échec**.

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

> **Remarque :** redéfinir un hook exige un `impl FormRequest` écrit à la main -
> l'attribut `#[request]` et `#[derive(FormRequest)]` génèrent leur propre
> impl (vide), ils ne conviennent donc qu'au cas courant, sans
> redéfinition.

### Règles asynchrones dans les requêtes

La macro `validate!` ne sait pas tisser un `.await`, si bien que les
règles adossées à la base de données s'exécutent dans
`after_validation_async` - l'étape finale de validation, que `extract()`
appelle automatiquement. C'est là que [`Unique`](#la-règle-unique) et
toute `AsyncRule` personnalisée participent à la validation automatique
des requêtes ; aucune plomberie par handler n'est nécessaire.

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

Comme l'étape asynchrone ne s'exécute qu'une fois les étapes synchrones
passées, une valeur mal formée (un e-mail syntaxiquement invalide)
n'atteint jamais la requête `Unique` en base de données.

## La règle `Unique`

`Unique` vérifie qu'une valeur n'existe pas déjà dans une table.
Construisez-la avec `Unique::new(table, column)` et affinez-la avec l'API
fluide :

```rust
use suprnova::Unique;

// l'e-mail doit être unique, en ignorant la ligne en cours d'édition
Unique::new("users", "email").ignore(current_user_id)

// e-mail unique *par tenant*, comparé sans tenir compte de la casse
Unique::new("users", "email")
    .where_eq("tenant_id", tenant_id)
    .case_insensitive()
```

| Méthode du builder | Effet |
|----------------|--------|
| `.ignore(id)` | exclut la ligne dont l'`id` vaut `id` (cas de l'édition de soi-même) |
| `.ignore_with_column(col, id)` | exclut sur une colonne clé autre qu'`id` |
| `.where_eq(col, value)` | cantonne la vérification aux lignes où `col = value` ; plusieurs appels se combinent par ET |
| `.case_insensitive()` | compare avec `LOWER(col) = LOWER(?)` |

La table, la colonne, la clé d'exclusion et chaque colonne de `where_eq`
sont validées contre une allowlist d'identifiants avant d'atteindre la
chaîne SQL ; la valeur testée et toutes les valeurs de cantonnement sont
des paramètres liés.

### Unique est indicative - la contrainte de base de données est la garantie

`Unique` exécute un `SELECT COUNT(*)` **avant** l'écriture, elle porte
donc une condition de course inévitable entre le moment de la
vérification et le moment de l'usage : deux requêtes simultanées peuvent
toutes deux passer la vérification, puis toutes deux insérer. La règle
`unique` de Laravel a exactement la même propriété. La **seule** garantie
réelle est une contrainte `UNIQUE` (ou un index unique) sur la colonne,
dans votre migration.

Utilisez les trois ensemble :

1. **La règle indicative** - un message rapide et convivial « cet e-mail
   est déjà pris » avant la soumission (et pour que Precognition puisse
   valider le champ).
2. **La contrainte `UNIQUE`** - le garde-fou qui fait autorité contre la
   course.
3. **`FrameworkError::from_unique_violation`** - au site d'écriture,
   ramenez la violation de contrainte que reçoit le perdant de la course
   au même 422 propre, au lieu de laisser fuir un 500 :

```rust
use suprnova::FrameworkError;

// `users.email` porte une contrainte UNIQUE dans la migration.
let user = new_user
    .insert(db)
    .await
    .map_err(|e| FrameworkError::from_unique_violation(
        "email",
        "That email address is already registered.",
        e,
    ))?;
```

`from_unique_violation` retourne une erreur `Validation` 422 quand
l'erreur de base de données est une violation de contrainte d'unicité, et
laisse passer inchangée toute autre erreur (MySQL, Postgres et SQLite sont
tous reconnus).

## Autorisation asynchrone

`FormRequest::authorize(&Request) -> bool` s'exécute **avant** l'analyse
du corps, si bien qu'elle peut rejeter les requêtes non autorisées sans
lire la charge utile. Elle est synchrone par conception : à ce stade, la
requête détient encore le corps en streaming, le hook ne peut donc pas
faire `.await`. Une autorisation qui doit interroger la base de données ou
une policy asynchrone a sa place dans l'un de ces endroits, pas dans
`authorize` :

- **Le middleware** - s'exécute avant `extract()`, est `async`, et
  court-circuite en retournant `Err(response)` (voir
  [Middleware](middleware.md)). Le bon endroit pour « cet utilisateur
  a-t-il seulement le droit d'atteindre cette route ».
- **Le Gate** - appelez `Gate::allows_async` / `Gate::authorize_async`
  dans le handler, une fois que vous avez l'utilisateur authentifié et la
  ressource (voir [Autorisation](authorization.md)).
- **`after_validation_async`** - pour une vérification d'autorisation qui
  dépend du corps de requête analysé, exécutez-la dans le hook asynchrone,
  aux côtés de vos autres règles asynchrones.

## Soumissions de formulaires Inertia

Un échec de validation répond à deux publics différemment. Un client REST
obtient le `422` avec `{ message, errors }`. Une visite Inertia obtient un `303`
de retour vers la page du formulaire avec les erreurs flashées dans la session, parce que
le client Inertia affiche une modale d'erreur pour toute réponse qu'il ne
reconnaît pas comme une réponse Inertia - un `422` ne remplirait jamais
`form.errors`.

Rien dans le handler ne change. Sur la page de destination, chaque champ
porte son premier message comme une chaîne :

```svelte
{#if errors?.email}
  <p class="text-red-600">{errors.email}</p>
{/if}
```

Voir [Réponses Inertia](frontend-inertia-responses.md#validation-failures)
pour les sacs d'erreurs, `with_all_errors`, et où pointe la redirection.

## Notes de conception

- **Validation partielle.** Une `FormRequest` se désérialise en une struct
  typée avant que la validation ne s'exécute, si bien que la struct *est*
  le schéma : un champ qui peut être absent doit être un `Option<T>`.
  C'est aussi ce qui permet à Precognition de valider une charge utile
  partielle - rendez facultatifs les champs qu'un brouillon peut omettre.
- **Messages des règles.** Les règles intégrées retournent des messages à
  clé (`validation-min` avec ses arguments et un repli en anglais),
  résolus via le catalogue à la limite de sérialisation. Traduisez ou
  reformulez n'importe lequel d'entre eux en définissant le même id dans
  `lang/<locale>/validation.ftl` - sans envelopper la règle. Voir
  [Localisation](localization.md).
- **`Min` / `Max` / `Between`** sont des règles de longueur de chaîne
  (comptée en valeurs scalaires Unicode). Pour des bornes numériques,
  validez avec `#[validate(range(...))]` sur le derive ou avec une règle
  personnalisée - les règles de longueur ne sont pas des comparaisons de
  valeurs.

## Résumé

| Tâche | API |
|------|-----|
| Règles par champ | `#[validate(...)]` sur la `FormRequest` (voir Requêtes) |
| Règles composées / inter-champs | `validate! { self => ... }` |
| Règle de forme JSON (tableau/objet) | `field => ArrayKeys(&[...]);` / `field => Distinct { .. };` |
| Facultatif « si présent » | `field ?: Rule;` |
| Facultatif conditionnellement requis | `field ?=> Rule => with ctx;` |
| Règle asynchrone / adossée à la BD | `after_validation_async` + `AsyncRule::check_async` |
| Unicité | `Unique::new(t, c)` + contrainte `UNIQUE` + `from_unique_violation` |
| Autorisation asynchrone | middleware / `Gate::*_async` / `after_validation_async` |

## Suivant

- [Requêtes](requests.md) - la surface `#[request]` /
  `#[derive(FormRequest)]`, le chemin quotidien de la validation dérivée
- [Objets de données](data.md) - `#[derive(Data, Validate)]` pour une
  seule struct qui est à la fois une requête entrante et un DTO sortant
- [Modèle d'erreur](error-model.md) - comment `ValidationErrors` devient
  le corps JSON 422, aux côtés de tous les autres chemins d'erreur
- [Localisation](localization.md) - traduire les messages de règles, la
  convention `field-<name>`, et les `ValidationMessage` à clé
- [Autorisation](authorization.md) - `Gate`, `Policy`, et où se situe
  l'autorisation par rapport à la validation
- [Middleware](middleware.md) - le bon endroit pour les vérifications
  « cette requête a-t-elle seulement le droit de passer » qui ont besoin
  d'un `.await`
