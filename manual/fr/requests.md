# Requêtes

Les handlers Suprnova reçoivent une `Request` - la requête HTTP telle
qu'elle arrive du réseau - ou une struct de requête de formulaire typée
qui analyse, valide et autorise le corps avant que votre code ne
s'exécute. Les deux chemins passent par la même macro `#[handler]` ;
vous choisissez la forme route par route. Ce chapitre couvre les deux,
plus l'extracteur de téléversement multipart et les accesseurs bruts
auxquels vous faites appel dans un middleware.

## Requêtes de formulaire typées

L'attribut `#[request]` marque une struct comme `FormRequest`. La macro
ajoute les derives `serde::Deserialize` et `validator::Validate` et émet
un `impl FormRequest`, pour que la macro `#[handler]` sache l'extraire
et la valider à l'entrée :

```rust
use suprnova::request;

#[request]
pub struct CreateUserRequest {
    #[validate(email(message = "Please provide a valid email address"))]
    pub email: String,

    #[validate(length(min = 8, message = "Password must be at least 8 characters"))]
    pub password: String,

    #[validate(length(min = 1, max = 100, message = "Name is required"))]
    pub name: String,
}
```

Un handler qui nomme ce type comme paramètre reçoit une valeur déjà
validée :

```rust
use suprnova::{handler, json_response, Response};
use crate::requests::CreateUserRequest;

#[handler]
pub async fn store(form: CreateUserRequest) -> Response {
    // `form` est validé - ce code ne s'exécute que si chaque règle est passée.
    json_response!({ "email": form.email, "name": form.name })
}
```

Un handler qui nomme `Request` à la place reçoit la requête brute telle
quelle :

```rust
use suprnova::{handler, json_response, Request, Response};

#[handler]
pub async fn index(req: Request) -> Response {
    json_response!({ "path": req.path() })
}
```

Les deux sont des extracteurs - la macro `#[handler]` recherche
`FromRequest::from_request` pour chaque type de paramètre, et toute
struct qui implémente `FormRequest` obtient gratuitement une impl
`FromRequest` générale.

## Règles de validation

La validation passe par la crate `validator`. Règles courantes :

### Validations de chaînes

```rust
#[request]
pub struct ExampleRequest {
    // Requis (non vide)
    #[validate(length(min = 1, message = "This field is required"))]
    pub name: String,

    // Format e-mail
    #[validate(email(message = "Invalid email address"))]
    pub email: String,

    // Format URL
    #[validate(url(message = "Invalid URL"))]
    pub website: String,

    // Contraintes de longueur
    #[validate(length(min = 8, max = 100))]
    pub password: String,

    // Motif regex - PHONE_REGEX doit être un `static` ou un `const`
    // visible depuis le point d'expansion du validateur. Déclarez-le
    // une seule fois, typiquement dans le même module :
    #[validate(regex(path = "PHONE_REGEX", message = "Invalid phone number"))]
    pub phone: String,
}

use std::sync::LazyLock;
use regex::Regex;

// validator 0.20 implémente `AsRegex` pour `std::sync::LazyLock<Regex>`
// mais pas pour `once_cell::sync::Lazy<Regex>` - utilisez le type de la
// std pour que l'expansion `#[validate(regex(path = "..."))]` du derive
// passe la vérification de types.
static PHONE_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\+?[0-9\s\-()]{7,20}$").unwrap());
```

### Validations numériques

```rust
#[request]
pub struct ProductRequest {
    // Validation d'intervalle - les littéraux doivent correspondre au
    // type du champ. `f64` prend `0.0` / `10000.0`, pas les littéraux
    // entiers `0` / `10000`.
    #[validate(range(min = 0.0, max = 10000.0, message = "Price must be between 0 and 10000"))]
    pub price: f64,

    // Valeur minimale
    #[validate(range(min = 1))]
    pub quantity: i32,

    // Valeur maximale
    #[validate(range(max = 100))]
    pub discount_percent: i32,
}
```

### Validations imbriquées et de collections

```rust
use serde::Deserialize;

#[derive(Deserialize, Validate)]
pub struct Address {
    #[validate(length(min = 1))]
    pub street: String,

    #[validate(length(min = 1))]
    pub city: String,
}

#[request]
pub struct OrderRequest {
    // Validation de struct imbriquée
    #[validate(nested)]
    pub shipping_address: Address,

