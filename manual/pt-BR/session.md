# Sessões

A sessão é o bag de chave/valor por usuário que sobrevive entre
solicitações no mesmo navegador. O Suprnova traz um driver apoiado em
banco de dados pronto para uso, o conecta através do `SessionMiddleware`,
e expõe a sessão ativa através de duas funções livres - `session()` para
leituras, `session_mut()` para escritas. Use-a sempre que um valor deva
viver mais do que uma solicitação, mas não deva ser algo que a URL ou um
JWT carreguem.

## Como uma solicitação vê a sessão

`SessionMiddleware` roda em toda solicitação e faz cinco coisas, nesta
ordem:

1. Lê o id da sessão e o timestamp do último touch de atividade
   bem-sucedido a partir do cookie `suprnova_session` (criptografado com
   AES-256-GCM). Cookies adulterados, indecifráveis ou malformados são
   tratados como ausentes.
2. Carrega `SessionData` do armazenamento apenas quando um cookie válido
   nomeia uma sessão. Solicitações sem cookie começam com uma sessão
   limpa em memória e não disparam um miss garantido no banco de dados.
   Um cookie cuja linha não existe mais é limpo sem recriar uma linha
   vazia. Um erro de leitura do armazenamento registra `warn!` e deixa
   uma solicitação sem estado seguir, mas uma mutação no handler aí é
   fail-closed em vez de sobrescrever um estado armazenado desconhecido.
3. Envelhece os dados de flash: `_flash.old.*` é descartado,
   `_flash.new.*` é renomeado para `_flash.old.*`. Depois desse passo,
   tudo o que a solicitação anterior colocou em flash está legível; tudo
   o que esta solicitação colocar em flash estará legível na próxima.
4. Vincula a sessão a um slot task-local pela duração do handler.
   `session()` e `session_mut()` consultam esse slot.
5. Depois que o handler retorna, persiste o estado sujo da sessão ou um
   touch limitado de expiração deslizante, anexa um cookie criptografado
   de substituição somente depois de uma escrita bem-sucedida, e drena os
   cookies pendentes fora de banda (por exemplo, um cookie de remember-me
   recém-rotacionado). Uma solicitação limpa e sem cookie não faz nenhum
   I/O no armazenamento de sessão e não recebe nenhum cookie de sessão.

O passo 5 tem uma garantia de segurança que vale destacar: **se a sessão
foi modificada nesta solicitação e a escrita no armazenamento falha, a
resposta é substituída por um 500.** Retornar o sucesso do handler
significaria entregar ao cliente um cookie para um estado que o banco de
dados nunca registrou - a próxima solicitação carregaria uma sessão vazia
e a mutação (login, rotação de CSRF, flash) sumiria silenciosamente.
Solicitações somente de leitura que falham apenas em um touch devido de
`last_activity` registram `warn!`, mantêm o cookie existente, e passam
adiante.

## Lendo a sessão

```rust
use suprnova::session::session;

if let Some(s) = session() {
    let user_id: Option<String> = s.get("preferred_username");
    if s.has("cart") {
        // ...
    }
    if s.missing("locale") {
        // primeira visita
    }
}
```

`session()` clona o `SessionData` atual. Retorna `None` fora de um escopo
de solicitação (um teste unitário que não instalou o middleware, um
subcomando de CLI). Para um valor tipado, `get::<T>` desserializa a
partir do JSON subjacente; em uma chave ausente ou um tipo errado, você
recebe `None` e nenhum panic.

## Escrevendo na sessão

`session_mut` recebe uma closure que recebe `&mut SessionData`:

```rust
use suprnova::session::session_mut;

session_mut(|s| {
    s.put("locale", "en");
    s.put("preferences", serde_json::json!({
        "theme": "dark",
        "notifications": true,
    }));
    s.forget("legacy_key");
});
```

A closure é síncrona - as guardas do lock subjacente são liberadas antes
de qualquer `.await`, então isso se compõe dentro de handlers async sem
segurar o lock através de suspensões. Qualquer coisa que você serialize
precisa implementar `Serialize`; a desserialização em `get` exige
`DeserializeOwned`.

A forma de closure (em vez de retornar uma guarda) é deliberada. Futures
no Tokio podem retomar em uma worker thread diferente daquela em que
começaram, então a sessão tem que viver em um slot `task_local!` e ser
emprestada através de uma seção crítica presa a um escopo. O formato
`|s|` torna essa fronteira explícita e impede que você segure sem querer
uma guarda de mutex através de um `.await`.

