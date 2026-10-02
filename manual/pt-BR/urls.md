# Geração de URLs

URLs são como seu app referencia a si mesmo - todo redirecionamento, todo
link de email, todo href de `<Link>` do Inertia, todo download assinado
tem que vir de algum lugar. Escrever caminhos na mão torna as refatorações
dolorosas e as renomeações de rota inseguras. O Suprnova traz um pequeno
namespace `url::` e um helper irmão `route()` que recebem um nome mais
parâmetros e devolvem uma string, com o percent-encoding já resolvido,
cunhagem de assinatura disponível, e verificação que bate byte a byte com
o formato de rede do Laravel.

Este capítulo é a referência da superfície de geração de URL. O capítulo
[Roteamento](routing.md) cobre como declarar rotas e nomeá-las; este
cobre o que você faz com esses nomes depois.

```rust
use suprnova::{route, url};

// Lookup por nome → URL
let profile = route("users.show", &[("id", "42")]).unwrap();
//   "/users/42"

// URL absoluta contra APP_URL
let absolute = url::to("/dashboard");
//   "https://app.test/dashboard"

// Link assinado para reset de senha
let link = url::signed_route("password.reset", &[("token", reset_token)])?;
//   "/password/reset/xyz?signature=ab12..."

// Verifica na solicitação de entrada
if url::has_valid_signature(&request)? {
    // aja sobre ela
}
```

Tudo neste capítulo é reexportado sob `suprnova::url::*` e
`suprnova::route`, para que o código consumidor nunca precise alcançar o
módulo de roteamento diretamente.

## Rotas nomeadas

Um nome é um rótulo de string anexado a uma rota no momento do registro.
Depois que um nome existe, `route(name, params)` o resolve de volta para
um padrão de URL e substitui os parâmetros. Os nomes vivem em um único
registro process-global - há uma tabela `name → path` por binário em
execução, não uma por `Router`.

```rust
use suprnova::{routes, get, post};

routes! {
    get!("/", controllers::home::index).name("home"),
    get!("/users/{id}", controllers::users::show).name("users.show"),
    post!("/users", controllers::users::store).name("users.store"),
}
```

A chamada `.name(...)` registra `"users.show" → "/users/{id}"`. A partir
daí, qualquer ponto do processo consegue resolver o nome:

```rust
use suprnova::route;

let url = route("users.show", &[("id", "42")]);
// Some("/users/42")

let missing = route("does.not.exist", &[]);
// None
```

Registrar de novo o mesmo par `(name, path)` é idempotente - útil quando
o registro de rotas roda mais de uma vez durante a inicialização.
Registrar um nome sob um caminho *diferente* causa panic; essa colisão é
um bug com formato de segurança, porque helpers como `Redirect::route`
apontariam silenciosamente para o lado que vencesse a corrida.

### Os helpers de lookup

| Função | Retorna | Quando a rota não existe |
|---|---|---|
| `route(name, params)` | `Option<String>` | `None` |
| `route_with_params(name, params_map)` | `Option<String>` | `None` |
| `try_route(name, params)` | `Result<String, RouteUrlError>` | `Err(NameNotFound)` |
| `try_route_with_params(name, params_map)` | `Result<String, RouteUrlError>` | `Err(NameNotFound)` |

O par leniente `route` / `route_with_params` deixa qualquer segmento
`{placeholder}` não preenchido literalmente na saída - tudo bem para logs
de debug, inseguro para entregar a um navegador. O par estrito
`try_route` / `try_route_with_params` retorna
`RouteUrlError::MissingParams { name, missing }` listando os placeholders
não preenchidos, para que o chamador possa falhar de forma explícita em
vez de redirecionar um usuário para `/users/{id}`.

```rust
use suprnova::routing::{try_route, RouteUrlError};

match try_route("users.show", &[]) {
    Ok(url) => /* seguro para redirecionar */,
    Err(RouteUrlError::MissingParams { name, missing }) => {
        // missing == vec!["id"]
        return Err(FrameworkError::internal(
            format!("cannot build URL for {name}: missing {missing:?}"),
        ));
    }
    Err(RouteUrlError::NameNotFound(name)) => {
        return Err(FrameworkError::internal(format!("unknown route: {name}")));
    }
}
```

