# Política de bloqueio

Suprnova é um único processo Tokio de longa duração, não uma frota de
workers PHP de curta duração. Todo registry global de processo, singleton,
e cache compartilhado que você vincula na inicialização sobrevive a toda
solicitação que o toca. Isso muda uma coisa pequena mas consequente sobre
como você recorre a `std::sync::Mutex` e `std::sync::RwLock`: um panic
enquanto mantém uma guarda *envenena o lock* pelo resto da vida do
processo, e o próximo chamador precisa decidir o que fazer a respeito.
Este capítulo é a política de todo o projeto para essa decisão - dois
padrões autorizados, quando escolher qual, e por que você nunca deve
recorrer a um `.lock().unwrap()` bruto em código de framework ou de
aplicação.

## Por que este capítulo existe

No Laravel você nunca pensava em locks envenenados porque não havia
nenhum. PHP é shared-nothing: um erro fatal derruba o processo de uma
solicitação, a próxima solicitação começa em um processo novo, nenhum
estado em memória sobrevive para corromper. Suprnova funciona do jeito
oposto. O processo inicializa uma vez, registries são populados, e eles
permanecem vivos durante toda a vida do binário. Um handler que sofre
panic enquanto mantém uma guarda de escrita em um `RwLock` global de
processo deixa esse lock *envenenado* - todo `.read()` e `.write()`
subsequente retorna `Err(PoisonError)` para sempre, a menos que alguém o
recupere explicitamente.

O idioma padrão do Rust - `.lock().unwrap()` - converte esse `Err` em um
panic. Que então se torna outro lock envenenado em algum lugar acima na
stack. Que então derruba o próximo subsistema que o toca. Uma solicitação
ruim vira uma cascata que resulta em um processo meio-morto.

A política abaixo previne essa cascata.

> **Escopo.** Isso se aplica a `std::sync::Mutex` e `std::sync::RwLock`,
> que carregam estado de envenenamento. Os primos assíncronos em
> `tokio::sync` (`Mutex`, `RwLock`, `Semaphore`) *não* envenenam - um
> panic enquanto mantém uma guarda `tokio::sync::Mutex` derruba a guarda
> de forma limpa e o próximo `.lock().await` tem sucesso. Se seu hot path
> é assíncrono e você não precisa adquirir a guarda a partir de um
> contexto síncrono (uma impl de `Drop`, um callback do framework, um
> subcomando de CLI), prefira as variantes do Tokio e a questão
> desaparece.

## Os dois padrões autorizados

Todo lugar no framework que mantém um lock `std::sync` usa um de
exatamente dois padrões. Escolha da mesma forma no seu próprio código.

### Padrão 1 - Mapear o envenenamento para um erro retornado

Quando o chamador já retorna `Result<_, E>` e mais um `?` não muda sua
forma, exponha o envenenamento como um erro e deixe a solicitação falhar
de forma limpa. O framework usa helpers internos `pub(crate)`
(`lock::read`, `lock::write`, `lock::lock`) que mapeiam uma guarda
envenenada para `FrameworkError::internal("<context> lock poisoned")`,
embutindo um rótulo fornecido pelo chamador para que os logs possam dizer
qual subsistema envenenou sem que cada call site precise envolver o
próprio erro.

O padrão que esses helpers codificam é curto o suficiente para escrever
inline no seu código de aplicação:

```rust
use std::collections::HashMap;
use std::sync::RwLock;
use suprnova::FrameworkError;

static FEATURE_FLAGS: RwLock<HashMap<String, bool>> = RwLock::new(HashMap::new());

pub fn enable(flag: &str) -> Result<(), FrameworkError> {
    let mut guard = FEATURE_FLAGS
        .write()
        .map_err(|_| FrameworkError::internal("feature flags lock poisoned"))?;
    guard.insert(flag.to_string(), true);
    Ok(())
}

pub fn is_enabled(flag: &str) -> Result<bool, FrameworkError> {
    let guard = FEATURE_FLAGS
        .read()
        .map_err(|_| FrameworkError::internal("feature flags lock poisoned"))?;
    Ok(guard.get(flag).copied().unwrap_or(false))
}
```

