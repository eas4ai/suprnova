# Solicitações

Handlers do Suprnova recebem um `Request` - a solicitação HTTP em nível
de rede - ou um struct de form request tipado que faz parse, valida e
autoriza o corpo antes de o seu código executar. Os dois caminhos vivem
na mesma macro `#[handler]`; você escolhe a forma por rota. Este
capítulo cobre os dois, mais o extractor de upload multipart e os
acessadores crus que você usa em middleware.

## Form requests tipados

O attribute `#[request]` marca um struct como um `FormRequest`. A macro
acrescenta os derives `serde::Deserialize` e `validator::Validate` e
emite um `impl FormRequest`, para que a macro `#[handler]` saiba que
deve extraí-lo e validá-lo na entrada:

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

Um handler que nomeia esse tipo como parâmetro recebe um valor já
validado:

```rust
use suprnova::{handler, json_response, Response};
use crate::requests::CreateUserRequest;

#[handler]
pub async fn store(form: CreateUserRequest) -> Response {
    // `form` está validado - este código só executa se toda regra passou.
    json_response!({ "email": form.email, "name": form.name })
}
```

Já um handler que nomeia `Request` recebe a solicitação crua sem
alterações:

```rust
use suprnova::{handler, json_response, Request, Response};

#[handler]
pub async fn index(req: Request) -> Response {
    json_response!({ "path": req.path() })
}
```

Os dois são extractors - a macro `#[handler]` procura
`FromRequest::from_request` para cada tipo de parâmetro, e todo struct
que implementa `FormRequest` ganha de graça uma impl blanket de
`FromRequest`.

## Regras de validação

A validação passa pelo crate `validator`. Regras comuns:

### Validações de string

```rust
#[request]
pub struct ExampleRequest {
    // Obrigatório (não vazio)
    #[validate(length(min = 1, message = "This field is required"))]
    pub name: String,

    // Formato de email
    #[validate(email(message = "Invalid email address"))]
    pub email: String,

    // Formato de URL
    #[validate(url(message = "Invalid URL"))]
    pub website: String,

    // Restrições de comprimento
    #[validate(length(min = 8, max = 100))]
    pub password: String,

    // Padrão de regex - PHONE_REGEX precisa ser um `static` ou `const`
    // visível a partir do ponto de expansão do validator. Declare-o uma
    // vez, tipicamente no mesmo módulo:
    #[validate(regex(path = "PHONE_REGEX", message = "Invalid phone number"))]
    pub phone: String,
}

use std::sync::LazyLock;
use regex::Regex;

// O validator 0.20 implementa `AsRegex` para `std::sync::LazyLock<Regex>`
// mas não para `once_cell::sync::Lazy<Regex>` - use o tipo da std para que
// a expansão do `#[validate(regex(path = "..."))]` do derive passe na
// checagem de tipos.
static PHONE_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\+?[0-9\s\-()]{7,20}$").unwrap());
```

### Validações numéricas

```rust
#[request]
pub struct ProductRequest {
    // Validação de intervalo - os literais precisam bater com o tipo do
    // campo. `f64` recebe `0.0` / `10000.0`, não os literais inteiros
    // `0` / `10000`.
    #[validate(range(min = 0.0, max = 10000.0, message = "Price must be between 0 and 10000"))]
    pub price: f64,

    // Valor mínimo
    #[validate(range(min = 1))]
    pub quantity: i32,

    // Valor máximo
    #[validate(range(max = 100))]
    pub discount_percent: i32,
}
```

### Validações aninhadas e de coleção

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
    // Validação de struct aninhado
    #[validate(nested)]
    pub shipping_address: Address,

    // Comprimento da coleção
    #[validate(length(min = 1, message = "At least one item required"))]
    pub items: Vec<String>,
}
```

### Attributes comuns de validação