`Redirect::route` usa `try_route_with_params` por baixo dos panos
exatamente por esse motivo - um redirecionamento com um `{id}` cru no
header `Location` seria pior do que falhar.

### O percent-encoding é automático

Valores de parâmetro são codificados segundo as regras de segmento de
caminho da RFC 3986 antes de serem substituídos. Isso cobre os gen-delims
e os sub-delims (`/ ? # [ ] @ ! $ & ' ( ) * + , ; =`), os caracteres de
controle, o espaço, e o próprio `%`. Caracteres não reservados
(`A-Z a-z 0-9 - _ . ~`) passam sem alteração.

```rust
use suprnova::route;

// Um slug que contém uma barra fica contido em um único segmento:
route("posts.show", &[("slug", "hello/world")]);
// Some("/posts/hello%2Fworld")

// Tentativas de path traversal não conseguem escapar do segmento:
route("users.show", &[("id", "../../etc/passwd")]);
// Some("/users/..%2F..%2Fetc%2Fpasswd")

// Unicode de verdade passa intocado:
route("users.show", &[("id", "user-é-42")]);
// Some("/users/user-%C3%A9-42")
```

O lado da correspondência preserva esse round-trip - uma solicitação para
`/posts/hello%2Fworld` corresponde à rota `/posts/{slug}` e um handler
que lê `req.param("slug")` vê `"hello/world"`, decodificado. Codifique na
fronteira, decodifique na fronteira; nunca veja os bytes crus no código
do handler.

### Lookup reverso

Quando você tem um padrão de rota correspondido e quer o nome registrado -
por exemplo para logging ou para verificações
`Request::route_is("users.show")` - use `route_name_for_pattern`:

```rust
use suprnova::routing::route_name_for_pattern;

let name = route_name_for_pattern("/users/{id}");
// Some("users.show")
```

Isso é uma varredura O(n) sobre o registro de nomes. n é o número de
nomes registrados; mesmo com contagens de rota de quatro dígitos o custo
é desprezível comparado ao ciclo de vida da solicitação em volta. A
função é exposta para ferramentas e middleware - `Request::route_is` já a
chama por você quando você compara com uma rota nomeada dentro de um
handler.

## URLs absolutas

Para todo o resto - montar emails, compartilhar URLs, enviar metadados
Open Graph - você quer uma URL absoluta com o esquema e o host certos.
`url::to` junta um caminho a `APP_URL`:

```rust
use suprnova::url;

// No env: APP_URL=https://app.example.com
let url = url::to("/about");
// "https://app.example.com/about"

// URLs que já são absolutas passam sem alteração:
let cdn = url::to("https://cdn.example/asset.js");
// "https://cdn.example/asset.js"

let proto_relative = url::to("//cdn.example/asset.js");
// "//cdn.example/asset.js"
```

O host, o esquema e a porta vêm todos de `APP_URL`. Se `APP_URL` for
`http://localhost:8765`, então `url::to("/foo")` produz
`"http://localhost:8765/foo"`. A barra final de `APP_URL` é normalizada
e removida, para que você nunca termine com `https://host//path`.

### Forçando HTTPS

`url::secure(path)` constrói a mesma URL absoluta, mas eleva o esquema
para `https://` mesmo que `APP_URL` seja `http://`:

```rust
use suprnova::url;

// No env: APP_URL=http://app.example.com
url::secure("/login");
// "https://app.example.com/login"
```

Em produção você tipicamente define `APP_URL` para o seu host HTTPS uma
vez e nunca chama `secure` diretamente - a elevação é para ambientes em
que o desenvolvimento local roda sobre HTTP mas um link específico
precisa ser HTTPS (por exemplo, uma URL de callback embutida em uma
sessão de pagamento).

### Lendo a URL atual

Dentro de um handler, a própria solicitação é a fonte da verdade:

```rust
use suprnova::url;

async fn breadcrumbs(req: Request) -> Response {
    let here = url::current(&req);       // "/posts/42?expand=author"
    let full = url::full(&req);          // "https://app.test/posts/42?expand=author"
    let back = url::previous("/");        // URL anterior registrada pela sessão
    // ...
}
```