    // Longueur de la collection
    #[validate(length(min = 1, message = "At least one item required"))]
    pub items: Vec<String>,
}
```

### Attributs de validation courants

| Attribut | Description | Exemple |
|-----------|-------------|---------|
| `email` | Format d'e-mail valide | `#[validate(email)]` |
| `url` | Format d'URL valide | `#[validate(url)]` |
| `length` | Longueur d'une chaîne ou d'une collection | `#[validate(length(min = 1, max = 100))]` |
| `range` | Intervalle numérique | `#[validate(range(min = 0, max = 100))]` |
| `regex` | Correspondance avec un motif regex | `#[validate(regex(path = "PATTERN"))]` |
| `contains` | La chaîne contient une sous-chaîne | `#[validate(contains(pattern = "@"))]` |
| `does_not_contain` | La chaîne ne contient pas | `#[validate(does_not_contain(pattern = "admin"))]` |
| `nested` | Valide une struct imbriquée | `#[validate(nested)]` |

## Réponses d'erreur de validation

Quand la validation échoue, Suprnova retourne une réponse 422 avec le
sac d'erreurs compatible Laravel / Inertia :

```json
HTTP 422 Unprocessable Entity

{
    "message": "The given data was invalid.",
    "errors": {
        "email": ["Please provide a valid email address"],
        "password": ["Password must be at least 8 characters"]
    }
}
```

La forme de `errors` correspond exactement à ce que les clients
`@inertiajs/*` lisent depuis `usePage().props.errors`.

## Exemple complet

Un point de terminaison d'inscription d'utilisateur, de bout en bout.

**Définir la requête :**

```rust
// src/requests/create_user.rs
use suprnova::request;

#[request]
pub struct CreateUserRequest {
    #[validate(email(message = "Please provide a valid email address"))]
    pub email: String,

    #[validate(length(min = 8, message = "Password must be at least 8 characters"))]
    pub password: String,

    #[validate(length(min = 2, max = 50, message = "Name must be between 2 and 50 characters"))]
    pub name: String,
}
```

**Créer le contrôleur :**

```rust
// src/controllers/user.rs
use suprnova::{handler, json_response, Request, Response, ResponseExt};
use crate::requests::CreateUserRequest;

#[handler]
pub async fn index(_req: Request) -> Response {
    json_response!({ "users": [] })
}

#[handler]
pub async fn store(form: CreateUserRequest) -> Response {
    // La validation est passée - créer l'utilisateur
    // Dans une vraie application, vous enregistreriez en base ici

    json_response!({
        "user": {
            "email": form.email,
            "name": form.name
        },
        "message": "User created successfully"
    })
    .status(201)
}
```

**Enregistrer les routes :**

```rust
// src/routes.rs
use suprnova::{get, post, routes};
use crate::controllers;

routes! {
    get!("/users", controllers::user::index).name("users.index"),
    post!("/users", controllers::user::store).name("users.store"),
}
```

## Autorisation et hooks inter-champs

Le trait `FormRequest` expose trois hooks de cycle de vie : `authorize`,
`after_validation` et `after_validation_async`. L'attribut `#[request]`
comme la forme `#[derive(FormRequestDerive)]` émettent pour vous un
`impl FormRequest` par défaut. Pour redéfinir un hook, ajoutez l'opt-out
`#[form_request(custom_hooks)]` afin de supprimer l'impl par défaut,
puis écrivez la vôtre. (Cela reflète le motif
`#[multipart(custom_hooks)]`.)

```rust
use suprnova::{FormRequest, FormRequestDerive, Request};
use serde::Deserialize;
use validator::Validate;

#[derive(Deserialize, Validate, FormRequestDerive)]
#[form_request(custom_hooks)]
pub struct DeleteUserRequest {
    pub user_id: i64,
}

impl FormRequest for DeleteUserRequest {
    fn authorize(req: &Request) -> bool {
        // Retournez false pour court-circuiter avec un 403 Forbidden
        // avant que le corps ne soit lu.
        req.header("X-Admin-Token").is_some()
    }
}
```

L'opt-out fonctionne aussi sous la forme de l'attribut `#[request]` -
utile quand vous voulez les derives automatiques de l'attribut mais
devez redéfinir des hooks :