| Attribute | Descrição | Exemplo |
|-----------|-------------|---------|
| `email` | Formato de email válido | `#[validate(email)]` |
| `url` | Formato de URL válido | `#[validate(url)]` |
| `length` | Comprimento de string/coleção | `#[validate(length(min = 1, max = 100))]` |
| `range` | Intervalo numérico | `#[validate(range(min = 0, max = 100))]` |
| `regex` | Correspondência com um padrão de regex | `#[validate(regex(path = "PATTERN"))]` |
| `contains` | String contém a substring | `#[validate(contains(pattern = "@"))]` |
| `does_not_contain` | String não contém | `#[validate(does_not_contain(pattern = "admin"))]` |
| `nested` | Valida o struct aninhado | `#[validate(nested)]` |

## Respostas de erro de validação

Quando a validação falha, o Suprnova retorna uma resposta 422 com o
conjunto de erros compatível com Laravel / Inertia:

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

O formato de `errors` casa diretamente com o que clientes
`@inertiajs/*` leem de `usePage().props.errors`.

### Campos aninhados

Uma falha de `#[validate(nested)]` é reportada sob uma chave pontilhada
que nomeia o caminho completo, a mesma notação que o Laravel usa. Um
struct aninhado contribui com `parent.field`; um elemento de um
`Vec<T>` validado contribui com `parent.<index>.field`:

```json
{
    "message": "The given data was invalid.",
    "errors": {
        "shipping_address.street": ["Validation failed for field 'shipping_address.street'"],
        "items.1.name": ["Validation failed for field 'items.1.name'"]
    }
}
```

O índice `1` é o segundo elemento - o primeiro elemento passou e está
ausente do conjunto. Vincule a chave direto no cliente:
`form.errors['items.1.name']`.

## Exemplo completo

Um endpoint de registro de usuário, de ponta a ponta.

**Defina o request:**

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

**Crie o controlador:**

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
    // A validação passou - crie o usuário
    // Em uma app real, você salvaria no banco de dados aqui

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

**Registre as rotas:**

```rust
// src/routes.rs
use suprnova::{get, post, routes};
use crate::controllers;

routes! {
    get!("/users", controllers::user::index).name("users.index"),
    post!("/users", controllers::user::store).name("users.store"),
}
```

## Autorização e hooks entre campos

A trait `FormRequest` expõe três hooks de ciclo de vida: `authorize`,
`after_validation`, e `after_validation_async`. Tanto o attribute
`#[request]` quanto a forma `#[derive(FormRequestDerive)]` emitem um
`impl FormRequest` padrão para você. Para sobrescrever qualquer hook,
adicione o opt-out `#[form_request(custom_hooks)]` para suprimir a impl
padrão e então escreva a sua. (Isso espelha o padrão
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
        // Retorne false para fazer short-circuit com 403 Forbidden antes
        // de o corpo ser lido.
        req.header("X-Admin-Token").is_some()
    }
}
```

O opt-out também funciona na forma do attribute `#[request]` - útil
quando você quer os auto-derives do attribute mas precisa sobrescrever
hooks:

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

Quando `authorize` retorna `false`, a extração retorna
`FrameworkError::Unauthorized` e renderiza:

```json
HTTP 403 Forbidden

{ "message": "This action is unauthorized." }
```

`after_validation` é o hook síncrono entre campos - use-o para regras
como "senha e confirmação precisam bater". `after_validation_async` é a
versão assíncrona correspondente e é onde regras apoiadas em banco de
dados (por exemplo, o `Unique` embutido) participam da validação
automática. Os dois disparam depois que as regras por campo do
`validator` passam; `extract` aborta no primeiro estágio que falhar.

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

### Limites de tamanho do corpo

O attribute por struct `#[form_request(max_body_bytes = N)]` sobrescreve
o limite process-global de 8 MiB em um único FormRequest:

```rust
use suprnova::FormRequestDerive;
use serde::Deserialize;
use validator::Validate;

#[derive(Deserialize, Validate, FormRequestDerive)]
#[form_request(max_body_bytes = 64 * 1024 * 1024)] // 64 MiB
pub struct ImportPayload {
    pub rows: Vec<Row>,
}

#[derive(Deserialize, Validate)]
pub struct Row { /* ... */ }
```