Dentro de um handler, `is_enabled(...)?` colapsa através do mesmo caminho
`FrameworkError → HttpResponse` que todo outro erro de framework usa: o
cliente recebe um 500 sanitizado com `{"message": "Internal Server
Error"}`, o log estruturado captura a mensagem de envenenamento rotulada,
o request id é preservado de ponta a ponta, e o resto do processo
continua servindo tráfego. Veja o capítulo [Tratamento de erros](errors.md)
para o caminho de conversão completo.

Use este padrão quando:

- O chamador já retorna `Result` (a maioria das operações falíveis
  retorna).
- Um lock envenenado representa uma falha real e irrecuperável do
  subsistema - não há uma "verdade parcial" sensata para recair.
- Você quer que operadores *vejam* o envenenamento nos logs na próxima
  vez que o subsistema for tocado. A mensagem rotulada é sua pista
  forense.

O dispatcher de notifications, o transporte de mail, o registry de
mailable, os db event listeners, e o registry de conexão nomeada do
framework todos usam este padrão. Um panic em qualquer um deles se
manifesta como um 500 na próxima solicitação que atinge o registry; tudo
o mais continua executando.

### Padrão 2 - Recuperar no lugar com `into_inner()`

Quando a assinatura do chamador *não* é falível (um lookup `bool`, uma
verificação de roteamento no hot path, um caminho do qual o ciclo de vida
da solicitação depende) ou quando o estado compartilhado é
estruturalmente seguro para usar depois de uma escrita parcial, recupere
a guarda e continue:

```rust
use std::collections::HashMap;
use std::sync::RwLock;

static ALLOWED_INCLUDES: RwLock<HashMap<&'static str, Vec<&'static str>>> =
    RwLock::new(HashMap::new());

pub fn allows(dto: &str, field: &str) -> bool {
    ALLOWED_INCLUDES
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(dto)
        .map(|fields| fields.contains(&field))
        .unwrap_or(false)
}

pub fn register(dto: &'static str, fields: &'static [&'static str]) {
    let mut guard = ALLOWED_INCLUDES
        .write()
        .unwrap_or_else(|e| e.into_inner());
    guard.insert(dto, fields.to_vec());
}
```

`PoisonError::into_inner()` retorna a guarda apesar do envenenamento.
Leituras e escritas subsequentes prosseguem normalmente - o lock
permanece envenenado para consultas `is_poisoned()`, mas o fluxo de dados
é restaurado.

O framework usa este padrão em `data::registry` (a allowlist de
include-set lida em toda resposta JSON:API), `auth::manager` (o map de
auth-provider nomeado), `app::paths` (o cache de resolved-paths), os
fakes de teste para mail e events, e o map de loaded-env-keys em config.
Cada um é um lugar onde ou nenhum chamador tem um `Result` para retornar,
ou o estado é append-only e estruturalmente seguro de continuar usando.

Use este padrão quando:

- A assinatura do chamador é simples (`bool`, `&str`, um clone de um
  valor armazenado) e mudá-la para `Result` forçaria todo chamador - às
  vezes todo subsistema do framework - a bolhar o erro.
- O estado compartilhado consegue tolerar uma escrita parcial. Maps e
  caches append-only são a forma típica: o pior caso é uma entrada
  ausente ou stale, que o chamador já trata (default-deny, recair para o
  primário, recomputar).
- O hot path executa com frequência suficiente para que retornar um erro
  em cada solicitação subsequente seria operacionalmente pior do que
  degradar.

## Como escolher entre eles

A regra de decisão, em uma frase: **se o pior caso de usar o estado
pós-envenenamento é uma resposta errada com consequências, mapeie para
um erro; se é uma entrada ausente ou stale que o chamador já trata,
recupere no lugar.**

Percorrendo passo a passo:

1. **A assinatura do chamador é `Result<_, E>`?** Se não, você tem que
   recuperar no lugar - adicionar `Result` a um `bool` normalmente é um
   refactor de todo o projeto e não vale a pena por causa de um caso
   extremo de envenenamento.