## Dados de flash

Valores de flash ficam visíveis por **uma** solicitação subsequente e
depois somem. O padrão usual: um controlador escreve um flash, retorna um
redirecionamento, a próxima página renderiza o flash.

```rust
use suprnova::session::session_mut;

session_mut(|s| s.flash("status", "Profile updated."));
```

Na próxima solicitação:

```rust
use suprnova::session::session_mut;

let status: Option<String> = session_mut(|s| s.get_flash("status"));
```

`get_flash` remove o valor ao retorná-lo. Para a variante que lê sem
consumir, use `get::<String>("_flash.old.status")`, mas a forma que
consome é o que os controladores geralmente querem.

A superfície de flash completa do Laravel está disponível:

- `flash(key, value)` - escreve para a próxima solicitação
- `now(key, value)` - escreve apenas para a solicitação atual
- `reflash()` - coloca em flash de novo tudo que está visível agora, por
  mais uma rodada
- `keep(&["k1", "k2"])` - coloca em flash de novo um subconjunto
  específico
- `flash_input(map)` / `old_input()` / `get_old_input(key)` - o bag de
  input de formulário usado pelos helpers `Redirect::with_input` /
  `old()`

## Regenerar e invalidar

Depois de uma mudança de credencial (login, reset de senha, aprovação em
2FA) você rotaciona o id da sessão, para que um id fixado de antes da
mudança não seja mais válido:

```rust
use suprnova::session::{regenerate_session_id, regenerate_csrf_token};

regenerate_session_id();        // novo id, mesmos dados
regenerate_csrf_token();        // novo token CSRF, mesmo id e mesmos dados
```

Para limpar a sessão inteiramente (logout):

```rust
use suprnova::session::invalidate_session;

invalidate_session();           // limpa os dados + cunha um token CSRF novo
```

Para um evento de segurança que precisa revogar todas as sessões de um
usuário (reset de senha em outro lugar, recuperação de conta, logout
forçado pelo admin):

```rust
use suprnova::session::destroy_all_for_user;

let rows = destroy_all_for_user("user-42").await?;
tracing::info!(revoked = rows, "all sessions destroyed");
```

Isso envolve `SessionStore::destroy_for_user` sobre o
`DatabaseSessionDriver` padrão do framework. Se você vinculou um
armazenamento customizado, chame `destroy_for_user` nele diretamente.

## Helpers de autenticação

`auth_user_id()` retorna o id do usuário autenticado no momento
(consultando primeiro o estado de autenticação com escopo de solicitação,
caindo de volta para o campo persistido na sessão):

```rust
use suprnova::session::{auth_user_id, is_authenticated};

if is_authenticated() {
    let uid = auth_user_id().expect("just checked");
    // ...
}
```

Normalmente você conduz a autenticação através da facade
[Auth](authentication.md) - `Auth::login`, `Auth::logout`,
`Auth::user()`. Os helpers de sessão são a camada de baixo nível sobre a
qual essas facades se apoiam; recorra a eles quando precisar inspecionar
a sessão crua ou quando estiver implementando o seu próprio guard.

## Outras operações

A API de `SessionData` espelha a superfície `Store` do Laravel:

| Método | O que faz |
|---|---|
| `get::<T>(key)` | leitura tipada |
| `put(key, value)` | escrita tipada |
| `forget(key)` | remove uma única chave |
| `forget_many(&[..])` | remove várias chaves |
| `flush()` | limpa todos os dados (mantém o id) |
| `has(key)` / `missing(key)` | verificação de presença |
| `has_any(&[..])` / `has_all(&[..])` | presença em lote |
| `all()` | empresta o mapa subjacente |
| `only(&[..])` / `except(&[..])` | clones filtrados |
| `pull::<T>(key)` | pega e esquece de uma só vez |
| `push(key, value)` | acrescenta a um valor de array |
| `increment(key, n)` / `decrement(key, n)` | contadores inteiros |
| `remember::<T>(key, \|\| default())` | pega, ou calcula e guarda |
| `replace(&[(k, v), ..])` | faz flush e então grava em lote |
| `put_many(&[(k, v), ..])` | gravação em lote com merge |
| `previous_url()` / `set_previous_url(url)` | o que `Redirect::back` lê |
| `password_confirmed()` / `password_confirmed_at()` | timestamp de "o usuário confirmou a senha agora mesmo" |