| Helper | Retorna | Fonte |
|---|---|---|
| `url::current(&req)` | caminho + query desta solicitação | O `Request` atual |
| `url::full(&req)` | URL absoluta desta solicitação | `APP_URL` + `current(&req)` |
| `url::previous(fallback)` | a URL anterior registrada pelo middleware de sessão | `_previous.url` na sessão, ou `fallback` |

`previous` é o que sustenta `Redirect::back` - o middleware de sessão
registra a URL de todo GET HTML bem-sucedido, para que um `POST` de
formulário consiga voltar para a página que o enviou. Partials do
Inertia, solicitações JSON-API (`Accept: application/json` sem
`text/html`) e respostas que não são 2xx/3xx são puladas, para que você
nunca volte a um endpoint intermediário que o usuário nunca viu. O
middleware também se recusa a registrar uma URL que não seja relativa à
raiz e de mesma origem: um caminho de solicitação no formato `//host` ou
`/\host` (ambos lidos por um navegador como relativos ao protocolo, não
como path), ou que carregue um byte de controle ASCII em qualquer ponto
(um `TAB` ou nova linha que o analisador de URLs de um navegador remove
antes de comparar origens, transformando o que parece um path seguro em
uma das duas formas acima), nunca é armazenado - e a mesma checagem é
executada novamente em toda leitura, portanto um valor armazenado por
uma versão anterior também continua falhando em vez de ser confiado
somente por já estar na sessão. De qualquer forma, `previous` e
`Redirect::back` não podem ser direcionados para fora da origem por um
caminho incomum que alcance sua aplicação, passado ou presente.

## URLs assinadas

URLs assinadas deixam você cunhar uma URL que prova ter vindo do seu
servidor, sem armazenar a URL em lugar nenhum. A assinatura é um
HMAC-SHA256 sobre a forma canônica da URL usando o seu `APP_KEY`; o
servidor recalcula o HMAC na solicitação de entrada e aceita apenas
assinaturas que batem.

Recorra a URLs assinadas quando houver:

- **Links entregues por email** - reset de senha, verificação de email,
  convite por email, login por magic link. A URL tem que sobreviver a uma
  ida e volta por uma caixa de entrada sem poder ser guardada como estado
  opaco.
- **Downloads efêmeros** - links de "sua exportação CSV está pronta" que
  expiram em 24 horas, alternativas ao S3 assinado em que você quer que a
  URL continue no seu domínio.
- **Webhooks que apontam de volta para você** - callbacks de terceiros
  que devem recusar chamadas forjadas sem exigir um lookup de banco de
  dados por solicitação.

```rust
use suprnova::url;
use chrono::Utc;

// URL assinada permanente - nunca expira.
let link = url::signed_route(
    "password.reset",
    &[("user", user_id), ("token", token)],
)?;
// "/password/reset/42/xyz?signature=ab12cd34..."

// URL assinada temporária - expira daqui a uma hora.
let expires_at = Utc::now().timestamp() + 3600;
let link = url::temporary_signed_route(
    "verify.email",
    &[("user", user_id)],
    expires_at,
)?;
// "/verify/email/42?expires=1748803600&signature=def012..."
```

Repare que `expires_at_epoch_seconds` é um **timestamp UNIX absoluto**,
não uma duração. Calcule-o no local da chamada:

```rust
let one_hour_from_now = chrono::Utc::now().timestamp() + 3600;
let one_day_from_now  = chrono::Utc::now().timestamp() + 86_400;
```

Isso mantém a assinatura do helper pequena e deixa você reutilizar a
mesma função tanto para prazos relativos a agora quanto para prazos
absolutos explícitos.

### Verificando

No lado de entrada, você verifica a assinatura contra a solicitação viva:

```rust
use suprnova::{url, FrameworkError, Request, Response, HttpResponse};

pub async fn reset(req: Request) -> Response {
    reset_inner(req).await.map_err(HttpResponse::from)
}

async fn reset_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    if !url::has_valid_signature(&req)? {
        return Err(FrameworkError::forbidden("Invalid or expired link"));
    }
    // A assinatura está boa e não expirou - siga em frente.
    let user_id = req.param("user").unwrap();
    // ...
    Ok(HttpResponse::text("ok"))
}
```