```rust
use suprnova::{FormRequest, Request, request};

#[request]
#[form_request(custom_hooks)]
pub struct DeleteUserRequestAttr {
    pub user_id: i64,
}

impl FormRequest for DeleteUserRequestAttr {
    fn authorize(req: &Request) -> bool {
        req.header("X-Admin-Token").is_some()
    }
}
```

Quand `authorize` retourne `false`, l'extraction retourne
`FrameworkError::Unauthorized` et rend :

```json
HTTP 403 Forbidden

{ "message": "This action is unauthorized." }
```

`after_validation` est le hook inter-champs synchrone - utilisez-le pour
des règles du type « le mot de passe et sa confirmation doivent
correspondre ». `after_validation_async` en est l'homologue asynchrone,
et c'est là que les règles adossées à la base de données (par ex.
l'`Unique` intégré) participent à la validation automatique. Les deux se
déclenchent une fois que les règles `validator` par champ sont passées ;
`extract` abandonne à la première étape qui échoue.

```rust
use suprnova::{FormRequest, FormRequestDerive, ValidationErrors};
use serde::Deserialize;
use validator::Validate;

#[derive(Deserialize, Validate, FormRequestDerive)]
#[form_request(custom_hooks)]
pub struct UpdatePasswordRequest {
    #[validate(length(min = 8))]
    pub new_password: String,
    pub confirmation: String,
}

impl FormRequest for UpdatePasswordRequest {
    fn after_validation(&self) -> Result<(), ValidationErrors> {
        if self.new_password != self.confirmation {
            let mut errs = ValidationErrors::new();
            errs.add("confirmation", "passwords do not match");
            return Err(errs);
        }
        Ok(())
    }
}
```

### Plafonds de taille du corps

L'attribut `#[form_request(max_body_bytes = N)]`, propre à une struct,
redéfinit le plafond de 8 Mio global au processus sur une seule
FormRequest :

```rust
use suprnova::FormRequestDerive;
use serde::Deserialize;
use validator::Validate;

#[derive(Deserialize, Validate, FormRequestDerive)]
#[form_request(max_body_bytes = 64 * 1024 * 1024)] // 64 Mio
pub struct ImportPayload {
    pub rows: Vec<Row>,
}

#[derive(Deserialize, Validate)]
pub struct Row { /* ... */ }
```

`Content-Length` est analysé d'emblée et la requête est rejetée avec un
HTTP 413 *avant* qu'un seul octet du corps ne soit lu quand la taille
déclarée dépasse le plafond ; les clients qui mentent sur
`Content-Length` déclenchent quand même le compteur d'octets en flux
pendant la lecture.

## Détection du type de contenu

`FormRequest::extract` ne regarde que l'en-tête `Content-Type` :

- `application/x-www-form-urlencoded` → analysé via `serde_urlencoded`
- `application/json` ou tout suffixe `application/*+json` → analysé via `serde_json`
- Tout le reste (y compris un en-tête absent) → rejeté avec un HTTP 415
  Unsupported Media Type, avant que le corps ne soit lu

