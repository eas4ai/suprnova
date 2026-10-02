# Ações

Uma ação no Suprnova é uma struct com um único trabalho: manter um
único pedaço de lógica de negócio atrás de um método. É o análogo Rust
dos controladores invocáveis de ação única do Laravel - `RegisterUser`,
`PublishPost`, `ChargeInvoice`. A ação vive em `src/actions/`, carrega
o attribute `#[injectable]` para que o contêiner possa resolvê-la, e
expõe um método `execute(...)` que controladores (e jobs, e outras
ações) chamam. Não existe uma macro `#[action]` nem imposição por
parte do framework de que haja "um único método" - o formato é uma
convenção, e `#[injectable]` é a maquinaria que torna essa convenção
indolor.

```rust
use suprnova::{injectable, FrameworkError};

#[injectable]
pub struct RegisterUserAction {
    // Injete dependências como campos - veja "Dependências" abaixo
}

impl RegisterUserAction {
    pub async fn execute(&self, email: &str) -> Result<String, FrameworkError> {
        tracing::info!(action = "RegisterUser", email, "executed");
        Ok(format!("registered: {email}"))
    }
}
```

Resolva-a a partir de um handler com `App::resolve::<RegisterUserAction>()?`
e você separou sua lógica de domínio da camada HTTP sem inventar uma
classe-base de camada de serviço. Esse é o padrão inteiro.

## Gerando uma ação

```bash
suprnova make:action RegisterUser
```

A CLI normaliza o nome para PascalCase, acrescenta `Action` se o
sufixo estiver ausente, e então converte o nome do arquivo para
snake_case. Então:

| `make:action <Name>` | Nome da struct | Arquivo |
|---|---|---|
| `RegisterUser` | `RegisterUserAction` | `src/actions/register_user_action.rs` |
| `SendNotification` | `SendNotificationAction` | `src/actions/send_notification_action.rs` |
| `ProcessPayment` | `ProcessPaymentAction` | `src/actions/process_payment_action.rs` |
| `ChargeInvoiceAction` | `ChargeInvoiceAction` | `src/actions/charge_invoice_action.rs` |

O gerador escreve o arquivo e acrescenta uma linha
`pub mod register_user_action;` a `src/actions/mod.rs`. O stub emitido
compila imediatamente:

```rust
//! register_user_action action

use suprnova::{injectable, FrameworkError};

/// RegisterUserAction
///
/// Single-responsibility command resolved from the container. Inject any
/// dependencies as fields and the `#[injectable]` macro wires them at
/// resolve time.
#[injectable]
pub struct RegisterUserAction {
    // Add injected dependencies as fields here, e.g.
    // db: suprnova::DbConnection,
}

impl RegisterUserAction {
    /// Execute the action.
    pub async fn execute(&self) -> Result<String, FrameworkError> {
        Ok("RegisterUserAction executed".to_string())
    }
}
```

A assinatura - `async fn execute(&self) -> Result<_, FrameworkError>` -
é o formato seguro para produção: async, retornando um `Result` que
converte através do `?` diretamente para um `HttpResponse` no call
site. O corpo é um placeholder; troque-o pelo fluxo de trabalho real.

## O attribute `#[injectable]`

`#[injectable]` é a única peça de maquinaria do framework da qual o
padrão de ações depende. Ele se expande em três coisas:

1. Um `#[derive(Clone)]` na struct (e `Default` quando não há campos
   `#[inject]`).
2. Uma entrada `inventory::submit!` para que o boot possa descobrir o
   tipo.
3. Uma closure de auto-registro que `App::singleton_if_absent` executa
   uma vez durante `boot_services()`.

O contrato da macro:

| Formato da struct | Comportamento |
|---|---|
| Struct unitária (`pub struct Foo;`) | Deriva `Default + Clone`, registra `Default::default()` |
| Campos nomeados, nenhum `#[inject]` | Deriva `Default + Clone`, registra `Default::default()` |
| Campos nomeados com `#[inject]` | Deriva apenas `Clone`; cada campo `#[inject]` é resolvido a partir do contêiner no boot, campos não injetados usam o padrão |
| Struct tupla | Rejeitada em tempo de compilação - "use campos nomeados em vez disso" |