`has_valid_signature` retorna `true` apenas quando o HMAC bate E a URL
não está expirada. Para a distinção de três vias entre *inválida*,
*expirada* e *válida*, use `signature_verdict`:

```rust
use suprnova::{url, FrameworkError, HttpResponse, Request, Response};
use suprnova::routing::SignatureVerdict;

pub async fn reset(req: Request) -> Response {
    reset_inner(req).await.map_err(HttpResponse::from)
}

async fn reset_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    match url::signature_verdict(&req)? {
        SignatureVerdict::Valid => {
            // Siga em frente.
        }
        SignatureVerdict::Expired => {
            // Mande o usuário para uma página que explica que o link
            // expirou e oferece o envio de um novo.
            return Ok(HttpResponse::new()
                .status(302)
                .header("Location", "/password/reset-expired"));
        }
        SignatureVerdict::Invalid => {
            // Renderize um 403 genérico - não vaze se a assinatura estava
            // malformada, ausente, ou apenas errada.
            return Err(FrameworkError::forbidden("Invalid link"));
        }
    }
    // ...
    Ok(HttpResponse::text("ok"))
}
```

`signature_has_not_expired(&req)` está deprecada e agora responde
exatamente o que `has_valid_signature` responde. Recorra ao
`signature_verdict` acima; uma URL sem o parâmetro de query `expires` é
"nunca expirada" por definição, no Suprnova como no Laravel.

### Por que Suprnova diverge

O `URL::signatureHasNotExpired($request)` do Laravel é literalmente
"não expirada", então uma assinatura **forjada** volta como `true` - ela
nunca teve um prazo para perder. O do Suprnova costumava corresponder a
isso. Não corresponde mais: o helper exige antes uma assinatura válida.

A razão é que `expires` é fornecido pelo atacante até o HMAC dizer o
contrário, então nenhuma resposta derivada dele significa coisa alguma
antes de a assinatura conferir - e uma função cujo nome soa como uma
proteção estava deixando toda URL forjada passar por qualquer coisa que a
chamasse sozinha.

Exigir validade a colapsa em `has_valid_signature`, que é o motivo de ela
carregar uma deprecação em vez de uma flag de comportamento. Esse
colapso não é uma perda: sob um veredito de três estados não existe
nenhum "não expirada" que um único `bool` possa relatar honestamente
além de `Valid`. Se você quer distinguir *expirada* de *inválida* - para
dizer "solicite um link novo" em vez de "proibido" - é para isso que
existe `signature_verdict`, e ele diz isso no tipo.

### Assinando URLs arbitrárias

Se a URL que você quer assinar não vem de uma rota nomeada registrada -
uma URL de callback entregue por um terceiro, um caminho construído
dinamicamente em runtime - use `signed_url` diretamente:

```rust
use suprnova::url;

let callback = url::signed_url(
    "/webhooks/stripe/callback?order=42",
    Some(chrono::Utc::now().timestamp() + 600),  // expira em 10 minutos
)?;
```

Passe `None` na expiração para cunhar uma assinatura permanente. O lado
da verificação é o mesmo - `has_valid_signature(&req)` não se importa se
a URL foi cunhada a partir de uma rota nomeada ou de um caminho cru.

### Formato na rede

Duas URLs que diferem apenas na ordem dos parâmetros de query produzem
assinaturas idênticas, porque a forma canônica ordena os pares de query
lexicograficamente antes de calcular o hash. Isso importa porque clientes
às vezes reordenam parâmetros de query em trânsito (proxies, geradores de
preview de link, apps de email em celular), e uma URL assinada que
quebrasse sob reordenação seria inutilizável.

| Componente | Valor |
|---|---|
| Algoritmo | HMAC-SHA256 |
| Chave | Bytes crus da `APP_KEY` ativa |
| Payload | `path?<sorted-query>` (omite o `?` quando não há params) |
| Ordem de ordenação | `(key, value)` - todo par, repetições incluídas |
| Codificação | Digest de 64 caracteres codificado em hex |
| Comparação | Tempo constante via `subtle::ConstantTimeEq` |
| Chaves reservadas | `signature`, `expires` |

