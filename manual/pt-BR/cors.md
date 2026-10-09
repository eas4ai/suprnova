# CORS

`CorsMiddleware` responde a solicitações `OPTIONS` de preflight e decora
respostas cross-origin comuns com headers `Access-Control-Allow-*`. Você
o instala uma vez em `bootstrap()` quando um navegador em uma origem
diferente chama sua API - APIs públicas, uma SPA hospedada em outro
domínio, um webview mobile, ou um site de documentação hospedado
separadamente. Apps same-origin (Inertia servido a partir do mesmo host
que o backend, o padrão do Suprnova) não precisam de CORS de jeito
nenhum. O middleware espelha o `HandleCors` e o `config/cors.php` do
Laravel, mas como um builder tipado sobre `CorsConfig`.

## Instale-o globalmente

```rust,ignore
use std::time::Duration;
use suprnova::{global_middleware, CorsConfig, CorsMiddleware};

pub fn register() {
    global_middleware!(CorsMiddleware::new(
        CorsConfig::allow_origins(["https://app.example"])
            .allow_credentials(true)
            .max_age(Duration::from_secs(600)),
    ));
}
```

Um preflight é uma solicitação `OPTIONS` com um header
`Access-Control-Request-Method`. O router não tem rotas `OPTIONS`, então
um preflight nunca *corresponde* a uma rota - mas o servidor do Suprnova
executa a chain de middleware global em solicitações não correspondidas
(terminando em um 404), então um `CorsMiddleware` instalado globalmente
enxerga o preflight e faz short-circuit dele com `204` antes que o 404
chegue a ser produzido. **É por isso que o CORS precisa ser instalado
globalmente, e não por rota.**

## Escolhendo uma política de origem

Intencionalmente não existe um `Default` para `CorsConfig`. Uma política
permissiva por reflexo é uma armadilha de segurança, então você precisa
escolher:

| Builder | Comportamento |
| --- | --- |
| `CorsConfig::allow_origins([...])` | Allowlist fixa. A origem só é ecoada de volta quando corresponde exatamente a uma entrada. |
| `CorsConfig::any_origin()` | Wildcard `*`. Com credentials habilitados, o middleware ecoa a origem específica da solicitação em vez de `*` (a combinação `*` + credentials é inválida segundo a spec Fetch). |
| `.allow_origin_patterns([...])` | Padrões de regex somados por cima da lista literal. Úteis para subdomínios dinâmicos. |

```rust,ignore
CorsConfig::allow_origins(["https://app.example"])
    .allow_origin_patterns([r"^https://[a-z0-9-]+\.staging\.example$"])
```

Os padrões são ancorados automaticamente - `^` e `$` são acrescentados
no início / no fim quando ausentes, então uma correspondência parcial
contra uma URL de redirecionamento como
`https://evil.com/?u=https://app.example` não tem como vazar.

Uma regex inválida faz panic em tempo de config (no boot), não em tempo de
solicitação - é melhor expor o bug de config de forma evidente do que
falhar em fail-open silenciosamente.

`allowed_origins_patterns` (o alias com nome do Laravel) também está
disponível.

## Restringindo quais caminhos recebem CORS

A config `cors.php` do Laravel tem um array `paths` (`['api/*',
'sanctum/csrf-cookie']`) que limita a aplicação de CORS a padrões de URL
específicos. O Suprnova espelha isso:

```rust,ignore
CorsConfig::allow_origins(["https://app.example"])
    .paths(["api/*", "sanctum/csrf-cookie"])
```

Sem nenhum `paths` definido, o CORS executa em toda solicitação (o
padrão do Suprnova - já que o middleware é opt-in pelo registro). Com ao
menos um padrão definido, só as solicitações correspondentes recebem
tratamento de CORS (tanto os preflights **quanto** a decoração da
resposta real); todo o resto passa intocado.

Os padrões usam a semântica de `Str::is` do Laravel: `*` é um wildcard
multi-segmento, guloso através de `/`. A `/` inicial é normalizada, então
`"api/*"` e `"/api/*"` são equivalentes.

```rust,ignore
"api/*"             // corresponde a /api/users, /api/users/42
"api/*/posts"       // corresponde a /api/v2/posts, /api/v1/posts
"sanctum/csrf-cookie" // literal de correspondência exata
"*"                 // corresponde a tudo
```