Recorra a estes dentro de `session_mut` para as operações que alteram, e
a `session()` para leituras. O slot `previous_url` é preenchido
automaticamente pelo middleware em respostas GET HTML bem-sucedidas,
então `redirect()->back()` funciona sem que você faça nada.

## Configuração

Configure as sessões através de variáveis de ambiente -
`SessionConfig::from_env` as lê na inicialização:

```env
# Tempo de vida em minutos. Comanda o TTL da linha e o Max-Age do cookie.
SESSION_LIFETIME=120

# Mínimo de segundos entre escritas de expiração deslizante (padrão 5 minutos).
# Em tempo de execução, isso é limitado a um valor abaixo do tempo de vida da sessão.
SESSION_TOUCH_INTERVAL=300

# Cadência em segundos da coleta supervisionada de linhas expiradas (padrão 1 hora).
SESSION_GC_INTERVAL=3600

# Nome do cookie no cliente.
SESSION_COOKIE=suprnova_session

# Atributos do cookie
SESSION_SECURE=true          # exige HTTPS; O PADRÃO É true
SESSION_PATH=/
SESSION_DOMAIN=.example.com  # opcional; sem valor = somente o host
SESSION_SAME_SITE=Lax        # Lax | Strict | None
SESSION_PARTITIONED=false    # opt-in do CHIPS
SESSION_EXPIRE_ON_CLOSE=false # true → omite Max-Age, o navegador descarta ao fechar

# Conexão de BD nomeada para o armazenamento de sessão (opcional)
SESSION_CONNECTION=sessions

# Tempo de vida do token/cookie de remember-me em minutos (padrão 30 dias)
REMEMBER_LIFETIME=43200
```

Alguns padrões que vale destacar:

- **`SESSION_SECURE` tem `true` como padrão.** Sessões enviadas sobre
  HTTP puro seriam um risco de vazamento de credenciais, então a flag
  secure vem ligada por padrão. Para desenvolvimento local sobre HTTP,
  defina `SESSION_SECURE=false` no seu `.env` local.
- **`HttpOnly` está sempre ligado.** Não há botão para desativá-lo -
  expor o cookie de sessão ao JavaScript abre mão da principal proteção
  contra XSS, e não há motivo moderno legítimo para querer isso.
- **`SameSite` tem `Lax` como padrão.** `Strict` bloqueia a sessão na
  maioria das navegações GET entre sites (inclusive links de volta
  vindos de email); `Lax` é a resposta certa de costume.

Para configuração programática use o builder fluente:

```rust
use std::time::Duration;
use suprnova::SessionConfig;

let config = SessionConfig::new()
    .lifetime(Duration::from_secs(60 * 60))      // 1 hora
    .touch_interval(Duration::from_secs(5 * 60))
    .gc_interval(Duration::from_secs(60 * 60))
    .cookie_name("myapp_session")
    .secure(true)
    .domain(".example.com")
    .remember_lifetime(Duration::from_secs(30 * 24 * 60 * 60));
```

## Conectando tudo

`SessionMiddleware` é instalado como um middleware global na
inicialização do seu app. A ordem dos middlewares importa: a sessão
precisa vir antes do [CSRF](csrf.md), já que o CSRF lê o token por
sessão.

```rust
use std::sync::Arc;
use suprnova::{global_middleware, CsrfMiddleware, SessionConfig, SessionMiddleware};

pub async fn bootstrap() {
    let config = SessionConfig::from_env();

    // `install` registra também o supervisor de GC configurado.
    // Use `SessionMiddleware::new(config)` se preferir agendar o GC
    // você mesmo via `Schedule`.
    global_middleware!(SessionMiddleware::install(config).await);

    global_middleware!(CsrfMiddleware::new());
}
```

`SessionMiddleware::install` registra uma task de gc
[supervisionada](supervisors.md) que chama `gc()` a cada
`SESSION_GC_INTERVAL` (uma vez por hora, por padrão). A variante
`install_with_gc(config, interval).await` recebe um intervalo
customizado; `new(config)` pula a task de gc (útil se você preferir
chamar `gc()` a partir de uma entrada de
[Agendamento](scheduling.md)). A task supervisionada participa do dreno
de shutdown do framework, então o loop de gc sai de forma limpa em
`Ctrl-C` / `SIGTERM` em vez de ser abortado à força.