Pour les corps multipart (`multipart/form-data`), voir les
[téléversements de fichiers](#téléversements-de-fichiers-multipartrequest) ci-dessous.

## Lire le corps directement

Pour les points de terminaison ponctuels ou les middleware qui ne
veulent pas d'une `FormRequest` complète, le type `Request` lit lui-même
le corps de trois manières - chacune consomme `self`, car le corps ne
peut être lu qu'une seule fois :

```rust
use serde::Deserialize;
use suprnova::{handler, json_response, Request, Response};

#[derive(Deserialize)]
struct LoginForm { username: String, password: String }

#[handler]
pub async fn login(req: Request) -> Response {
    // Choisir l'analyseur explicitement.
    let form: LoginForm = req.form().await?;
    json_response!({ "user": form.username })
}

#[handler]
pub async fn webhook(req: Request) -> Response {
    // Même forme, du JSON sur le réseau.
    let payload: serde_json::Value = req.json().await?;
    json_response!({ "received": payload })
}

#[handler]
pub async fn ingest(req: Request) -> Response {
    // Choix automatique selon le Content-Type - JSON sauf si
    // `application/x-www-form-urlencoded` est explicite.
    let value: serde_json::Value = req.input().await?;
    json_response!({ "value": value })
}
```

Pour un accès brut, `req.body_bytes().await` retourne les `Bytes` mis en
tampon plus les métadonnées `RequestParts` (params de route et type de
contenu). Utilisez `body_bytes_with_cap(n)` pour redéfinir au cas par
cas le plafond global de 8 Mio.

## Résoudre des services à côté du formulaire

Les requêtes de formulaire validées se composent avec le [conteneur de
service](container.md). Utilisez `App::resolve::<T>()` (ou
`App::get::<T>()`) à l'intérieur du handler :

```rust
use suprnova::{handler, json_response, Response, App};
use crate::requests::CreateUserRequest;
use crate::services::UserService;

#[handler]
pub async fn store(form: CreateUserRequest) -> Response {
    let user_service = App::resolve::<UserService>()?;
    let user = user_service.create_user(&form.email, &form.name).await?;
    json_response!({ "user": user })
}
```

## Téléversements de fichiers (`MultipartRequest`)

`multipart/form-data` a son propre extracteur -
`#[derive(MultipartRequest)]` diffuse le corps partie par partie, en
déversant les grosses parties de fichier dans un fichier temporaire
au-delà du seuil configuré, de sorte qu'un téléversement de 200 Mio ne
tient jamais entièrement en RAM. Chaque champ porte une annotation
`#[field("name")]` qui nomme le champ tel qu'il est transmis ; les
champs de type fichier utilisent `UploadedFile<V>`, où `V` est un
validateur (ou un tuple de validateurs) de
`suprnova::http::upload::validators`.

```rust
use suprnova::{handler, json_response, MultipartRequest, Response};
use suprnova::http::upload::UploadedFile;
use suprnova::http::upload::validators::{Image, MaxSize};

#[derive(MultipartRequest)]
pub struct AvatarUpload {
    #[field("avatar")]
    pub avatar: UploadedFile<(Image, MaxSize<5_242_880>)>, // plafond de 5 Mio
    #[field("caption")]
    pub caption: Option<String>,
}

#[handler]
pub async fn upload_avatar(form: AvatarUpload) -> Response {
    // `avatar` est en mémoire ou dans un fichier temporaire selon sa taille.
    // `.bytes()` lit l'un comme l'autre ; `.store_as(...)` diffuse vers un disque.
    let bytes = form.avatar.bytes().await?;
    json_response!({ "size": bytes.len(), "caption": form.caption })
}
```

Formes de champ :

| Déclaration | Forme transmise |
|---|---|
| `UploadedFile<V>` | fichier requis |
| `Option<UploadedFile<V>>` | fichier facultatif |
| `Vec<UploadedFile<V>>` | téléversements en tableau (`photos[]`) |
| `String` / `u32` / tout `FromStr` | champ texte (requis) |
| `Option<String>` / `Option<T: FromStr>` | champ texte facultatif |
| `Vec<String>` / `Vec<T: FromStr>` | champs texte répétés |

Validateurs intégrés dans `suprnova::http::upload::validators` :

- `MaxSize<N>` - court-circuite à la limite d'octet quand le total
  courant dépasse `N` octets (HTTP 413).
- `Image` - rejette les parties dont les octets magiques ne se déclarent
  pas `image/*`.
- `MimeType<L>` - accepte une allowlist fixe fournie par votre propre
  type `MimeAllowlist`.
- `()` - sans effet ; `UploadedFile<()>` accepte n'importe quels octets.

Les validateurs se composent en tuples : `(Image, MaxSize<5_242_880>)`
exécute les deux, en court-circuitant au premier échec.

### Plafonds par champ et bornes de tableau

Le plafond d'octets sur le corps total est global (8 Mio par défaut pour
le multipart, configurable via
`suprnova::http::upload::set_global_max_multipart_body_bytes`). Les
plafonds par champ empêchent l'abus consistant à faire croître un
`Vec<UploadedFile<_>>` sans borne, à l'intérieur du budget d'octets, au
moyen d'un corps constitué de nombreuses petites parties :

```rust
#[derive(MultipartRequest)]
pub struct Gallery {
    #[field("photos", max_count = 8)]
    pub photos: Vec<UploadedFile<MaxSize<1_048_576>>>,
}
```

La (`max_count` + 1)-ième partie portant ce nom retourne un HTTP 422
avant toute allocation, donc la partie surnuméraire n'atteint jamais la
croissance du `Vec`.

### Hooks d'autorisation et de post-validation

`MultipartRequest` reflète les hooks de `FormRequest` via le trait
`MultipartRequestHooks`. Par défaut, le derive émet une impl vide ;
optez pour la vôtre avec `#[multipart(custom_hooks)]` :