**Chaves repetidas são assinadas, não colapsadas.** `?tag=a&tag=b` leva
os dois valores para o payload, então nenhum deles pode ser adicionado,
removido ou substituído sem quebrar a assinatura. Ordenar por
`(key, value)` em vez de só pela chave é o que mantém essa ordem total,
então a garantia de reordenação acima continua valendo quando uma chave
aparece mais de uma vez.

Vale dizer isso porque a alternativa morde forte. Uma versão anterior
canonicalizava para um mapa, que guardava apenas o último valor de uma
chave repetida. `Request::query_param` retornava o *primeiro*. Então um
`?user=victim` legitimamente assinado podia ser reproduzido como
`?user=attacker&user=victim` com a assinatura original: a verificação via
`victim` e passava, e o handler agia sobre `attacker`. A URL assinada e a
URL executada eram diferentes. Os três acessadores de query -
`query_param`, `query_params` e `Context::query_param` - agora resolvem
uma chave repetida para o seu último valor, e a forma canônica não perde
nada.

Um `signature` ou `expires` repetido é recusado de imediato. Esses são
parâmetros de controle; duas cópias de qualquer um deles não deixam
nenhuma resposta não arbitrária para "qual delas vale?", e o verificador
não deveria ser o componente que adivinha.

O payload do HMAC exclui qualquer parâmetro de query `signature`
preexistente (então assinar sobre assinatura é um no-op) e reemite um
valor `expires` novo a partir dos argumentos da chamada. Um cliente que
remove ou reescreve o `expires` quebra a assinatura; um cliente que
remove o `signature` falha como `Invalid`. Os dois são fail-closed.

O fragmento (`#section`) é removido da forma canônica porque navegadores
nunca transmitem fragmentos de volta ao servidor. Assinar sobre um
fragmento invalidaria todo link no instante em que um cliente
acrescentasse uma âncora - `?signature=...#docs` não passaria na
verificação do lado do servidor.

### Parâmetros de query reservados

`signature` e `expires` são nomes reservados de parâmetro de query. Uma
rota que legitimamente espere um parâmetro de query chamado `signature`
ou `expires` colidiria com a maquinaria de URLs assinadas, e o
verificador atribuiria o valor ao lugar errado. Ou renomeie o parâmetro,
ou coloque os parâmetros de entrada da rota sob um namespace diferente.

```rust
// Ruim - `signature` colide com o nome reservado.
get!("/api/check", check)  // recebe ?signature=hash

// Bom - coloque em um namespace próprio.
get!("/api/check", check)  // recebe ?body_signature=hash
```

As constantes são expostas por simetria com o formato de rede do Laravel:

```rust
use suprnova::routing::{SIGNATURE_KEY, EXPIRES_KEY};
// SIGNATURE_KEY == "signature"
// EXPIRES_KEY   == "expires"
```

### Rotação de chaves

URLs assinadas usam o mesmo `APP_KEY` que sustenta `Crypt::encrypt` e a
integridade do cookie de sessão. Rotacionar `APP_KEY` invalida toda
assinatura já cunhada que ainda esteja em trânsito - um email de reset de
senha em trânsito vira um 403 na próxima vez que o usuário clicar nele.

Para a maioria das aplicações esse é o comportamento correto. Se você
precisa de uma rotação suave com sobreposição (para que links antigos
continuem funcionando durante uma janela de implantação), use
`APP_KEY_PREVIOUS` para levar a chave anterior adiante; o chaveiro tenta
toda chave instalada na verificação. Veja o capítulo
[Hashing](hashing.md) para a explicação completa do chaveiro.

## Erros e casos de borda

Vale conhecer alguns modos de falha:

- **`route(name, ...)` retorna `None`** quando o nome não está
  registrado. Esta é a superfície leniente - a falha silenciosa é
  intencional, para que o código chamador possa cair para um padrão. Use
  `try_route` para uma falha explícita.
- **`try_route` retorna `Err(NameNotFound)`** para um nome desconhecido e
  `Err(MissingParams { name, missing })` quando um `{placeholder}`
  obrigatório não tem valor correspondente.
- **`url::signed_route` e afins retornam `FrameworkError`** quando a
  chave de criptografia não está instalada (por exemplo, você esqueceu o
  `APP_KEY` no `.env`). Isso falha na inicialização em produção, porque
  `Crypt::init` roda durante `Server::from_config`; o caminho de erro
  aqui existe para expor uma má configuração de forma evidente em vez
  de produzir links não verificáveis.