O `Content-Length` é parseado de antemão e a solicitação é rejeitada com
HTTP 413 *antes* de um byte do corpo ser lido, quando o tamanho
declarado excede o limite; clientes que mentem sobre o `Content-Length`
ainda esbarram no contador de bytes do streaming durante a leitura.

## Detecção de content type

`FormRequest::extract` olha apenas para o header `Content-Type`:

- `application/x-www-form-urlencoded` → parseado via `serde_urlencoded`
- `application/json` ou qualquer sufixo `application/*+json` → parseado via `serde_json`
- Qualquer outra coisa (inclusive um header ausente) → rejeitado com HTTP
  415 Unsupported Media Type, antes de o corpo ser lido

Para corpos multipart (`multipart/form-data`), veja
[uploads de arquivos](#uploads-de-arquivos-multipartrequest) abaixo.

## Lendo o corpo diretamente

Para endpoints pontuais ou middleware que não quer um `FormRequest`
completo, o próprio tipo `Request` lê o corpo de três formas - cada uma
consome `self`, porque o corpo pode ser lido no máximo uma vez:

```rust
use serde::Deserialize;
use suprnova::{handler, json_response, Request, Response};

#[derive(Deserialize)]
struct LoginForm { username: String, password: String }

#[handler]
pub async fn login(req: Request) -> Response {
    // Escolha o parser explicitamente.
    let form: LoginForm = req.form().await?;
    json_response!({ "user": form.username })
}

#[handler]
pub async fn webhook(req: Request) -> Response {
    // Mesma forma, com JSON na rede.
    let payload: serde_json::Value = req.json().await?;
    json_response!({ "received": payload })
}

#[handler]
pub async fn ingest(req: Request) -> Response {
    // Escolha automática com base no Content-Type - JSON, a menos que
    // `application/x-www-form-urlencoded` esteja explícito.
    let value: serde_json::Value = req.input().await?;
    json_response!({ "value": value })
}
```

Para acesso cru, `req.body_bytes().await` retorna os `Bytes`
bufferizados mais os metadados `RequestParts` (params de rota e content
type). Use `body_bytes_with_cap(n)` para sobrescrever o limite global de
8 MiB caso a caso.

## Resolvendo serviços junto com o formulário

Form requests validados se compõem com o
[contêiner de serviços](container.md). Use `App::resolve::<T>()` (ou
`App::get::<T>()`) dentro do handler:

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

## Uploads de arquivos (`MultipartRequest`)

`multipart/form-data` tem seu próprio extractor -
`#[derive(MultipartRequest)]` faz o streaming do corpo parte por parte,
descarregando partes de arquivo grandes em um arquivo temporário acima
do limiar configurado, para que um upload de 200 MiB nunca fique inteiro
na RAM. Cada campo carrega uma anotação `#[field("name")]` que nomeia o
campo transmitido; campos de arquivo usam `UploadedFile<V>`, onde `V` é
um validador (ou uma tupla de validadores) de
`suprnova::http::upload::validators`.

```rust
use suprnova::{handler, json_response, MultipartRequest, Response};
use suprnova::http::upload::UploadedFile;
use suprnova::http::upload::validators::{ImageFile, MaxSize};

#[derive(MultipartRequest)]
pub struct AvatarUpload {
    #[field("avatar")]
    pub avatar: UploadedFile<(ImageFile, MaxSize<5_242_880>)>, // limite de 5 MiB
    #[field("caption")]
    pub caption: Option<String>,
}

#[handler]
pub async fn upload_avatar(form: AvatarUpload) -> Response {
    // `avatar` está em memória ou em um arquivo temporário, conforme o tamanho.
    // `.bytes()` lê qualquer um dos dois; `.store_as(...)` faz streaming para um disco.
    let bytes = form.avatar.bytes().await?;
    json_response!({ "size": bytes.len(), "caption": form.caption })
}
```

Formas de campo:

| Declaração | Forma na rede |
|---|---|
| `UploadedFile<V>` | arquivo obrigatório |
| `Option<UploadedFile<V>>` | arquivo opcional |
| `Vec<UploadedFile<V>>` | uploads em array (`photos[]`) |
| `String` / `u32` / qualquer `FromStr` | campo de texto (obrigatório) |
| `Option<String>` / `Option<T: FromStr>` | campo de texto opcional |
| `Vec<String>` / `Vec<T: FromStr>` | campos de texto repetidos |

Validadores embutidos em `suprnova::http::upload::validators`:

- `MaxSize<N>` - faz short-circuit na fronteira do byte quando o total
  acumulado excede `N` bytes (HTTP 413).
- `ImageFile` - rejeita partes cujos magic bytes não declarem `image/*`.
  (Nomeado a partir da própria regra do Laravel; o nome simples `Image`
  pertence ao pipeline de manipulação de imagens - veja
  [Imagens](images.md).)
- `MimeType<L>` - aceita uma allowlist fixa fornecida pelo seu próprio
  tipo `MimeAllowlist`.
- `()` - no-op; `UploadedFile<()>` aceita quaisquer bytes.

Validadores se compõem como tuplas: `(ImageFile, MaxSize<5_242_880>)` roda
os dois, fazendo short-circuit na primeira falha.

### Limites por campo e limites de array

O limite de bytes do corpo inteiro é global (8 MiB por padrão para
multipart, configurável via
`suprnova::http::upload::set_global_max_multipart_body_bytes`). Limites
por campo evitam o abuso em que um corpo com muitas partes pequenas faz
`Vec<UploadedFile<_>>` crescer sem limite dentro do orçamento de bytes:

```rust
#[derive(MultipartRequest)]
pub struct Gallery {
    #[field("photos", max_count = 8)]
    pub photos: Vec<UploadedFile<MaxSize<1_048_576>>>,
}
```

A (`max_count` + 1)-ésima parte com esse nome retorna HTTP 422 antes de
alocar, então a parte extra nunca chega a fazer o `Vec` crescer.

### Hooks de autorização e pós-validação

`MultipartRequest` espelha os hooks de `FormRequest` através da trait
`MultipartRequestHooks`. Por padrão o derive emite uma impl vazia; opte
pela sua própria com `#[multipart(custom_hooks)]`:

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

### Streaming para o armazenamento

`UploadedFile::store_as` escreve a parte em um disco de armazenamento
registrado. Para partes apoiadas em disco o caminho é inteiramente
streaming (chunks de 64 KiB via `opendal::Operator::writer`); partes em
memória usam uma única chamada de escrita. Use a extensão derivada do
conteúdo quando o caminho de armazenamento for endereçado pelo conteúdo -
o header com o nome do arquivo não é confiável:

```rust
use suprnova::Storage;

let disk = Storage::disk("avatars")?;
let path = format!("{}.{}", user.id, form.avatar.extension_from_magic());
form.avatar.store_as(&disk, &path).await?;
```

Veja [Sistema de arquivos e armazenamento](filesystem.md) para o
registro de discos de armazenamento.

## Organização de arquivos

A estrutura padrão para requests:

```
src/
├── requests/
│   ├── mod.rs                 # Re-exporta todos os requests
│   ├── create_user.rs         # CreateUserRequest
│   ├── update_user.rs         # UpdateUserRequest
│   └── create_post.rs         # CreatePostRequest
├── controllers/
│   └── user.rs                # Usa CreateUserRequest
└── routes.rs
```

**src/requests/mod.rs:**
```rust
pub mod create_user;
pub mod update_user;

pub use create_user::CreateUserRequest;
pub use update_user::UpdateUserRequest;
```

## Segurança de tipos ponta a ponta com Inertia

Requests também podem derivar `InertiaProps` para gerar tipos
TypeScript, habilitando segurança de tipos ponta a ponta do seu backend
Rust até o seu frontend React.

### Gerando tipos TypeScript para requests

Adicione o derive `InertiaProps` junto com `#[request]`:

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

Rode a geração de tipos:

```bash
suprnova generate-types
```

Isso gera tipos TypeScript em `frontend/src/types/inertia-props.ts`:

```typescript
export interface CreateTodoRequest {
  title: string
  description: string | null
}
```

### Formulários com tipos seguros no Inertia

Use o componente `<Form>` do Inertia para o tratamento de formulários
mais limpo:

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

Para mais controle, combine `<Form>` com o hook `useForm` e os seus tipos
gerados:

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

### O que o derive te dá

- O TypeScript pega erros de digitação em nomes de campo e
  incompatibilidades de tipo em tempo de compilação.
- O autocomplete da IDE lê o `.ts` gerado diretamente.
- Renomeie um campo em Rust, rode `suprnova generate-types` de novo, e a
  superfície TypeScript acompanha.

Veja [Tipos TypeScript](frontend-typescript-types.md) para o pipeline de
geração completo.

## Acessadores de solicitação

Além do padrão de formulário validado acima, o tipo `Request` carrega acessadores no estilo Laravel para inspecionar a solicitação em nível de rede - URL, headers, query string, negociação de conteúdo, metadados de rota e IP do cliente. Eles são úteis em middleware, em handlers que querem acesso cru junto com um `FormRequest`, e em qualquer lugar onde parse validado não é a ferramenta certa.

### URL e caminho

| Método | Retorna | Observações |
|--------|---------|-------|
| `req.path()` | `&str` | Caminho cru da URI. |
| `req.decoded_path()` | `String` | Caminho com os percent-escapes resolvidos. |
| `req.segments()` | `Vec<String>` | Caminho dividido em `/`, segmentos vazios descartados. |
| `req.segment(index, default)` | `Option<String>` | Acesso a segmento, com índice começando em 1. |
| `req.url()` | `String` | Esquema + host + caminho (sem query string). |
| `req.full_url()` | `String` | URL + query string. |
| `req.full_url_with_query(&[("k","v")])` | `String` | Acrescenta ou sobrescreve chaves de query. |
| `req.full_url_without_query(&["k"])` | `String` | Remove chaves de query. |

```rust
use suprnova::{handler, json_response, Request, Response};

#[handler]
pub async fn show(req: Request) -> Response {
    if req.is(&["admin/*"]) {
        // o caminho corresponde ao wildcard admin/*
    }
    json_response!({ "url": req.full_url() })
}
```

### Host, esquema, IP

| Método | Retorna | Ordem das fontes |
|--------|---------|--------------|
| `req.host()` | `Option<String>` | `X-Forwarded-Host` → `Host` → autoridade da URI. |
| `req.http_host()` | `Option<String>` | Host mais a porta, quando não for a padrão. |
| `req.scheme_and_http_host()` | `Option<String>` | `scheme://host:port`. |
| `req.scheme()` | `&'static str` | `"https"` quando [`secure`] é true, senão `"http"`. |
| `req.secure()` | `bool` | Esquema da URI → `X-Forwarded-Proto` → `X-Forwarded-Ssl: on`. |
| `req.ip()` | `Option<String>` | `X-Forwarded-For[0]` → `X-Real-IP` → endereço do peer. |
| `req.ips()` | `Vec<String>` | A chain completa: headers de proxy, depois o endereço do peer. |
| `req.user_agent()` | `Option<&str>` | Header `User-Agent`. |
| `req.port()` | `Option<u16>` | Porta do header Host → `X-Forwarded-Port` → porta da URI. |

### Headers e método

| Método | Retorna |
|--------|---------|
| `req.has_header("X-Foo")` | `bool` |
| `req.bearer_token()` | `Option<String>` (a última substring `Bearer `, com as vírgulas aparadas) |
| `req.is_method("POST")` | `bool` (insensível a maiúsculas e minúsculas) |
| `req.ajax()` | `X-Requested-With: XMLHttpRequest` |
| `req.pjax()` | Header `X-PJAX` com valor verdadeiro |
| `req.prefetch()` | `X-Moz`, `Purpose`, ou `Sec-Purpose` = `prefetch` |

### Negociação de conteúdo

```rust
if req.is_json() { /* o Content-Type carrega /json ou +json */ }
if req.expects_json() { /* AJAX sem restrição de Accept, ou Accept prefere JSON */ }
if req.wants_json() { /* o header Accept encabeça com JSON */ }
if req.accepts_html() { /* o Accept permite text/html */ }

let preferred = req.prefers(&["application/json", "text/html"]);
let acceptable = req.acceptable_content_types();
```

`accepts(&[ty])` corresponde tanto a tipos simples quanto a sufixos no estilo `application/<vendor>+json`. `accepts_any_content_type()` retorna true quando não há header Accept ou quando a preferência principal é `*/*`.

### Query string

```rust
let id: Option<String> = req.query_param("id");
let present: bool = req.has_query("id");
let map = req.query_params(); // HashMap<String, String>

// Parse tipado da query via serde
#[derive(serde::Deserialize)]
struct SearchQuery { page: u32, q: String }
let q: SearchQuery = req.query_into()?;
```

### Metadados de rota

Depois que o router despacha uma solicitação, o padrão correspondido fica registrado na solicitação:

```rust
if req.route_is(&["users.show", "users.*"]) {
    // estamos dentro da rota users.show ou users.*
}

let pattern = req.route_pattern(); // Some("/users/{id}")
let name = req.route_name();       // Some("users.show")
```

`route_is(&[...])` aceita wildcards `*` (a semântica de `Str::is` do Laravel).

## Abortando cedo

Para tratamento de erro com saída antecipada sem o envelope `Response` completo, os helpers `abort_with` / `abort_if` / `abort_unless` retornam um `FrameworkError` que é renderizado pelo pipeline padrão `From<FrameworkError> for HttpResponse`. Eles se compõem com `?` diretamente:

```rust
use suprnova::{abort_if, abort_unless, abort_with, handler, json_response, Request, Response};

#[handler]
pub async fn show(req: Request) -> Response {
    let id = req.param("id")?;

    // 404 quando o recurso está faltando.
    abort_if(id == "0", 404, "User not found")?;

    // 403 quando o chamador não está autenticado.
    abort_unless(req.has_header("Authorization"), 403, "Login required")?;

    // Ou levante um status incondicionalmente:
    if some_condition() {
        return Err(abort_with(418, "I'm a teapot").unwrap_err().into());
    }

    json_response!({ "id": id })
}
```

`abort_if` / `abort_unless` retornam `Ok(())` quando a condição é falsa, então o `?` segue normalmente.

## Por que Suprnova diverge

O Laravel expõe um input bag síncrono e mesclado - `$req->input('field')`,
`$req->all()`, `$req->only(['a','b'])`, `$req->boolean('flag')` - puxado
da query string e do corpo parseado juntos. O Suprnova não entrega essa
superfície. A razão:

- O corpo do Suprnova é consumido uma única vez e é async. Um `all()`
  síncrono exigiria bufferizar todo corpo de antemão para satisfazer um
  método que a maioria dos handlers nunca chama - a superfície de
  memória e de DoS é diferente do ciclo de vida de um processo por
  solicitação do PHP.
- A alternativa tipada (`#[request]` + `FormRequest`) dá nomes de campo
  verificados em tempo de compilação, validação e parse ciente do
  content type - exatamente a rede de segurança que o bag sem tipos não
  tem.

Para inspeção de query / header / rota, recorra a `query_param`,
`query_into`, `has_query`, `bearer_token`, e os leitores de header
acima. Para acesso ao lado do corpo, defina um struct `#[request]` ou um
extractor `#[derive(MultipartRequest)]`.

## Próximos passos

- [Validação](validation.md) - a biblioteca de regras por trás de
  `#[validate(...)]` e o formato do conjunto de erros 422
- [Respostas](responses.md) - construindo valores `HttpResponse` de
  volta a partir do seu handler, incluindo streaming e redirecionamentos
- [Tratamento de erros](errors.md) - padrões de handler construídos
  sobre o fato de `Response` ser `Result<HttpResponse, HttpResponse>`
- [Roteamento](routing.md) - registrando rotas e os parâmetros `{id}`
  que `req.param("id")` lê
- [Autenticação](authentication.md) - `Auth::user_as`, `Auth::attempt`,
  e os guards que resolvem o usuário atual a partir da solicitação
- [Sistema de arquivos e armazenamento](filesystem.md) - registrando os
  discos de armazenamento em que `UploadedFile::store_as` escreve