2. **Se um valor meio-escrito fosse observado, a aplicação tomaria uma
   decisão errada com consequências no mundo real?** Cobrar de um
   cliente errado, permitir um include não autorizado, conceder acesso
   ao tenant errado - isso é "sim, mapeie para um erro." Retornar
   `false` para "esse nome está registrado?" e recair para o pool
   primário - isso é "não, recupere no lugar."
3. **O estado é append-only ou naturalmente idempotente no
   re-registro?** Se sim, recuperar-no-lugar é seguro. Se uma escrita é
   uma transição de máquina de estados que depende do valor anterior,
   prefira mapear-para-erro para você não compor uma corrupção.

Na dúvida, mapeie para um erro. Uma solicitação retornando 500 é um
sinal alto que você pode corrigir; respostas erradas silenciosas não
são.

## Nunca recorra a `.lock().unwrap()`

A forma proibida:

```rust
// NUNCA - um panic em qualquer lugar do call graph abaixo
// desta linha envenena o lock e todo chamador subsequente
// transforma o envenenamento em outro panic.
let mut guard = SOMETHING.lock().unwrap();
```

`.expect("…")` é a mesma coisa com uma mensagem mais agradável. Ambos
convertem um `Err` de lock envenenado em um panic que a rede
`AssertUnwindSafe(...).catch_unwind()` do ciclo de vida da solicitação
captura e converte em um 500 - essa rede é uma *última linha de defesa*,
não uma licença para pular a decisão acima. APIs públicas do framework e
código de aplicação devem escolher um dos dois padrões autorizados.

As duas exceções em que `.unwrap()` é aceitável em um lock `std::sync`:

- **Setup de teste que *quer* afirmar que o envenenamento foi
  alcançado** - o próprio helper de indução de envenenamento de
  `framework/src/lock.rs` usa `.unwrap()` dentro da thread que sofre
  panic de propósito.
- **O caminho de erro de uma operação de envenenamento que já falhou** -
  no momento em que você está dentro da thread de `poison_rw(...)`, o
  panic *é* o objetivo.

Se você não está em um desses casos, escolha um padrão da seção acima.

## E se minha função retornar `bool`?

Essa é a situação em que `ConnectionRegistry::has` vive. É um lookup
`bool` no hot path do roteamento de read-replica do executor, chamado
inline como `if ConnectionRegistry::has("read_replica").await { … }`.
Ampliá-lo para `Result<bool, FrameworkError>` forçaria todo chamador no
executor a bolhar com `?`, propagando um caminho de código de erro
interno para decisões de roteamento que só querem um sim/não.

O padrão recuperar-no-lugar resolve isso - retorne `false` e deixe a
lógica de fallback do chamador entrar em ação (aqui, o executor recai
para o pool primário, que é o comportamento seguro de qualquer forma).
Para garantir que os operadores ainda vejam a condição, emita um
`tracing::warn!` de disparo único na primeira vez que o envenenamento
for observado:

```rust
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;
use std::collections::HashMap;

static REGISTRY: RwLock<HashMap<String, ()>> = RwLock::new(HashMap::new());
static POISON_WARNED: AtomicBool = AtomicBool::new(false);

pub fn has(name: &str) -> bool {
    match REGISTRY.read() {
        Ok(g) => g.contains_key(name),
        Err(_) => {
            // Seguro contra corrida: apenas o primeiro observador loga.
            if !POISON_WARNED.swap(true, Ordering::SeqCst) {
                tracing::warn!(
                    target: "myapp::registry",
                    "registry lock poisoned - `has({name})` degrading to false",
                );
            }
            false
        }
    }
}
```

O gate baseado em `swap` importa: o envenenamento de `RwLock` é
pegajoso, então sem o gate toda chamada subsequente dispararia o warning
novamente e inundaria seus logs. Com o gate, você obtém exatamente um
warn por processo por registry, e um getter que retorna `Result`
correspondente (`get`, `register`) no mesmo registry vai expor o
envenenamento na próxima vez que algo *realmente precisar* que o lookup
tenha sucesso. Isso dá aos operadores dois sinais: um warn precoce de
"algo está errado", e um 500 definitivo no momento em que uma
solicitação realmente dependeu do registry.