- **`has_valid_signature` retorna `Ok(false)`**, não `Err`, para uma
  assinatura inválida ou expirada. A variante `FrameworkError` fica
  reservada para as falhas do tipo "o servidor nem consegue verificar"
  (chave ausente).
- **Uma URL assinada com o `expires` adulterado** é verificada como
  `Invalid`, não `Expired`. O payload do HMAC inclui o valor de
  `expires`, então alterá-lo quebra a assinatura primeiro.

```rust
use suprnova::{routing::SignatureVerdict, url};

// Todas estas são Invalid, não Expired:
url::signature_verdict(&req)?;  // parâmetro de query signature ausente
url::signature_verdict(&req)?;  // signature é lixo que não é hex
url::signature_verdict(&req)?;  // o caminho foi adulterado (/orders/1 → /orders/2)
url::signature_verdict(&req)?;  // algum valor de parâmetro de query foi adulterado
url::signature_verdict(&req)?;  // o valor de expires foi adulterado

// Esta é Expired:
url::signature_verdict(&req)?;  // HMAC válido, mas agora > expires
```

## Por que Suprnova diverge

A facade `URL` do Laravel carrega `asset()`, `secureAsset()`,
`assetFrom()` e `action()`. O Suprnova não traz nenhum deles - por
razões deliberadas.

**Assets**. A abordagem de frontend do Suprnova é o Vite mais os discos
de armazenamento ([Sistema de arquivos e armazenamento](filesystem.md)),
não um helper de asset separado. A diretiva `@vite('resources/app.ts')`
do Vite (ou o equivalente do adaptador Inertia) emite as URLs com hash
corretas em produção e a URL do dev server em desenvolvimento. Construir
um canal `URL::asset()` paralelo dividiria a abordagem dos assets entre
dois sistemas que teriam que concordar sobre hashing, versionamento e
qual manifest é autoritativo. O lado do Vite já ganhou essa
responsabilidade.

**Roteamento por action**. O `action('UserController@show', ['id' => 1])`
do Laravel depende do roteamento por string de classe do PHP -
controladores são classes com métodos, e o framework consegue fazer
lookup reverso de uma string `action`. Handlers em Rust são funções
livres. O análogo mais próximo são as rotas nomeadas, e
`route("users.show", &[("id", "1")])` já é a interface certa.
Reintroduzir roteamento por string de action sobre os tipos de handler do
Rust não acrescentaria nada real em relação a rotas nomeadas.

**`URL::forceScheme()` / `URL::forceRootUrl()`**. O Laravel expõe esses
métodos para testes e para sites atrás de proxies reversos que não
repassam `X-Forwarded-Proto`. O Suprnova trata os dois casos por
configuração: `APP_URL` carrega o host e o esquema canônicos; para
ambientes com proxy, o middleware de proxy confiável
([Middleware](middleware.md)) lê os headers `X-Forwarded-*` e atualiza a
URL da solicitação antes de ela chegar ao seu handler. Não há nada para
`forceScheme` sobrescrever - `APP_URL` já diz qual é o esquema.

O que de fato existe aqui é a forma voltada ao usuário que os
consumidores procuram, com os mesmos nomes no formato do Laravel onde
eles se traduzem de forma limpa. O corte é intencional, não um descuido.

## Próximos passos

- [Roteamento](routing.md) - declarando rotas, nomeando-as, grupos de
  rotas, roteamento de recursos, e a superfície completa de
  correspondência por método
- [Respostas](responses.md) - `Redirect::route`,
  `Redirect::signed_route`, `Redirect::back`, e o resto da família de
  helpers de redirecionamento que consome a geração de URLs
- [Hashing](hashing.md) - ciclo de vida do `APP_KEY`, rotação de chaves,
  e o chaveiro compartilhado que sustenta a assinatura de URLs junto com
  a criptografia
- [Fluxos de autenticação](auth-flows.md) - os usuários em produção de
  URLs assinadas: reset de senha, verificação de email, e cookies de
  remember-me
- [Solicitações](requests.md) - `Request::path`, `Request::query`,
  `Request::route_is`, e o lado inverso de todo helper deste capítulo