```rust
use suprnova::{MultipartRequest, Request, ValidationErrors};
use suprnova::http::upload::{MultipartRequestHooks, UploadedFile};

#[derive(MultipartRequest)]
#[multipart(custom_hooks)]
pub struct GuardedUpload {
    #[field("file")]
    pub file: UploadedFile,
}

impl MultipartRequestHooks for GuardedUpload {
    fn authorize(req: &Request) -> bool {
        req.header("X-Admin-Token").is_some()
    }

    fn after_validation(&self) -> Result<(), ValidationErrors> {
        if self.file.size == 0 {
            let mut errs = ValidationErrors::new();
            errs.add("file", "empty file");
            return Err(errs);
        }
        Ok(())
    }
}
```

### Streaming vers le stockage

`UploadedFile::store_as` écrit la partie sur un disque de stockage
enregistré. Pour les parties adossées au disque, le chemin est
entièrement en flux (blocs de 64 Kio via `opendal::Operator::writer`) ;
les parties en mémoire utilisent un unique appel d'écriture. Utilisez
l'extension dérivée du contenu quand le chemin de stockage est adressé
par contenu - l'en-tête de nom de fichier n'est pas digne de confiance :

```rust
use suprnova::Storage;

let disk = Storage::disk("avatars")?;
let path = format!("{}.{}", user.id, form.avatar.extension_from_magic());
form.avatar.store_as(&disk, &path).await?;
```

Voir [Système de fichiers et stockage](filesystem.md) pour le registre
des disques de stockage.

## Organisation des fichiers

La structure standard pour les requêtes :

```
src/
├── requests/
│   ├── mod.rs                 # Réexporte toutes les requêtes
│   ├── create_user.rs         # CreateUserRequest
│   ├── update_user.rs         # UpdateUserRequest
│   └── create_post.rs         # CreatePostRequest
├── controllers/
│   └── user.rs                # Utilise CreateUserRequest
└── routes.rs
```

**src/requests/mod.rs :**
```rust
pub mod create_user;
pub mod update_user;

pub use create_user::CreateUserRequest;
pub use update_user::UpdateUserRequest;
```

## Sûreté de typage de bout en bout avec Inertia

Les requêtes peuvent aussi dériver `InertiaProps` pour générer des types TypeScript, ce qui donne une sûreté de typage de bout en bout, de votre backend Rust jusqu'à votre frontend React.

### Générer les types TypeScript pour les requêtes

Ajoutez le derive `InertiaProps` à côté de `#[request]` :

```rust
use suprnova::{request, InertiaProps};

#[request]
#[derive(InertiaProps)]
pub struct CreateTodoRequest {
    #[validate(length(min = 1, message = "Title is required"))]
    pub title: String,

    #[validate(length(max = 500))]
    pub description: Option<String>,
}
```

Lancez la génération de types :

```bash
suprnova generate-types
```

Cela génère des types TypeScript dans `frontend/src/types/inertia-props.ts` :

```typescript
export interface CreateTodoRequest {
  title: string
  description: string | null
}
```

### Formulaires typés avec Inertia

Utilisez le composant `<Form>` d'Inertia pour la gestion de formulaire la plus propre :

```tsx
import { Form, usePage } from '@inertiajs/react'

export default function CreateTodo() {
  const { errors } = usePage().props

  return (
    <Form action="/todos" method="post">
      <input
        type="text"
        name="title"
        placeholder="Todo title"
      />
      {errors?.title && <span className="error">{errors.title}</span>}

      <textarea
        name="description"
        placeholder="Description (optional)"
      />

      <button type="submit">Create Todo</button>
    </Form>
  )
}
```

Pour plus de contrôle, combinez `<Form>` avec le hook `useForm` et vos types générés :

```tsx
import { Form, useForm } from '@inertiajs/react'
import type { CreateTodoRequest } from '../types/inertia-props'

export default function CreateTodo() {
  const { data, setData, errors, processing } = useForm<CreateTodoRequest>({
    title: '',
    description: null,
  })

  return (
    <Form action="/todos" method="post">
      {({ processing }) => (
        <>
          <input
            type="text"
            name="title"
            value={data.title}
            onChange={(e) => setData('title', e.target.value)}
            placeholder="Todo title"
          />
          {errors.title && <span className="error">{errors.title}</span>}

          <textarea
            name="description"
            value={data.description || ''}
            onChange={(e) => setData('description', e.target.value || null)}
            placeholder="Description (optional)"
          />

          <button type="submit" disabled={processing}>
            Create Todo
          </button>
        </>
      )}
    </Form>
  )
}
```