## O que o framework já protege

Você não precisa aplicar esta política a nenhum estado que o framework
possui - ela já está em vigor. Concretamente:

- O registry de conexão nomeada (`ConnectionRegistry::register`, `get`,
  `has`) mapeia o envenenamento para `FrameworkError::internal` nas
  escritas e nas leituras que retornam `Result`; `has` degrada para
  `false` com o gate de warn-once.
- O dispatcher de notifications e o registry de factory, o registry de
  mailable, o transporte de mail, a captura em memória de mail, e os DB
  event listeners todos retornam `FrameworkError::internal` no
  envenenamento.
- A allowlist de include de `data::registry`, o map de provider de
  `auth::manager`, `app::paths`, o cache de loaded-env-keys, e os fakes
  de teste em memória todos recuperam no lugar.

Onde você intersecta esses subsistemas através de sua API pública
(`Notification::send`, `Mail::send`, `Auth::user`, `DB::connection`, o
caminho de resposta JSON:API), um lock envenenado do framework se
manifesta como um 500 limpo - nunca um panic no seu call site.

## Por que Suprnova diverge

O Laravel não tem uma política de lock porque não tem estado
compartilhado de longa duração. Cada solicitação PHP recebe seu próprio
processo, sua própria memória, suas próprias cópias de todo singleton.
Não há registry em memória para envenenar e nenhum conceito de "a
próxima solicitação" herdar dano da anterior - o runtime garante uma
folha em branco.

Suprnova é construído sobre o Tokio, que te dá exatamente o estado
compartilhado de longa duração que o PHP descarta. WebSockets baratos,
caches em memória, pools de conexão que você não paga para reconstruir -
tudo isso precisa de registries globais de processo que sobrevivem a
qualquer solicitação individual. Essa capacidade é todo o motivo de
migrar para Rust nesse estilo de app (veja a [introdução](introduction.md)
para a motivação completa do framework). O custo de tê-la é que agora
você precisa pensar sobre o que acontece quando uma thread que sofreu
panic deixa o estado compartilhado em uma condição guardada, porque *há*
estado compartilhado para deixar.

A política de dois padrões é a resposta mais simples que mantém a
capacidade e remove o custo. Recupere no lugar onde o estado é seguro de
continuar usando; mapeie para um erro onde você prefere ter um 500
limpo a uma resposta errada. Ambas as opções deixam o resto do processo
servindo tráfego. Nenhuma delas deixa um unwrap que sofreu panic
esperando para derrubar o subsistema acima.

Essa é a mesma forma da [decisão fail-open vs fail-closed](rate-limiting.md)
que o framework aplica a backends de cache e rate-limit inalcançáveis:
uma escolha explícita de política no call site, não um padrão implícito.
Async em toda parte te dá estado de longa duração; o framework te dá o
playbook para mantê-lo honesto.

## Próximos passos

- [Tratamento de erros](errors.md) - como `FrameworkError::internal` se
  torna o 500 sanitizado que o cliente recebe, com a mensagem de
  envenenamento rotulada preservada no seu log estruturado.
- [Contêiner de serviços](container.md) - onde os registries globais de
  processo que esta política protege realmente vivem, e por que o
  escopo task-local/thread-local impede que testes herdem as vinculações
  uns dos outros.
- [Ciclo de vida da solicitação](lifecycle.md) - o limite de panic
  (`execute_chain_safely`) que captura o unwrap de *último recurso* e o
  converte em um 500, para que você entenda exatamente o que a rede de
  segurança faz e por que ela não é uma desculpa para pular a política
  acima.
- [Limitação de taxa](rate-limiting.md) - a história paralela do
  `BackendErrorPolicy` para backends que podem estar *inalcançáveis* em
  vez de envenenados; mesmo princípio de escolha explícita, modo de
  falha diferente.
- [Testes](testing.md) - como `TestContainer::fake` e a camada de
  contêiner thread-local impedem que testes paralelos poluam os
  registries uns dos outros, que é o complemento em tempo de teste para
  a história de tratamento de envenenamento.