Uma ação resolvida é um clone do singleton armazenado. O custo é um
`Clone` por chamada `App::resolve::<Action>()?`, que para uma unit
struct ou uma struct de serviços envolvidos em `Arc` é um punhado de
incrementos de refcount. Estado pesado pertence atrás de serviços
`Arc<dyn …>` que a ação injeta, não dentro da própria ação.

### `#[inject]` acontece no boot, não a cada chamada

Quando o framework inicializa, `App::boot_services()` percorre todo
registro `#[injectable]` e os executa em um loop de novas tentativas
de ponto fixo. Cada entrada tenta resolver seus campos `#[inject]` a
partir do contêiner. Se uma dependência ainda não foi registrada, a
entrada é adiada para a próxima iteração. O loop executa até que toda
entrada tenha sucesso ou nenhum progresso seja feito - e em caso de
falha o framework retorna um erro estruturado nomeando o tipo não
resolvível ou o ciclo.

A consequência prática: **`App::resolve::<MyAction>()` clona o
singleton já construído**. Ele não executa a resolução de `#[inject]`
a cada chamada. Qualquer injectable do qual uma ação dependa precisa
ele mesmo estar registrado antes da ação - seja via seu próprio
attribute `#[injectable]`, seja por um `App::bind` / `App::singleton`
manual na sua função `bootstrap()`. O loop de novas tentativas cuida
da ordenação do inventory para você; ele não inventa serviços
ausentes.

## Usando uma ação a partir de um controlador

O formato padrão de handler: resolver, executar, renderizar.

```rust
use suprnova::{App, Request, Response, ResponseExt, json_response};

use crate::actions::register_user_action::RegisterUserAction;

pub async fn store(_req: Request) -> Response {
    let action = App::resolve::<RegisterUserAction>()?;
    let result = action.execute("alice@example.com").await?;

    json_response!({ "ok": true, "result": result }).status(201)
}
```

Os dois pontos de `?` funcionam porque os dois tipos de erro se
convertem em `HttpResponse` via impls de `From` - `App::resolve`
retorna `Result<T, FrameworkError>` e o conversor de erro do framework
cuida do resto. Registro de serviço ausente aparece como um 500 com o
nome do serviço no log estruturado, não um panic. Veja
[Modelo de erros](error-model.md) para o quadro completo.

Se você preferir evitar o `?` no resolve - por exemplo em um caminho
que deve falhar duro no tempo de boot - `App::get::<RegisterUserAction>()`
retorna `Option<T>` e você pode usar `.expect("registered at boot")`
para falhar de forma explícita se errou a fiação.

## Ações assíncronas que tocam o banco de dados

Este é o caminho que a maioria das ações de fato toma - carregar ou
escrever através de um model Eloquent. Extraia o corpo do seu domínio;
a superfície é a mesma.

```rust
use suprnova::{attrs, injectable, FrameworkError, Model};

use crate::models::todos::Todo;

#[injectable]
pub struct CreateRandomTodoAction;

impl CreateRandomTodoAction {
    pub async fn execute(&self) -> Result<Todo, FrameworkError> {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
            % 10000;

        Todo::create(attrs! {
            title: format!("Todo #{}", n),
            description: format!("created at {}", n),
            done: false,
        })
        .await
    }
}

#[injectable]
pub struct ListTodosAction;

impl ListTodosAction {
    pub async fn execute(&self) -> Result<Vec<Todo>, FrameworkError> {
        Ok(<Todo as suprnova::eloquent::Model>::all().await?.into_vec())
    }
}
```

`Todo::create(attrs!{...})` e `Todo::all()` vêm da macro
`#[suprnova::model]`. Veja [API Eloquent](eloquent.md) para a
superfície do model. Note que `Model::all()` retorna uma
`Collection<Todo>` - o exemplo chama `.into_vec()` para entregar ao
controlador um `Vec` simples; você também pode retornar a `Collection`
diretamente e deixar o serializer renderizá-la.

Conectando isso a um controlador:

```rust
use suprnova::{App, Request, Response, ResponseExt, json_response};

use crate::actions::todo_action::{CreateRandomTodoAction, ListTodosAction};

pub async fn create_random(_req: Request) -> Response {
    let action = App::resolve::<CreateRandomTodoAction>()?;
    let todo = action.execute().await?;
    json_response!({ "ok": true, "todo": todo }).status(201)
}

pub async fn list(_req: Request) -> Response {
    let action = App::resolve::<ListTodosAction>()?;
    let todos = action.execute().await?;
    json_response!({ "ok": true, "todos": todos })
}
```

Dois `?` por handler; o controlador permanece um adaptador fino entre
HTTP e o domínio.

## Dependências via `#[inject]`

Quando uma ação precisa de colaboradores - um mailer, um logger, um
serviço de domínio - declare-os como campos e marque cada um com
`#[inject]`:

```rust
use suprnova::{injectable, FrameworkError};

use crate::services::{MailerService, LoggerService};

#[injectable]
pub struct SendWelcomeEmailAction {
    #[inject]
    mailer: MailerService,
    #[inject]
    logger: LoggerService,
}

impl SendWelcomeEmailAction {
    pub async fn execute(&self, to: &str) -> Result<(), FrameworkError> {
        self.logger.info(&format!("welcome → {to}"));
        self.mailer.send_welcome(to).await
    }
}
```

Tanto `MailerService` quanto `LoggerService` precisam elas mesmas
estar registradas no contêiner antes que esta ação inicialize - seja
com seu próprio attribute `#[injectable]`, seja por uma chamada em
`bootstrap()`:

```rust
// Em src/bootstrap.rs
App::singleton(MailerService::from_env()?);
App::singleton(LoggerService::default());
```

Se qualquer uma das dependências estiver ausente quando o boot executa
o loop de ponto fixo, o boot retorna um erro nomeando o tipo não
resolvido e o framework sai com código não-zero em vez de iniciar com
um contêiner parcialmente conectado.

Campos não marcados com `#[inject]` recorrem a `Default::default()`,
então você pode misturar dependências injetadas com estado simples sem
escrever um construtor.

## Quando usar uma ação

A regra prática: uma ação existe quando o mesmo pedaço de trabalho é
(ou pode vir a ser) disparado a partir de mais de um ponto de entrada.
Um fluxo de registro que executa tanto a partir de uma rota HTTP
quanto de um job enfileirado pertence a `RegisterUserAction`. Um
handler pontual de "renderizar esta página de índice" não precisa de
uma ação - mantenha-o no controlador.

| Bom encaixe | Exemplo |
|---|---|
| Operações de negócio multi-etapas | `RegisterUserAction`, `CheckoutAction` |
| Trabalho compartilhado entre HTTP + fila | `IssueRefundAction` (despachada das duas formas) |
| Lógica que vale a pena testar sem uma solicitação | `CalculateTotalsAction` |
| Integrações externas | `SendEmailAction`, `SyncInventoryAction` |
| Qualquer coisa que o controlador de outra forma faria inline + duplicaria | gatilho da regra dos três |

Comparada a um controlador, uma ação é reutilizável, não tem
vinculação a `Request`, e é trivial de chamar a partir de um teste
(`App::resolve` + `await`). Um controlador permanece uma fronteira
ciente de HTTP que sabe como traduzir o resultado de uma ação em um
`Response`.

| Controlador | Ação |
|---|---|
| Lida com uma rota | Reutilizável entre rotas, jobs, agendamentos |
| Conhece `Request` / `Response` | Conhece seus tipos de domínio |
| Retorna `Response` | Retorna `Result<T, FrameworkError>` |
| Chama ações | Chamada por controladores (e outros) |

## Ações, o barramento e as filas

Ações não são o único lugar onde a lógica de negócio pode viver - o
[Barramento](bus.md) lida com comandos despachados com saídas tipadas,
e a [Fila](queues.md) lida com trabalho que deve executar em um
worker. Escolha pela forma como o trabalho é invocado:

| Você quer… | Use |
|---|---|
| Lógica de negócio síncrona, chamável a partir de um controlador ou de um job | **Ação** (`#[injectable]` + `execute`) |
| Um comando tipado com um handler registrado, chamável via `Bus::dispatch` | [Barramento](bus.md) |
| Trabalho durável, com retry, fora da task de solicitação | [Fila](queues.md) |

Misturar as duas abordagens é normal: um `BusHandler` ou um `Job`
frequentemente apenas resolve uma ação e chama seu `execute`. A ação
guarda a lógica de domínio; o barramento ou a fila guarda os metadados
de dispatch.

## Layout de arquivos

O que `make:action` emite, mais o espaço para agrupar:

```
src/
├── actions/
│   ├── mod.rs                          // pub mod register_user_action;
│   ├── register_user_action.rs
│   ├── send_welcome_email_action.rs
│   └── billing/                        // agrupe por domínio quando o diretório crescer
│       ├── mod.rs
│       ├── charge_invoice_action.rs
│       └── issue_refund_action.rs
├── controllers/
└── main.rs
```

Nada no framework exige esse layout; o gerador escreve em
`src/actions/` porque essa é a convenção. Mova uma ação para
`src/billing/actions/` e ela continuará funcionando - `#[injectable]`
é agnóstico quanto à localização.

## Testando uma ação

Como uma ação é apenas uma struct resolvível pelo contêiner com um
método `async`, a superfície de teste é `App::resolve` + `await`. A
mesma fixture de teste `TestDatabase` usada em outros lugares funciona
aqui:

```rust
use suprnova::{describe, expect, test, App};
use suprnova::testing::TestDatabase;

use crate::actions::todo_action::ListTodosAction;
use crate::models::todos::Todo;

describe!("ListTodosAction", {
    test!("returns all todos", async fn(_db: TestDatabase) {
        Todo::create(suprnova::attrs! { title: "Test", description: "", done: false })
            .await
            .unwrap();

        let action = App::resolve::<ListTodosAction>().unwrap();
        let todos = action.execute().await.unwrap();

        expect!(todos).to_have_length(1);
    });
});
```

Veja [Testes](testing.md) para a superfície completa de `describe!` /
`test!` / `expect!` e para `TestContainer::fake` quando você quiser
injetar um fake-mailer ou fake-gateway em uma ação sob teste.

## Por que Suprnova diverge

Os controladores de ação única do Laravel - classes com um método
`__invoke` em `App\Actions\` - são construídas por solicitação. O
contêiner resolve a classe, executa a injeção via construtor, e a
instância é descartada quando a resposta sai. O modelo de
processo-por-solicitação do PHP torna isso essencialmente gratuito.

As ações do Suprnova são singletons residentes no contêiner:
construídas uma vez no boot com os campos `#[inject]` resolvidos
naquele momento, e clonadas a cada `App::resolve`. O padrão se encaixa
no Rust porque clonar uma struct de serviços envolvidos em `Arc` custa
alguns incrementos de refcount, enquanto construir-e-descartar uma
struct a cada solicitação forçaria cada campo por uma alocação. A
convenção no formato Laravel - uma struct, um método, nomeado pela
operação - sobrevive intacta; a fiação por baixo dela é moldada para o
Tokio.

A outra separação intencional: controladores permanecem funções
livres (veja [Controladores](controllers.md)), então a camada HTTP é
uma transformação pura de solicitação-para-resposta sem superfície de
DI própria. A injeção no estilo construtor acontece na fronteira do
`#[injectable]`, dentro da ação, onde ela pertence.

## Próximos passos

- [Controladores](controllers.md) - as funções livres voltadas para HTTP que resolvem e chamam ações
- [Contêiner de serviços](container.md) - o que `App::resolve`, `App::singleton`, e o lookup em três camadas realmente fazem
- [Barramento](bus.md) - dispatch de comando tipado para quando você quer um handler registrado em vez de uma ação resolvida
- [Testes](testing.md) - `App::resolve` + `TestContainer::fake` para testes de ação herméticos
- [Modelo de erros](error-model.md) - como o `?` em `App::resolve::<Action>()?` e `action.execute().await?` colapsa em uma resposta limpa