## Pulando via predicado

Para predicados sobre a forma da solicitação que não cabem em um padrão
de caminho (pular com base em um header, executar CORS só em produção,
pular durante health checks), use `skip_when`:

```rust,ignore
CorsConfig::any_origin()
    .skip_when(|req| req.header("X-Internal-Call").is_some())
    .skip_when(|req| req.path() == "/healthz")
```

Espelha o `HandleCors::skipWhen(Closure)` do Laravel, mas vive na
política em vez de ser estado global mutável. Múltiplos callbacks
`skip_when` podem ser registrados; basta um retornar `true` para o CORS
ser pulado.

## Métodos, headers, headers expostos

```rust,ignore
CorsConfig::allow_origins(["https://app.example"])
    .methods(["GET", "POST", "DELETE"])           // padrão = GET/POST/PUT/PATCH/DELETE/OPTIONS/HEAD
    .allow_headers(["Content-Type", "X-CSRF-TOKEN"])  // restringe; padrão = refletir a solicitação
    .allow_any_headers()                          // o "reflita o que foi pedido" explícito
    .expose_headers(["X-Total-Count", "Link"])    // headers que o JS pode ler na resposta
```

Aliases com os nomes do Laravel (para que quem vem do `cors.php`
encontre o que espera):

- `allowed_methods(...)` ≡ `methods(...)`
- `allowed_headers(...)` ≡ `allow_headers(...)`
- `exposed_headers(...)` ≡ `expose_headers(...)`
- `allowed_origins_patterns(...)` ≡ `allow_origin_patterns(...)`
- `supports_credentials(...)` ≡ `allow_credentials(...)`

## Credentials e `*`

Segundo a spec Fetch, `Access-Control-Allow-Origin: *` é inválido junto
com credentials - o navegador rejeita a resposta. Com uma lista
explícita de origens (`allow_origins([...])`) mais
`allow_credentials(true)`, o middleware ecoa o `Origin` específico da
solicitação em vez de `*`, e a política funciona como esperado.

**`any_origin() + allow_credentials(true)` faz panic em tempo de
build.** A combinação é um bypass completo do allowlisting de origem:
qualquer página de um atacante consegue fazer solicitações cross-origin
com credentials e ler as respostas. Em vez de emitir o header errado em
tempo de execução, o construtor da política falha de forma explícita para
que a configuração incorreta nunca chegue a um deploy em execução. Use uma
allowlist explícita:

```rust,ignore
// CORRETO - allowlist explícita com credentials.
CorsConfig::allow_origins(["https://app.example"]).allow_credentials(true)
// → em uma solicitação com Origin: https://app.example
// → resposta: Access-Control-Allow-Origin: https://app.example
//             Access-Control-Allow-Credentials: true

// REJEITADO em tempo de build - faz panic com uma mensagem de remediação.
// CorsConfig::any_origin().allow_credentials(true)
```

## Max-age

```rust,ignore
.max_age(Duration::from_secs(600))   // tipado
.max_age_secs(600)                   // segundos inteiros, no estilo Laravel
```

`Access-Control-Max-Age` diz ao navegador por quanto tempo ele pode
manter o resultado do preflight em cache. Mais alto = menos round-trips
de preflight, e mudanças de política demoram mais para se propagar.

## O que o middleware realmente emite

### Preflight (`OPTIONS` + `Access-Control-Request-Method`)

Se a origem for permitida:

```
HTTP/1.1 204 No Content
Access-Control-Allow-Origin: <origin>
Access-Control-Allow-Credentials: true        // quando credentials estão habilitados
Access-Control-Allow-Methods: GET, POST, ...
Access-Control-Allow-Headers: <reflected or fixed>
Access-Control-Max-Age: 600                   // quando definido
Vary: Origin, Access-Control-Request-Method, Access-Control-Request-Headers
```

Se a origem não for permitida: um `204` nu + `Vary` (sem nenhum
`Access-Control-*`). A verificação de header ausente feita pelo navegador
é o que produz o erro de CORS - a mesma convenção do `tower-http`.

### Resposta cross-origin real

Quando a solicitação traz um header `Origin` e a origem é permitida:

```
Access-Control-Allow-Origin: <origin or *>
Access-Control-Allow-Credentials: true        // quando habilitado
Access-Control-Expose-Headers: X-Total, Link  // quando configurado
Vary: Origin                                  // só quando não for "*"
```

Um ACAO `*` é idêntico para toda origem, então nenhum `Vary` é
necessário; uma origem específica varia por origem, então caches
compartilhados precisam chaveá-la.

## Testando handlers de CORS

O CORS é imposto do lado do navegador - o servidor executa o handler
mesmo quando a origem não é permitida; ele só não decora a resposta.
Esse é o comportamento testável:

```rust,ignore
let (status, headers, body) = request_with_origin(
    "/api/data",
    "https://app.example",
).await;
assert_eq!(status, 200);
assert_eq!(
    headers.get("access-control-allow-origin"),
    Some(&"https://app.example".to_string()),
);
```

Para uma origem não permitida, o handler executa e o corpo volta, mas é
a ausência de `Access-Control-Allow-Origin` que impede o navegador de
lê-lo.

## Matriz de paridade do Laravel

| `cors.php` do Laravel | Builder do Suprnova |
| --- | --- |
| `paths` | `.paths([...])` |
| `allowed_methods` | `.methods([...])` / `.allowed_methods([...])` |
| `allowed_origins` | `CorsConfig::allow_origins([...])` |
| `allowed_origins_patterns` | `.allow_origin_patterns([...])` / `.allowed_origins_patterns([...])` |
| `allowed_headers` | `.allow_headers([...])` / `.allowed_headers([...])` |
| `exposed_headers` | `.expose_headers([...])` / `.exposed_headers([...])` |
| `max_age` | `.max_age(Duration)` / `.max_age_secs(u64)` |
| `supports_credentials` | `.allow_credentials(bool)` / `.supports_credentials(bool)` |
| `HandleCors::skipWhen(closure)` | `.skip_when(\|req\| ...)` |

O middleware é registrado globalmente, em vez do "instalado
automaticamente para `paths`" do estilo Laravel - a chain de middleware
do Suprnova é explícita, veja [Middleware](middleware.md) para o design.

### Por que Suprnova diverge

O `HandleCors` do Laravel é anexado automaticamente ao kernel e lê sua
política de `config/cors.php`. Essa forma funciona para PHP porque o
array de config é o único lugar em que um framework de uma solicitação
por processo consegue compartilhar configuração sem reavaliá-la a cada
solicitação. O Suprnova expõe as mesmas opções como um builder
`CorsConfig` tipado que você registra explicitamente com
`global_middleware!`, o que mantém a chain de middleware visível em
`bootstrap()` e deixa o compilador impor a escolha entre allowlist e
wildcard (não há `Default` para `CorsConfig`, então você não tem como
entregar sem querer um `Access-Control-Allow-Origin: *` por ter
esquecido de preencher um valor de config).

A outra divergência é que os preflights alcançam o middleware mesmo em
caminhos sem rota. O Laravel roteia `OPTIONS` pelo seu router, de modo
que o preflight corresponde a uma rota `OPTIONS` (registrada
automaticamente para cada rota REST). O router do Suprnova não tem rotas
`OPTIONS`; em vez disso o servidor executa a chain de middleware global
em solicitações não correspondidas antes de retornar 404, então um
`CorsMiddleware` instalado globalmente faz short-circuit do preflight
com `204` antes que o caminho de não-encontrado seja tomado. É por isso
que o CORS *precisa* ser instalado globalmente - um registro por rota
nunca veria o preflight.

## Próximos passos

- [Middleware](middleware.md) - a trait, a chain, registro global vs
  por rota, hooks termináveis
- [CSRF](csrf.md) - o outro middleware global que a maioria das apps
  instala junto com o CORS
- [Roteamento](routing.md) - como rotas são correspondidas (e por que
  preflights não correspondem), mais o caminho sem fallback em que a
  chain global executa
- [Ciclo de vida da solicitação](lifecycle.md) - onde o CORS fica na
  chain em relação à sessão, ao CSRF e ao handler
- [Configuração](configuration.md) - padrões de config tipada para
  middleware que precisam de configurações vindas do ambiente