### Ce que le derive vous apporte

- TypeScript attrape les fautes de frappe dans les noms de champ et les
  incompatibilités de type dès la compilation.
- L'autocomplétion de l'IDE lit directement le `.ts` généré.
- Renommez un champ en Rust, relancez `suprnova generate-types`, et la
  surface TypeScript suit.

Voir [Types TypeScript](frontend-typescript-types.md) pour le pipeline
de génération complet.

## Accesseurs de `Request`

Au-delà du motif de formulaire validé ci-dessus, le type `Request` porte des accesseurs façon Laravel pour inspecter la requête telle qu'elle arrive du réseau - URL, en-têtes, chaîne de requête, négociation de contenu, métadonnées de route et IP du client. Ils sont utiles dans un middleware, dans les handlers qui veulent un accès brut à côté d'une `FormRequest`, et partout où l'analyse validée n'est pas le bon outil.

### URL et chemin

| Méthode | Retourne | Notes |
|--------|---------|-------|
| `req.path()` | `&str` | Chemin brut de l'URI. |
| `req.decoded_path()` | `String` | Chemin avec les échappements en pourcentage résolus. |
| `req.segments()` | `Vec<String>` | Chemin découpé sur `/`, segments vides écartés. |
| `req.segment(index, default)` | `Option<String>` | Accès aux segments, numérotés à partir de 1. |
| `req.url()` | `String` | Schéma + hôte + chemin (sans chaîne de requête). |
| `req.full_url()` | `String` | URL + chaîne de requête. |
| `req.full_url_with_query(&[("k","v")])` | `String` | Ajoute ou redéfinit des clés de requête. |
| `req.full_url_without_query(&["k"])` | `String` | Retire des clés de requête. |

```rust
use suprnova::{handler, json_response, Request, Response};

#[handler]
pub async fn show(req: Request) -> Response {
    if req.is(&["admin/*"]) {
        // le chemin correspond au motif générique admin/*
    }
    json_response!({ "url": req.full_url() })
}
```

### Hôte, schéma, IP

| Méthode | Retourne | Ordre des sources |
|--------|---------|--------------|
| `req.host()` | `Option<String>` | `X-Forwarded-Host` → `Host` → autorité de l'URI. |
| `req.http_host()` | `Option<String>` | L'hôte plus le port quand il n'est pas celui par défaut. |
| `req.scheme_and_http_host()` | `Option<String>` | `scheme://host:port`. |
| `req.scheme()` | `&'static str` | `"https"` quand [`secure`] est vrai, sinon `"http"`. |
| `req.secure()` | `bool` | Schéma de l'URI → `X-Forwarded-Proto` → `X-Forwarded-Ssl: on`. |
| `req.ip()` | `Option<String>` | `X-Forwarded-For[0]` → `X-Real-IP` → adresse du pair. |
| `req.ips()` | `Vec<String>` | Chaîne complète : en-têtes de proxy, puis adresse du pair. |
| `req.user_agent()` | `Option<&str>` | En-tête `User-Agent`. |
| `req.port()` | `Option<u16>` | Port de l'en-tête Host → `X-Forwarded-Port` → port de l'URI. |

### En-têtes et méthode

| Méthode | Retourne |
|--------|---------|
| `req.has_header("X-Foo")` | `bool` |
| `req.bearer_token()` | `Option<String>` (dernière sous-chaîne `Bearer `, virgules retirées) |
| `req.is_method("POST")` | `bool` (insensible à la casse) |
| `req.ajax()` | `X-Requested-With: XMLHttpRequest` |
| `req.pjax()` | En-tête `X-PJAX` à valeur vraie |
| `req.prefetch()` | `X-Moz`, `Purpose` ou `Sec-Purpose` = `prefetch` |

### Négociation de contenu

```rust
if req.is_json() { /* le Content-Type porte /json ou +json */ }
if req.expects_json() { /* AJAX sans restriction d'Accept, ou Accept préfère JSON */ }
if req.wants_json() { /* l'en-tête Accept place JSON en tête */ }
if req.accepts_html() { /* Accept autorise text/html */ }

let preferred = req.prefers(&["application/json", "text/html"]);
let acceptable = req.acceptable_content_types();
```

