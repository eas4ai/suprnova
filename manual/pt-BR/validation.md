# Validação

O Suprnova valida a entrada da solicitação em duas trilhas complementares:

1. **Validação por derive** - attributes `#[validate(...)]` em um struct
   `FormRequest`, executados automaticamente por `extract()`. Este é o
   caminho do dia a dia e está coberto em [Solicitações](requests.md).
   Ele trata regras por campo (`email`, `length`, `range`, …) de forma
   declarativa.
2. **Objetos de regra + a macro `validate!`** - valores simples que
   implementam [`Rule`](#objetos-de-regra) / `ContextualRule` / `AsyncRule`,
   compostos de forma imperativa. Recorra a eles quando precisar de
   lógica entre campos, de regras que tocam o banco de dados, ou de
   regras que você quer guardar e passar adiante.

As duas trilhas acumulam no mesmo conjunto
[`ValidationErrors`](error-model.md) e renderizam a mesma forma
Laravel/Inertia `{ "message", "errors": { field: [...] } }` (HTTP
422).

## Objetos de regra

Uma regra é um valor que implementa uma de três traits:

| Trait | Forma | Uso |
|-------|-------|-----|
| `Rule` | `passes(&self, value: &str)` | verificação pura sobre um valor |
| `ContextualRule` | `passes(&self, value, ctx)` | verificação que lê campos irmãos |
| `AsyncRule` | `async passes(&self, value)` | verificação que usa `.await` (BD, HTTP) |

Regras `Rule` embutidas: `Required`, `Email`, `Min`, `Max`, `Between`,
`In`, `NotIn`, `Integer`, `Numeric`, `Boolean`, `Alpha`, `AlphaNum`,
`Url`, `HttpUrl`, `Uuid`. Regras `ContextualRule` embutidas:
`RequiredIf`, `RequiredWith`, `RequiredUnless`, `Same`, `Different`,
`Confirmed`. Regra `AsyncRule` embutida: [`Unique`](#a-regra-unique).

```rust
use suprnova::{Rule, rules::Email};

Email.passes("user@example.com")?; // Ok(())
```

> **Nota:** `Numeric` aceita um número **finito** - `NaN`, `inf`, e magnitudes que
> transbordam para infinito são rejeitados, mesmo que o parser do Rust
> aceitasse essas strings. Use `HttpUrl` (não `Url`) para entradas de
> callback/webhook/avatar: `Url` faz parse de qualquer esquema que
> `url::Url` aceite (`file:`, `javascript:`, URIs customizadas),
> enquanto `HttpUrl` exige `http`/`https`.

### Escrevendo sua própria regra

Uma regra customizada é um struct unitário (ou que carrega dados) com um
único impl. A trait te dá `check()` de graça - ele empurra qualquer
mensagem de falha para um conjunto `ValidationErrors` sob o campo
nomeado - então a regra se encaixa no `validate!` e nos hooks
`after_validation` sem alterações:

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

// Agora utilizável em todo lugar:
StartsWith("acct_").passes("acct_1234")?;
// ou, em uma linha do validate!:
//   stripe_id => Required, StartsWith("acct_");
```

Uma `String` se converte em uma `ValidationMessage` que renderiza
literalmente, que é tudo de que uma app em um único idioma precisa. Para
ter a mensagem traduzida por locale, retorne uma mensagem *com chave* -
`ValidationMessage::keyed("validation-starts-with").arg("prefix", self.0).fallback(…)` -
e defina o id em `lang/<locale>/validation.ftl`. Veja
[Localização](localization.md), que também cobre a sobrescrita das
mensagens das regras embutidas e a convenção de nomenclatura
`field-<name>`.

Para lógica entre campos, implemente [`ContextualRule`] em vez disso - o
método `passes` recebe um `&FormContext` (um `HashMap<String, String>`
com os valores dos campos irmãos) junto com o valor sob teste. Para
verificações apoiadas em banco de dados, implemente [`AsyncRule`] e
use-a a partir de `after_validation_async`.

## A macro `validate!`

`validate!` executa uma cadeia de regras sobre os campos de um struct,
acumulando toda falha em um único `ValidationErrors`. É a casa idiomática
do hook síncrono entre campos, [`after_validation`](#hooks-entre-campos).

```rust
use suprnova::{validate, ValidationErrors, rules::{Required, Email, Min, Max, RequiredIf}};

fn after_validation(&self) -> Result<(), ValidationErrors> {
    // Regras contextuais leem valores irmãos de um `FormContext` que você
    // constrói - um mapa de nome de campo para seu valor em string.
    let mut ctx = std::collections::HashMap::new();
    ctx.insert("billing_type".to_string(), self.billing_type.clone());
    validate! { self =>
        email       => Required, Email;          // linha de forma obrigatória
        bio         ?: Min(10), Max(500);        // opcional: valida só se Some
        card_number ?=> RequiredIf {             // presença condicional (veja abaixo)
            other: "billing_type",
            value: "card",
        } => with ctx;
    }
}
```

Cada linha tem uma de três formas:

- **`field => Rule1, Rule2;`** - forma obrigatória. As regras rodam
  diretamente sobre `&self.field` (para `String`, `i64`, ou qualquer
  coisa que faça deref para o borrow que a regra espera).
- **`field ?: Rule1, Rule2;`** - opcional. O campo é `Option<T>`; as
  regras rodam apenas quando ele é `Some`, e são **inteiramente puladas
  em `None`**. Esta é a semântica "se estiver presente, valide"
  (`sometimes`) do Laravel.
- **`field ?=> Rule1, Rule2;`** - presença condicional. Também para um
  campo `Option<String>`, mas as regras rodam **mesmo quando `None`** (a
  ausência é tratada como a string vazia). Esta é a linha para regras
  condicionais de presença como `RequiredIf`, que precisam ser capazes de
  *reprovar um campo ausente* - o caso que `?:` não consegue expressar
  porque pula em `None`.

Uma regra contextual é seguida de `=> with $ctx` (um
`&HashMap<String, String>` com os valores dos campos irmãos). A macro é
**síncrona** - para regras assíncronas use o
[hook](#regras-assíncronas-em-solicitações) abaixo.

> **Aviso:** Uma armadilha comum: escrever `card_number ?: RequiredIf {...} => with ctx;`.
> Em uma linha `?:`, `None` pula todas as regras, então `RequiredIf` nunca
> consegue reprovar um campo ausente. Use `?=>` para qualquer regra que
> precise disparar na ausência.

## Hooks entre campos

`FormRequest` executa dois hooks entre campos depois das regras por campo
derivadas, tanto no fluxo normal quanto no de Precognition. `extract()`
executa os estágios em ordem - `validate()` derivado, depois
`after_validation`, depois `after_validation_async` - e **aborta no
primeiro estágio que falhar**.

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

> **Nota:** Sobrescrever hooks exige um `impl FormRequest` escrito à mão - o
> attribute `#[request]` e o `#[derive(FormRequest)]` geram o seu próprio
> impl (vazio), então eles servem apenas para o caso comum, sem
> sobrescrita.

### Regras assíncronas em solicitações

A macro `validate!` não consegue entrelaçar `.await`, então regras
apoiadas em banco de dados rodam em `after_validation_async` - o estágio
final da validação, que `extract()` chama automaticamente. É aqui que
[`Unique`](#a-regra-unique) e qualquer `AsyncRule` customizada
participam da validação automática da solicitação; não é preciso ligar
nada manualmente em cada handler.

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

Como o estágio assíncrono só roda depois que os estágios síncronos
passam, um valor malformado (um email sintaticamente inválido) nunca
chega à consulta `Unique` no banco de dados.

## A regra `Unique`

`Unique` verifica que um valor ainda não existe em uma tabela. Construa-a
com `Unique::new(table, column)` e refine com a API fluente:

```rust
use suprnova::Unique;

// o email precisa ser único, ignorando a linha que está sendo editada
Unique::new("users", "email").ignore(current_user_id)

// email único *por tenant*, comparado sem diferenciar maiúsculas
Unique::new("users", "email")
    .where_eq("tenant_id", tenant_id)
    .case_insensitive()
```

| Método do builder | Efeito |
|----------------|--------|
| `.ignore(id)` | exclui a linha cujo `id` é igual a `id` (caso de editar a si mesmo) |
| `.ignore_with_column(col, id)` | exclui por uma coluna de chave que não seja `id` |
| `.where_eq(col, value)` | delimita a verificação às linhas onde `col = value`; várias chamadas se combinam com AND |
| `.case_insensitive()` | compara com `LOWER(col) = LOWER(?)` |

A tabela, a coluna, a chave de exclusão e toda coluna de `where_eq` são
validadas contra uma allowlist de identificadores antes de chegarem à
string SQL; o valor sob teste e todos os valores de escopo são parâmetros
vinculados.

### Unique é consultivo - a restrição do banco de dados é a garantia

`Unique` executa um `SELECT COUNT(*)` **antes** da escrita, então carrega
uma corrida inevitável entre o momento da verificação e o momento do uso:
duas solicitações concorrentes podem ambas passar na verificação e depois
ambas inserir. A regra `unique` do Laravel tem exatamente a mesma
propriedade. A **única** garantia real é uma restrição `UNIQUE` (ou um
índice único) sobre a coluna na sua migração.

Use as três juntas:

1. **A regra consultiva** - uma mensagem rápida e amigável de "esse email
   já está em uso" antes do envio (e assim o Precognition consegue
   validar o campo).
2. **A restrição `UNIQUE`** - a proteção autoritativa contra a corrida.
3. **`FrameworkError::from_unique_violation`** - no local da escrita,
   mapeie a violação de restrição que o perdedor de uma corrida recebe de
   volta para o mesmo 422 limpo, em vez de vazar um 500:

```rust
use suprnova::FrameworkError;

// `users.email` tem uma restrição UNIQUE na migração.
let user = new_user
    .insert(db)
    .await
    .map_err(|e| FrameworkError::from_unique_violation(
        "email",
        "That email address is already registered.",
        e,
    ))?;
```

`from_unique_violation` retorna um erro `Validation` 422 quando o erro do
banco de dados é uma violação de restrição de unicidade, e repassa
qualquer outro erro sem alterações (MySQL, Postgres e SQLite são todos
reconhecidos).

## Autorização assíncrona

`FormRequest::authorize(&Request) -> bool` roda **antes** de o corpo ser
parseado, então consegue rejeitar solicitações não autorizadas sem ler o
payload. Ele é síncrono por design: nesse ponto a solicitação ainda
segura o corpo em streaming, então o hook não pode usar `.await`. A
autorização que precisa consultar o banco de dados ou uma policy
assíncrona pertence a um destes lugares, não a `authorize`:

- **Middleware** - roda antes de `extract()`, é `async`, e faz
  short-circuit retornando `Err(response)` (veja
  [Middleware](middleware.md)). O lugar certo para "este usuário sequer
  tem permissão de alcançar esta rota?".
- **O Gate** - chame `Gate::allows_async` / `Gate::authorize_async` no
  handler depois de já ter em mãos o usuário autenticado e o recurso
  (veja [Autorização](authorization.md)).
- **`after_validation_async`** - para uma verificação de autorização que
  depende do corpo parseado da solicitação, execute-a no hook assíncrono
  junto com as suas outras regras assíncronas.

## Notas de design

- **Validação parcial.** Um `FormRequest` desserializa para um struct
  tipado antes de a validação rodar, então o struct *é* o schema: um
  campo que pode estar ausente precisa ser `Option<T>`. É também isso que
  permite ao Precognition validar um payload parcial - torne opcionais os
  campos que um rascunho pode omitir.
- **Mensagens das regras.** As regras embutidas retornam mensagens com
  chave (`validation-min` mais seus argumentos e um fallback em inglês),
  resolvidas através do catálogo na fronteira de serialização. Traduza ou
  reescreva qualquer uma delas definindo o mesmo id em
  `lang/<locale>/validation.ftl` - sem precisar envolver a regra. Veja
  [Localização](localization.md).
- **`Min` / `Max` / `Between`** são regras de comprimento de string
  (contado em valores escalares Unicode). Para limites numéricos, valide
  com `#[validate(range(...))]` no derive ou com uma regra customizada -
  as regras de comprimento não são comparações de valor.

## Resumo

| Tarefa | API |
|------|-----|
| Regras por campo | `#[validate(...)]` no `FormRequest` (veja Solicitações) |
| Regras compostas / entre campos | `validate! { self => ... }` |
| Opcional "se estiver presente" | `field ?: Rule;` |
| Opcional condicionalmente obrigatório | `field ?=> Rule => with ctx;` |
| Regra assíncrona / apoiada em BD | `after_validation_async` + `AsyncRule::check_async` |
| Unicidade | `Unique::new(t, c)` + restrição `UNIQUE` + `from_unique_violation` |
| Autorização assíncrona | middleware / `Gate::*_async` / `after_validation_async` |

## Próximos passos

- [Solicitações](requests.md) - a superfície `#[request]` /
  `#[derive(FormRequest)]`, o caminho do dia a dia da validação derivada
- [Objetos de dados](data.md) - `#[derive(Data, Validate)]` para um
  struct que é ao mesmo tempo uma solicitação de entrada e um DTO de
  saída
- [Modelo de erros](error-model.md) - como `ValidationErrors` vira o
  corpo JSON 422, ao lado de todos os outros caminhos de erro
- [Localização](localization.md) - traduzindo mensagens de regras, a
  convenção `field-<name>`, e `ValidationMessage`s com chave
- [Autorização](authorization.md) - `Gate`, `Policy`, e onde a
  autorização se encaixa em relação à validação
- [Middleware](middleware.md) - o lugar certo para verificações de "esta
  solicitação sequer pode passar" que precisam de `.await`