Endpoints protegidos de operações podem expor o estado do coletor sem
consultar a tabela de sessões:

```rust
use suprnova::session::session_gc_metrics;

let metrics = session_gc_metrics();
tracing::info!(
    runs = metrics.runs,
    failures = metrics.failures,
    removed_rows = metrics.removed_rows,
    last_success = metrics.last_success_unix_seconds,
    "session collector status"
);
```

Para usar um armazenamento que não seja de banco de dados - para testes,
ou para um driver apoiado em Redis que você mesmo escreva - implemente
`SessionStore` e passe-o via `with_store`:

```rust
use std::sync::Arc;
use suprnova::{SessionConfig, SessionMiddleware, SessionStore};

let store: Arc<dyn SessionStore> = Arc::new(MyRedisStore::new());
let mw = SessionMiddleware::with_store(SessionConfig::from_env(), store);
```

## A tabela de sessões

O driver padrão espera uma tabela `sessions` com este formato (a entidade
SeaORM em `framework/src/session/driver/database.rs` é a fonte da
verdade):

| Coluna | Tipo | Observações |
|---|---|---|
| `id` | VARCHAR PK | id de sessão alfanumérico minúsculo de 40 caracteres |
| `user_id` | VARCHAR NULL | id do usuário autenticado (string, suporta ids opacos) |
| `payload` | TEXT | mapa de dados da sessão serializado em JSON |
| `csrf_token` | VARCHAR | token CSRF por sessão |
| `last_activity` | TIMESTAMP | último acesso; comanda a expiração + o GC |

Dois índices acompanham a tabela: `idx_sessions_user_id` (para
`destroy_for_user`) e `idx_sessions_last_activity` (para `gc()`).

Um app criado com scaffold já inclui uma migração
`create_sessions_table` que corresponde a esse formato. Se você trouxer
as suas próprias migrações, espelhe os nomes das colunas exatamente - o
SeaORM os resolve posicionalmente e uma coluna renomeada não vai
corresponder.

### Por que Suprnova diverge

Dois pontos em que o Laravel fez uma escolha com formato de PHP que o
Tokio nos deixa fazer de outro jeito:

**Coleta de lixo.** O Laravel roda uma loteria de 2/100 em toda
solicitação: cada solicitação tem 2% de chance de disparar o GC de sessão
inline. Isso funciona no PHP porque toda solicitação já cria um processo
novo de qualquer jeito. No Tokio nós temos workers de vida longa, então
`SessionMiddleware::install` registra uma única task
[supervisionada](supervisors.md) que chama `gc()` em um intervalo fixo.
Sem overhead por solicitação, sem surpresa probabilística - agendamento
explícito em vez de loteria, e o loop de reinício do supervisor captura
panics, então um único gc ruim não mata o daemon.

**`session_mut` na forma de closure.** O Laravel te entrega
`$request->session()` e deixa você chamar métodos nele. Nós não, porque
handlers no Suprnova são futures e um future pode retomar em uma worker
thread diferente daquela em que começou. A sessão vive em um slot
`task_local!` do Tokio, o que significa que o acesso emprestado tem que
acontecer dentro de um escopo. A forma de closure torna esse escopo
explícito e impede estaticamente o erro de segurar uma guarda de mutex
através de um `.await`.

**Fail-closed em escritas sujas.** Um touch de atividade limitado que
falha registra `warn!` e deixa a solicitação passar com o cookie existente
(o estado visível ao usuário está intacto). Uma escrita que falha de uma
sessão *modificada* - login, flash, rotação de CSRF - retorna 500.
Entregar silenciosamente ao cliente um cookie para um estado que o
armazenamento nunca registrou faria um login "bem-sucedido" sumir já na
solicitação seguinte; é melhor expor a falha de forma explícita.

## Próximos passos

- [Autenticação](authentication.md) - `Auth::login`, guards, a chain de
  provedores de usuário
- [Fluxos de autenticação](auth-flows.md) - reset de senha, 2FA,
  throttling de força bruta, remember-me
- [CSRF](csrf.md) - como o token CSRF da sessão é verificado nas escritas
- [Middleware](middleware.md) - escrevendo o seu próprio middleware que
  lê ou escreve na sessão
- [Ciclo de vida da solicitação](lifecycle.md) - onde `SessionMiddleware`
  fica na chain