`accepts(&[ty])` fait correspondre aussi bien les types nus que les suffixes du style `application/<vendor>+json`. `accepts_any_content_type()` retourne vrai quand il n'y a pas d'en-tête Accept ou que la préférence de tête est `*/*`.

### Chaîne de requête

```rust
let id: Option<String> = req.query_param("id");
let present: bool = req.has_query("id");
let map = req.query_params(); // HashMap<String, String>

// Analyse typée de la chaîne de requête via serde
#[derive(serde::Deserialize)]
struct SearchQuery { page: u32, q: String }
let q: SearchQuery = req.query_into()?;
```

### Métadonnées de route

Après que le routeur a réparti une requête, le motif correspondant est enregistré sur la requête :

```rust
if req.route_is(&["users.show", "users.*"]) {
    // nous sommes dans la route users.show ou users.*
}

let pattern = req.route_pattern(); // Some("/users/{id}")
let name = req.route_name();       // Some("users.show")
```

`route_is(&[...])` accepte les caractères génériques `*` (la sémantique `Str::is` de Laravel).

## Abandonner tôt

Pour une gestion d'erreur en sortie anticipée sans l'enveloppe `Response` complète, les helpers `abort_with` / `abort_if` / `abort_unless` retournent une `FrameworkError` qui se rend à travers le pipeline standard `From<FrameworkError> for HttpResponse`. Ils se composent directement avec `?` :

```rust
use suprnova::{abort_if, abort_unless, abort_with, handler, json_response, Request, Response};

#[handler]
pub async fn show(req: Request) -> Response {
    let id = req.param("id")?;

    // 404 quand la ressource est absente.
    abort_if(id == "0", 404, "User not found")?;

    // 403 quand l'appelant n'est pas authentifié.
    abort_unless(req.has_header("Authorization"), 403, "Login required")?;

    // Ou levez un statut sans condition :
    if some_condition() {
        return Err(abort_with(418, "I'm a teapot").unwrap_err().into());
    }

    json_response!({ "id": id })
}
```

`abort_if` / `abort_unless` retournent `Ok(())` quand la condition est fausse, donc le `?` poursuit normalement.

## Pourquoi Suprnova diverge

Laravel expose un sac d'entrées synchrone et fusionné -
`$req->input('field')`, `$req->all()`, `$req->only(['a','b'])`,
`$req->boolean('flag')` - tiré à la fois de la chaîne de requête et du
corps analysé. Suprnova ne livre pas cette surface. La raison :

- Le corps de Suprnova est à consommation unique et asynchrone. Un
  `all()` synchrone exigerait de mettre chaque corps en tampon d'emblée
  pour satisfaire une méthode que la plupart des handlers n'appellent
  jamais - la surface mémoire et la surface de déni de service diffèrent
  du cycle de vie request-per-process de PHP.
- L'alternative typée (`#[request]` + `FormRequest`) donne des noms de
  champ vérifiés à la compilation, de la validation et une analyse qui
  tient compte du content-type - exactement le filet de sécurité qui
  manque au sac non typé.

Pour inspecter la chaîne de requête, les en-têtes ou la route, faites
appel à `query_param`, `query_into`, `has_query`, `bearer_token` et aux
lecteurs d'en-têtes ci-dessus. Pour l'accès côté corps, définissez une
struct `#[request]` ou un extracteur `#[derive(MultipartRequest)]`.

## Suivant

- [Validation](validation.md) - la bibliothèque de règles derrière
  `#[validate(...)]` et la forme du sac d'erreurs 422
- [Réponses](responses.md) - reconstruire des valeurs `HttpResponse`
  depuis votre handler, streaming et redirections compris
- [Gestion des erreurs](errors.md) - les motifs de handler bâtis sur le
  fait que `Response` est un `Result<HttpResponse, HttpResponse>`
- [Routage](routing.md) - enregistrer des routes et les paramètres
  `{id}` que lit `req.param("id")`
- [Authentification](authentication.md) - `Auth::user_as`,
  `Auth::attempt` et les guards qui résolvent l'utilisateur courant
  depuis la requête
- [Système de fichiers et stockage](filesystem.md) - enregistrer les
  disques de stockage sur lesquels `UploadedFile::store_as` écrit
