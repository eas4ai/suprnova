# Localização

A localização no Suprnova é um único módulo com quatro faces:
catálogos de mensagens no servidor, erros de validação que chegam já
traduzidos, os *mesmos* bytes de catálogo entregues ao navegador, e
formatação de número, data e lista com reconhecimento de locale. O
formato de mensagem é o [Fluent](https://projectfluent.org) - o
`.ftl` da Mozilla, o mesmo que o Firefox usa - e o subsistema inteiro
vem ligado por padrão atrás da feature `localization`.

O tour mais curto possível. Escreva um catálogo:

```ftl
# lang/en/app.ftl
welcome = Welcome to { $app }!
```

```ftl
# lang/es/app.ftl
welcome = ¡Bienvenido a { $app }!
```

Use a partir de um handler:

```rust
use suprnova::{__, handler, HttpResponse, Request, Response};

#[handler]
pub async fn greet(_req: Request) -> Response {
    Ok(HttpResponse::text(__!("welcome", app: "Suprnova")))
}
```

Uma solicitação com `Accept-Language: es` recebe a string em
espanhol, porque o `LocaleMiddleware` resolveu o locale antes do seu
handler rodar. Nada mais no handler muda - nenhum parâmetro de locale
é passado adiante, nenhum `&Translator` na assinatura.

## Por que localização

Três razões para isso ser uma preocupação do framework em vez de um
crate que você escolhe:

- **Mensagens de validação são strings do framework, não suas.** "The
  email field is required." é emitida bem no fundo de `Rule::passes`,
  longe de qualquer código que você possui. A menos que o framework
  carregue uma costura de tradução, um app em espanhol lança erros de
  validação em inglês - ou você envolve cada regra à mão. As regras
  embutidas do Suprnova retornam mensagens *chaveadas*; você as
  traduz soltando um arquivo `.ftl`, e nunca toca nas regras.
- **O navegador precisa das mesmas strings.** Um app Inertia
  renderiza metade do seu texto em Rust e metade em Svelte/React/Vue.
  Dois sistemas de tradução significam dois formatos de arquivo, dois
  workflows de review, e duas chances da mesma frase divergir. O
  Suprnova serve o catálogo exato que o servidor resolveu a partir de
  `/_suprnova/lang/<locale>.ftl`, e os starter kits o fazem parse com
  `@fluent/bundle` - um único conjunto de arquivos, uma única fonte
  da verdade.
- **Plurais e formatos são dados CLDR, não concatenação de string.**
  O inglês tem duas categorias de plural, o russo e o polonês quatro,
  o árabe seis. Um número é `1,234.56` em `en-US` e `1.234,56` em
  `de-DE`. O Fluent seleciona sobre categorias de plural CLDR e o
  ICU4X faz a formatação, então nenhum dos dois é algo que você
  escreve à mão por locale.

Desligar a feature (`--no-default-features`) é suportado: o módulo
de localização não compila, e a validação renderiza suas strings de
fallback em inglês embutidas. Nada mais muda de forma.

## Layout de arquivos

Os catálogos vivem sob `lang/`, um diretório por locale:

```
myapp/
├── lang/
│   ├── en/
│   │   ├── app.ftl
│   │   └── validation.ftl
│   └── es/
│       ├── app.ftl
│       └── validation.ftl
├── src/
└── frontend/
```

As regras:

- **Um nome de diretório é um locale BCP-47** - `en`, `en-GB`,
  `pt-BR`, `zh-Hans`. Um diretório cujo nome não faz parse é pulado
  com um `warn!` em vez de falhar o boot.
- **Todo `.ftl` em um diretório de locale se mescla em um único
  catálogo**, em ordem alfabética de nome de arquivo. Divida por
  feature (`auth.ftl`, `billing.ftl`, `emails.ftl`) à vontade; ids de
  mensagem são globais dentro do locale, então `auth.ftl` e
  `billing.ftl` não podem definir o mesmo id.
- **O catálogo de validação em inglês do próprio framework carrega
  primeiro**, em todo bundle de locale. Seus arquivos carregam por
  cima dele, e uma definição posterior vence. Esse é o mecanismo de
  override inteiro: defina `validation-min` em `lang/es/validation.ftl`
  e o bundle em espanhol usa o seu.
- **A raiz é `lang_path()`** - `<APP_BASE_PATH>/lang`. Defina
  `APP_BASE_PATH` quando o binário roda a partir de outro lugar que
  não a raiz do projeto (uma unit do systemd, um container com um
  `WorkingDirectory` diferente), ou chame `use_lang_path("…")` para
  mover só o diretório `lang`. Veja
  [Variáveis de ambiente](env-vars.md).
- **Um diretório `lang/` ausente não é um erro.** Um app novo
  precisa bootar, então o translator sobe com o catálogo em inglês
  embutido e nada mais. Um `.ftl` *malformado* é uma história
  diferente: erros de parse falham o boot, nomeando o arquivo e o que
  o parser objetou, porque um catálogo silenciosamente meio-carregado
  é pior que um processo parado.
- **Em `local` e `development`, os catálogos fazem hot-reload.** Cada
  solicitação faz stat de `lang/` e só reparseia quando algo de fato
  mudou, então editar um `.ftl` aparece no próximo refresh. Produção
  nunca refaz o stat; os catálogos são lidos uma vez no boot.

## FTL em cinco minutos

Fluent é um formato pequeno. Esta seção é tudo que você precisa para
um app típico.

**Mensagens** são pares `id = value`. Ids são kebab-case por
convenção (os do próprio framework são), valores correm até o fim da
linha, e linhas de continuação indentadas são unidas:

```ftl
# Um comentário. Vinculado à mensagem abaixo dele.
sign-in = Sign in
password-hint =
    Use at least 12 characters. A passphrase of a few
    ordinary words beats a short string of symbols.
```

**Argumentos** são placeables `{ $name }`. Você os fornece no momento
da chamada; argumentos ausentes são um erro, não uma string vazia
(`Lang::get` então percorre sua cadeia - veja
[A facade `Lang`](#a-facade-lang)):

```ftl
greeting = Olá, { $name }!
invoice-line = { $qty } × { $item }
```

**Termos** começam com `-`, são privados ao catálogo, e existem para
que um nome de marca ou uma frase repetida viva em um único lugar:

```ftl
-product-name = Suprnova
about = About { -product-name }
footer = © 2026 { -product-name }. All rights reserved.
```

**Seletores** são o condicional do Fluent. O valor do seletor é
comparado contra chaves de variante; exatamente uma variante é
marcada como padrão com `*`:

```ftl
cart-summary =
    { $count ->
        [0] Your cart is empty.
        [one] One item in your cart.
       *[other] { $count } items in your cart.
    }
```

`[0]` casa com o número literal zero. `[one]` e `[other]` são
**categorias de plural CLDR**, resolvidas para o locale do bundle -
que é onde o Fluent ganha o seu lugar. O inglês tem duas categorias;
o russo tem quatro, e um tradutor russo escreve as quatro sem você
mudar uma linha de Rust:

```ftl
# lang/ru/app.ftl
unread-messages =
    { $count ->
        [one] У вас { $count } непрочитанное сообщение.
        [few] У вас { $count } непрочитанных сообщения.
        [many] У вас { $count } непрочитанных сообщений.
       *[other] У вас { $count } непрочитанного сообщения.
    }
```

O CLDR atribui `1`, `21`, `31` a `one`; `2`–`4`, `22`–`24` a `few`;
`0`, `5`–`20`, `25`–`30` a `many`; e frações a `other`. A mesma
chamada `__!("unread-messages", count: 22)` renderiza corretamente
em inglês, russo, polonês e árabe, porque a seleção de categoria é
dado, não código.

**Sempre coloque o `*` em `other`.** É a única categoria que o CLDR
define para todo locale, então é a única variante garantida a
existir - e o padrão é para onde um valor de seletor não casado cai,
incluindo qualquer contagem não-inteira. Marcar `*[many]` (ou
qualquer outra categoria) como padrão manda frações para texto
escrito para números inteiros.

> **Passe contagens como números.** `__!("unread-messages", count: 3)`
> envia um número JSON e seleciona uma categoria de plural.
> `count: "3"` envia uma string, que só pode casar com uma chave de
> variante literal - ela vai cair no seu padrão `*[other]`. Essa é a
> armadilha do FTL que mais vale memorizar.

**Funções** são chamadas dentro de placeables. Duas estão
registradas: `NUMBER()` (embutida do Fluent) e `DATETIME()` (do
Suprnova):

```ftl
score = Your score is { NUMBER($points) } out of { NUMBER($total) }.
published = Published { DATETIME($when, dateStyle: "medium") }
```

Veja [Formatação com reconhecimento de locale](#formatação-com-reconhecimento-de-locale)
para as duas.

**Uma limitação deliberada:** o Suprnova resolve só *valores* de
mensagem flat. A sintaxe de attribute do Fluent (`login .placeholder = …`)
faz parse mas não é endereçável através de `Lang::get`, então mantenha
um id por string: `login-placeholder`, não `login.placeholder`. Ids
são um namespace flat por locale - prefixe-os (`auth-login-title`,
`billing-invoice-due`) em vez de buscar uma hierarquia que o resolver
não tem.

## A facade `Lang`

`Lang` é o ponto de entrada do lado do servidor. Todo método lê o
**locale atual**, que o middleware vinculou para esta solicitação.

| Método | Retorna | Notas |
|---|---|---|
| `Lang::get(key)` | `String` | Infalível. Roda a cadeia de fallback, depois retorna a própria chave |
| `Lang::get_with(key, args)` | `String` | O mesmo, com argumentos |
| `Lang::try_get(key)` | `Result<String, FrameworkError>` | Erra em vez de degradar |
| `Lang::try_get_with(key, args)` | `Result<String, FrameworkError>` | O mesmo, com argumentos |
| `Lang::has(key)` | `bool` | Se a chave resolve para o locale atual, ou em algum ponto da sua cadeia de fallback |
| `Lang::locale()` | `Locale` | O locale atual |
| `Lang::set_locale(locale)` | `()` | Muda para o resto desta solicitação |
| `Lang::available_locales()` | `Vec<Locale>` | Todo locale com um catálogo carregado |

```rust
use suprnova::{Lang, Locale, TranslateArgs};

let subject = Lang::get("password-reset-subject");

let mut args = TranslateArgs::new();
args.insert("name".into(), serde_json::json!("Ada"));
args.insert("count".into(), serde_json::json!(3));
let body = Lang::get_with("unread-messages", args);

if Lang::has("beta-banner") {
    // Só alguns locales lançam o texto do banner.
}

let locales: Vec<String> = Lang::available_locales()
    .iter()
    .map(Locale::as_str)
    .collect();
```

`TranslateArgs` é um map ordenado de `String` para
`serde_json::Value`, ambos reexportados da raiz do crate. Argumentos
Fluent são strings e números; outras formas JSON são transformadas em
string.

### A cadeia de fallback

`Lang::get` nunca falha, e nunca retorna uma string vazia. Em ordem:

1. O catálogo do **locale atual**.
2. Seus **parents de fallback configurados** (veja
   [Cadeias de fallback](#cadeias-de-fallback)), percorridos
   transitivamente, se algum estiver configurado - `pt-PT` antes de
   `pt-BR` antes do que quer que `pt-BR` mesmo nomeie como parent, e
   assim por diante.
3. O catálogo do **locale de fallback** (`APP_FALLBACK_LOCALE`,
   padrão `en`), a menos que já tenha aparecido antes nesta cadeia.
4. A **própria chave**, mais um `tracing::warn!` por par
   `(locale, key)` ausente - uma vez, não uma vez por solicitação,
   então uma chave ausente em um hot path não afoga seus logs.

O passo 4 é o motivo de uma tradução ausente renderizar
`checkout-submit` no botão em vez de um botão em branco: uma string
visivelmente errada é um bug report esperando para acontecer,
enquanto uma vazia é um mistério.

Quando você prefere saber a degradar, use os irmãos `try_*`. Eles
rodam os passos 1 a 3 e retornam `Err` em vez de fazer o passo 4:

```rust
use suprnova::Lang;

// Uma chave ausente aqui significa um email quebrado - falhe o job,
// não envie uma mensagem com uma chave crua na linha de assunto.
let subject = Lang::try_get("invoice-paid-subject")?;
```

### A macro `__!`

`__!` é o atalho para a memória muscular de quem vem do Laravel. Sem
argumentos ela chama `Lang::get`; com argumentos nomeados ela
constrói um `TranslateArgs` e chama `Lang::get_with`:

```rust
use suprnova::__;

let plain = __!("welcome-back");
let greeted = __!("greeting", name: "Ada");
let counted = __!("unread-messages", name: "Ada", count: 3);
```

Valores de argumento são qualquer coisa que converte em um
`serde_json::Value` - `&str`, `String`, inteiros, floats, `bool`. A
macro é exportada na raiz do crate, então `suprnova::__!("welcome-back")`
funciona sem o import quando você preferir não trazer `__` para o
escopo.

## Cadeias de fallback

`APP_FALLBACK_LOCALE` é uma única rede global sob todo locale. Às
vezes isso não é suficiente: português europeu e português
brasileiro compartilham quase tudo e divergem em um punhado de
palavras (`ficheiro`/`arquivo`, `utilizador`/`usuário`, `tu`/`você`),
e manter dois catálogos completos significa que toda string nova tem
que ser escrita duas vezes. Um **parent de fallback** deixa `pt-PT`
herdar de `pt-BR` antes de `pt-BR` cair mais para trás até o
`fallback_locale` global - então `lang/pt-PT/` só precisa guardar as
strings que são de fato diferentes.

### Configurando parents

Uma variável de ambiente, pares `child=parent` separados por vírgula:

```env
APP_LOCALE_PARENTS=pt-PT=pt-BR
```

Ou o builder, uma chamada por par, encadeável:

```rust
use suprnova::{Config, Locale, LocalizationConfig};

pub fn register_all() {
    let localization = LocalizationConfig::from_env()
        .expect("APP_LOCALE / APP_FALLBACK_LOCALE must be valid BCP-47")
        .parent(
            Locale::parse("pt-PT").expect("valid locale"),
            Locale::parse("pt-BR").expect("valid locale"),
        );

    Config::register(localization);
}
```

Os dois caminhos alimentam o mesmo map
(`LocalizationConfig::parents`), e os dois são validados no boot, não
no momento da solicitação:

- Um par sem `=`, ou com child ou parent vazio, é uma entrada
  malformada de `APP_LOCALE_PARENTS` - o boot falha nomeando o
  segmento ruim.
- Um locale inválido como BCP-47 em qualquer lado do par falha da
  mesma forma.
- Nomear o mesmo child duas vezes é config ambígua, não last-wins -
  o boot falha nomeando o child duplicado.
- **Um ciclo falha o boot.** O erro soletra o ciclo: dois locales se
  nomeando mutuamente (`pt-PT=pt-BR,pt-BR=pt-PT`) produz
  `` `pt-PT` -> `pt-BR` -> `pt-PT` ``. Um locale se nomeando como seu
  próprio parent (`pt-PT=pt-PT`) é o mesmo caso em miniatura -
  `` `pt-PT` -> `pt-PT` ``. (Dois caminhos de código levantam esse
  erro: o parse de `APP_LOCALE_PARENTS` - então qualquer app cuja
  config passa por `LocalizationConfig::from_env()` falha no
  carregamento da config - e o carregamento de catálogo do
  `FluentTranslator`, que captura um map cíclico construído
  programaticamente com `.parent(...)`. Só um app que constrói sua
  config inteiramente à mão *e* vincula seu próprio `Translator`
  customizado em `bootstrap_fn` escapa dos dois; o walk do `Lang` é
  guardado independentemente e ainda termina com segurança ali, só
  não recebe o erro alto de boot.)

O `.parent(child, parent)` do builder é last-write-wins para um child
repetido - uma chamada posterior sobrescrevendo uma anterior é só um
override posterior, não o caso de input ambíguo contra o qual
`APP_LOCALE_PARENTS` guarda.

### Ordem de resolução

Uma cadeia pode ter mais de um salto: `pt-PT` nomeia `pt-BR` como seu
parent, e `pt-BR` pode por sua vez nomear um parent próprio.
`Lang::get` / `try_get` / `get_with` / `try_get_with` / `has` todos
percorrem a coisa inteira, locale atual primeiro:

1. O catálogo do **locale atual**.
2. Seu **parent configurado**, depois o parent configurado *daquele*
   locale, transitivamente, até um locale sem parent configurado ser
   alcançado.
3. O **`fallback_locale`** global (`APP_FALLBACK_LOCALE`), a menos
   que já tenha aparecido antes na cadeia - incluindo o caso comum
   onde é só o próprio locale atual (o padrão `en`/`en`).

`Lang::get` / `Lang::get_with` caem para a própria chave se nada na
cadeia a resolver, exatamente como [A cadeia de fallback](#a-cadeia-de-fallback)
descreve; `Lang::try_get` / `Lang::try_get_with` retornam `Err`, e
`Lang::has` retorna `false`. Esse walk roda dentro da própria facade
`Lang`, então funciona para **qualquer** `Translator` - o
`FluentTranslator` empacotado, ou um driver que você escreve.

### Um exemplo executável

```
myapp/
├── lang/
│   ├── pt-BR/
│   │   ├── app.ftl
│   │   └── validation.ftl
│   └── pt-PT/
│       └── app.ftl
├── src/
└── frontend/
```

```ftl
# lang/pt-BR/app.ftl
welcome = Bem-vindo ao { $app }!
file-label = Arquivo
```

```ftl
# lang/pt-PT/app.ftl
file-label = Ficheiro
```

```rust
use suprnova::__;

// Uma solicitação que resolveu para `pt-PT`.
assert_eq!(__!("file-label"), "Ficheiro");                    // override do próprio pt-PT
assert_eq!(
    __!("welcome", app: "Suprnova"),
    "Bem-vindo ao Suprnova!"                                  // herdado de pt-BR
);
```

`lang/pt-PT/` nunca define `welcome` - não precisa. `file-label` é
uma diferença genuína de uma palavra entre os dois catálogos, então
é o único id que ganha um arquivo.

### Catálogos servidos são achatados

O endpoint `/_suprnova/lang/pt-PT.ftl` (veja
[O endpoint de catálogo](#o-endpoint-de-catálogo)) nunca pede ao
navegador para saber que `pt-BR` existe. O `FluentTranslator`
pré-mescla a cadeia inteira em um único resource por locale no
momento do carregamento - o catálogo embutido do framework no fundo
para locales `en`/`en-*`, depois a cadeia de parents configurada,
depois os próprios arquivos do locale - e serve *isso*, já achatado.
Busque `pt-PT.ftl` e a resposta carrega `welcome` e `file-label`
juntos, em uma única solicitação, sem lógica de cadeia do lado do
cliente. `?v=<hash>` ainda nomeia um único resource imutável; o hash
simplesmente agora cobre strings puxadas de `pt-BR` também.

**O achatamento cobre só os parents configurados** - ele nunca
alcança além deles até o `fallback_locale`. O catálogo servido de
`pt-PT` inclui as strings de `pt-BR` porque `pt-BR` é um *parent
configurado*; ele não inclui as strings de `en` só porque `en`
acontece de ser o fallback global. O campo `fallback` do
`LocaleShare` sempre nomeia o `fallback_locale` terminal, não afetado
por nada disso - ele diz ao frontend onde o walk no nível da facade
do `Lang` acabaria pousando, não o que já está no arquivo que ele
acabou de buscar.

### Regras de merge de arquivo delta

Um catálogo filho mescla sobre seu parent **no nível da AST do
Fluent**, não por concatenação textual e não por sombreamento de
mensagem inteira. A unidade de override é o *pattern*, então:

- **Um valor do child substitui o valor do parent**, na posição do
  parent no arquivo.
- **Uma entrada child com attributes mas sem valor mantém o valor do
  parent.** Retraduzir `.placeholder` não exige repetir o texto
  próprio da mensagem.
- **Attributes mesclam por nome.** Um attribute do child com o mesmo
  nome substitui o do parent, no lugar; um attribute só-do-child é
  anexado depois do próprio do parent. **Attributes que o child não
  menciona sobrevivem do parent** - sobrescrever o valor de uma
  mensagem nunca derruba silenciosamente seu `.placeholder` ou
  `.aria-label`.
- **Expressões de seletor substituem por inteiro, nunca
  variante-por-variante.** As variantes de um seletor são chaveadas
  às categorias de plural CLDR de um locale; como essas categorias
  são dependentes de locale, emendar uma variante do parent e outra
  do child poderia produzir um seletor sem a gramática de nenhum
  locale único por trás. Um child que sobrescreve um seletor de
  alguma forma precisa fornecer toda variante que quer.
- **Comentários em uma entrada sobrescrita permanecem os do parent.**
  O comentário documenta o id, e a unidade de override é o pattern,
  não o comentário.
- **Entradas só-do-child são anexadas ao final**, na própria ordem do
  child, comentários incluídos - um id que `pt-BR` nunca definiu não
  é um "override" de nada.

Termos (`-brand`) seguem a regra idêntica, com um estreitamento: o
valor de um termo nunca é opcional na sintaxe do Fluent, então o caso
"attributes-mas-sem-valor mantém o valor do parent" acima se aplica
só a mensagens - um termo do child sempre fornece um valor, e esse
valor sempre vence. Merge de attribute por nome, substituição de
pattern inteiro para o valor, e comentários que ficam com o parent se
aplicam a termos exatamente como a mensagens. Termos são rastreados
em seu próprio namespace - sobrescrever `-brand` nunca pode sombrear
uma mensagem também nomeada `brand`.

### Por que Suprnova diverge

O Laravel 13 tem exatamente um fallback: o valor de config global
único `fallback_locale`, consultado quando o array do locale atual
está faltando uma chave. Não existe o conceito de um locale herdando
de um locale irmão - `pt_PT.php` e `pt_BR.php` são dois arrays não
relacionados, e um app `pt_PT` ou duplica tudo que `pt_BR` já tem
traduzido, ou lança sem isso.

As cadeias de parent do Suprnova são a extensão do lado Rust: um
passo intermediário entre "este locale" e "o fallback global",
configurado por locale em vez de globalmente uma única vez. O
trade-off que não quisemos fazer foi empurrar essa complexidade para
o navegador - um frontend ciente de cadeia precisaria buscar
`pt-PT.ftl`, descobrir que está incompleto, buscar `pt-BR.ftl`
também, e mesclá-los do lado do cliente em JavaScript, usando regras
que teriam que casar exatamente com as do servidor. Achatar no
momento do carregamento em vez disso significa que o catálogo
servido é sempre um único arquivo completo e autocontido - o mesmo
contrato que o frontend já tinha antes das cadeias de parent
existirem, então `@fluent/bundle` e os wrappers dos kits precisaram
de zero mudanças para suportar essa feature.

## Detecção de locale

O `LocaleMiddleware` resolve um locale por solicitação e o vincula
pela duração do handler. A cadeia é orientada a config e **o
primeiro hit vence**:

1. **Sessão** - a chave `locale` na sessão, se o
   [middleware de sessão](session.md) rodou e o valor nomeia um
   locale disponível. É aqui que "o usuário escolheu Español nas
   configurações" vive.
2. **Cookie** - o cookie `locale`. Sobrevive ao logout, então uma
   escolha de idioma feita antes de entrar na conta não se perde.
3. **`Accept-Language`** - negociado contra `available_locales()` com
   `fluent-langneg`, honrando q-values. `fr-CH, es;q=0.8, en;q=0.5`
   contra os catálogos `en` + `es` resolve para `es`.
4. **`APP_LOCALE`** - o padrão configurado, quando nada acima bateu.

Um candidato que não faz parse, ou nomeia um locale sem catálogo, é
**pulado, não rejeitado**. Um usuário com um cookie `locale=zz`
velho vê o idioma padrão, não um 500. Um header `Accept-Language`
com lixo faz o mesmo. Input controlado por atacante alcança essa
cadeia em toda solicitação; ela nunca pode fazer mais que escolher um
idioma.

Conecte-o em `bootstrap.rs`, **depois** do middleware de sessão, já
que o passo 1 lê a sessão:

```rust
use std::sync::Arc;
use suprnova::{
    global_middleware, App, LocaleMiddleware, LocaleShare, SessionConfig, SessionMiddleware,
};

pub async fn register() {
    global_middleware!(SessionMiddleware::install(SessionConfig::from_env()).await);

    // Resolve o locale e o vincula para a solicitação.
    global_middleware!(LocaleMiddleware::from_env().expect("locale config"));

    // Entrega ao frontend seu locale + URL de catálogo em toda página Inertia.
    App::register_inertia_shared(Arc::new(LocaleShare));
}
```

`LocaleMiddleware::from_env()` lê `LocalizationConfig::from_env()`;
`LocaleMiddleware::new(config)` recebe um que você mesmo construiu.
Um app com scaffold já tem as duas linhas.

### Mudando o locale no meio da solicitação

`Lang::set_locale` é o `App::setLocale` do Laravel - ele reescreve o
locale da solicitação atual a partir daquele ponto em diante:

```rust
use suprnova::session::session_mut;
use suprnova::{FrameworkError, Lang, Locale};

/// O usuário acabou de trocar de idioma em um formulário de configurações.
pub fn switch_language(choice: &str) -> Result<(), FrameworkError> {
    let locale = Locale::parse(choice)?;
    Lang::set_locale(locale);                       // esta solicitação
    session_mut(|s| s.put("locale", choice));       // toda solicitação depois
    Ok(())
}
```

Note as duas metades: `set_locale` afeta *esta* solicitação (então a
mensagem flash do redirecionamento já está em espanhol), e a escrita
de sessão é o que a cadeia de detecção lê na *próxima*.

### Fora de uma solicitação

Comandos de console, workers de fila e tasks agendadas não têm
solicitação nem middleware. Ali, `Lang::set_locale` escreve um
override global ao processo que `Lang::locale()` consulta antes de
cair de volta para `APP_LOCALE`:

```rust
use suprnova::{command, FrameworkError, Lang, Locale, Mail};

use crate::mail::Digest;
use crate::models::user::User;

#[command(name = "mail:digest", description = "Send the weekly digest")]
pub async fn send_digest(_args: Vec<String>) -> Result<(), FrameworkError> {
    for user in User::query().get().await? {
        // A preferência armazenada de cada usuário, pela duração do email dele.
        Lang::set_locale(Locale::parse(&user.locale)?);
        Mail::to(&user.email).send(Digest::for_user(&user)).await?;
    }
    Ok(())
}
```

Como aquele override é global ao processo em vez de task-local,
defina-o no topo de cada unidade de trabalho como acima - não conte
com ele permanecer inalterado através de um `.await` com o qual
outra task poderia interlear.

## Configuração

Três variáveis de ambiente. `APP_LOCALE` e `APP_FALLBACK_LOCALE`
usam `en` como padrão; `APP_LOCALE_PARENTS` usa vazio como padrão -
nenhum override por locale, só `fallback_locale` se aplica:

```env
APP_LOCALE=en
APP_FALLBACK_LOCALE=en
# APP_LOCALE_PARENTS=pt-PT=pt-BR
```

Todo o resto é código, em `LocalizationConfig`. Ele se registra como
toda outra config tipada - no seu `config::register_all`, que roda
antes do boot:

```rust
// src/config/mod.rs
use suprnova::{Config, Detect, Locale, LocalizationConfig};

pub fn register_all() {
    let localization = LocalizationConfig::from_env()
        .expect("APP_LOCALE / APP_FALLBACK_LOCALE must be valid BCP-47")
        .default_locale(Locale::parse("es").expect("valid locale"))
        .use_isolating(true)                                // veja a nota de divergência
        .detection(vec![Detect::Session, Detect::Header])   // ignora o cookie
        .session_key("preferred_locale")
        .cookie_name("lang")
        .parent(                                            // veja Cadeias de fallback
            Locale::parse("pt-PT").expect("valid locale"),
            Locale::parse("pt-BR").expect("valid locale"),
        );

    Config::register(localization);
}
```

- `default_locale` / `fallback_locale` - sobrescrevem `APP_LOCALE` e
  `APP_FALLBACK_LOCALE` a partir do código. Um valor malformado em
  qualquer um dos dois falha o boot em vez de silenciosamente virar
  `en`.
- `use_isolating` - marcas de isolamento Unicode ao redor de
  interpolações. Desligado por padrão; ligue quando você lançar um
  locale RTL.
- `detection` - a cadeia, em ordem. Remover `Detect::Cookie` significa
  que uma escolha de idioma só vive na sessão; remover
  `Detect::Header` significa que a preferência do navegador é
  ignorada por completo.
- `session_key` / `cookie_name` - renomeiam as duas buscas.
- `parents` - parents de fallback por locale (`child -> parent`),
  percorridos antes de `fallback_locale` quando uma chave está
  ausente do catálogo do child; mesmo formato de
  `APP_LOCALE_PARENTS`. Adicione um com `.parent(child, parent)` -
  encadeável, last write wins para um child repetido. Veja
  [Cadeias de fallback](#cadeias-de-fallback) para o contrato completo
  (validação no boot, ordem de resolução, achatamento do catálogo
  servido).

O boot vincula um `Arc<dyn Translator>` no contêiner. Se seu app já
vinculou um, o framework o deixa em paz - o que é como você
substitui um translator seu próprio sem fazer fork de nada:

```rust
// src/bootstrap.rs
use std::sync::Arc;
use suprnova::{App, FluentTranslator, LocalizationConfig, Translator};

pub async fn register() {
    let config = LocalizationConfig::from_env().expect("locale config");
    let translator =
        FluentTranslator::from_dir("./catalogs", &config).expect("load catalogs");
    App::bind::<dyn Translator>(Arc::new(translator));
}
```

`Translator` é a costura de extensão: `translate`, `has`,
`available_locales`, `catalog`, `reload`. Um driver está disponível
(`FluentTranslator`), e um backend novo é um driver novo - não um
fork da superfície.

## Mensagens de validação traduzidas

Toda regra embutida retorna uma mensagem **chaveada**: uma chave de
catálogo, os argumentos que a mensagem precisa, e um fallback em
inglês. A tradução acontece uma vez, na fronteira de serialização -
`ValidationErrors::to_json` e o error bag do Inertia - nunca dentro
da regra. As regras ficam puras, e o subsistema inteiro é compilado
para fora quando não usado.

As chaves seguem uma convenção:

| Formato | Exemplo | Usado para |
|---|---|---|
| `validation-<rule>` | `validation-min`, `validation-required-if` | Um por regra embutida, kebab-cased |
| `field-<name>` | `field-email` | Um nome humano para um campo |
| `validation-invalid-data` | - | O banner de nível superior "The given data was invalid." |

Para traduzi-las, defina os ids que você se importa em qualquer
arquivo `.ftl` sob o locale alvo:

```ftl
# lang/es/validation.ftl
validation-invalid-data = Los datos proporcionados no son válidos.
validation-required = El campo { $field } es obligatorio.
validation-email = El campo { $field } debe ser una dirección de correo válida.
validation-min = El campo { $field } debe tener al menos { $min } caracteres.
validation-confirmed = La confirmación del campo { $field } no coincide.
```

`$field` está sempre disponível. Os próprios parâmetros de cada
regra são passados sob os nomes que carregam no catálogo em inglês
do framework - `$min`, `$max`, `$other`, `$value` - e
`framework/src/localization/catalogs/en/validation.ftl` é a lista
canônica de ids e argumentos. Copie os ids que você precisa de lá;
você nunca precisa sobrescrever todos eles.

Sobrescrever funciona por locale e por chave. Definir
`validation-min` em `lang/en/validation.ftl` substitui o texto em
inglês do framework para aquela única regra e deixa o resto em paz.

### Nomes de campo

Interpolar um nome de coluna cru produz "The email_address field is
required." A convenção `field-<name>` corrige isso:

```ftl
# lang/en/validation.ftl
field-email_address = email address
field-dob = date of birth
```

Antes de renderizar, o translator busca `field-<name>` para o locale
atual. Um hit é passado como `$field`; um miss cai de volta para o
nome do campo com underscores virando espaços. Então o arquivo acima
só é necessário para os nomes que humanizam mal.

### Regras customizadas

`Rule::passes` retorna `Result<(), ValidationMessage>`. Uma mensagem
chaveada participa da tradução:

```rust
use suprnova::{Rule, ValidationMessage};

pub struct StartsWith(pub &'static str);

impl Rule for StartsWith {
    fn passes(&self, value: &str) -> Result<(), ValidationMessage> {
        if value.starts_with(self.0) {
            Ok(())
        } else {
            Err(ValidationMessage::keyed("validation-starts-with")
                .arg("prefix", self.0)
                .fallback(format!("must start with {}", self.0)))
        }
    }
}
```

```ftl
# lang/en/validation.ftl
validation-starts-with = The { $field } field must start with { $prefix }.
```

Uma string simples ainda funciona, e é a resposta certa para uma
mensagem que só vai existir em um idioma:

```rust
Err("must start with acct_".into())   // sem chave: renderizada verbatim
```

Mensagens sem chave pulam a tradução por completo, o que é o que
mantém regras customizadas existentes compilando e se comportando
exatamente como antes.

### O fluxo do derive

Erros de `#[derive(Validate)]` também são chaveados. O código de erro
do crate `validator` vira `validation-<code>` com underscores virando
hifens, e todo param que o validator anexa vira um argumento de
mensagem - com duas exceções reservadas, `value` e `other`, que são
sempre descartadas. As duas carregam o *valor* de fato de um campo em
vez de metadados sobre a regra: `value` é o input ecoado sob teste, e
`other` (definido por `must_match`, a regra canônica de confirmação
de senha) é o valor do campo irmão. Nenhum dos dois é jamais
entregue ao catálogo, então nenhum override `.ftl` - não importa
como fraseie `validation-must-match` - pode interpolar um segredo
submetido em um corpo de resposta 422. Então uma falha de
`#[validate(email)]` resolve `validation-email` como a regra escrita
à mão faz, e um locale que traduz uma traduz as duas.

## O frontend

O navegador recebe os mesmos bytes que o servidor resolveu. Nada é
retraduzido, reexportado, ou mantido em sincronia à mão.

### O endpoint de catálogo

```
GET /_suprnova/lang/es.ftl              → 200 text/plain, ETag: "<hash>"
GET /_suprnova/lang/es.ftl?v=<hash>     → 200 + Cache-Control: public,
                                          max-age=31536000, immutable
GET /_suprnova/lang/es.ftl              → 304 quando If-None-Match casa
GET /_suprnova/lang/zz.ftl              → 404 (catálogo não existe)
```

O corpo é o catálogo mesclado para aquele locale - mensagens do
framework primeiro, depois sua cadeia de parent de fallback
configurada se houver (veja
[Cadeias de fallback](#cadeias-de-fallback)), depois seus arquivos na
ordem de carregamento. `ETag` é o hash do conteúdo. Peça por um hash
específico com `?v=` e a resposta é cacheável como imutável para
sempre, porque aquela URL só pode significar uma coisa; peça sem ele
e você recebe revalidação em vez disso. Como `/_suprnova/health`, o
caminho é isento da chain de middleware: precisa responder antes de
um locale ter sido resolvido, e não carrega dado de usuário nenhum.

### A prop compartilhada

`LocaleShare` é um `InertiaSharedData` que o framework lança.
Registrado em `bootstrap.rs` (veja
[Detecção de locale](#detecção-de-locale)), ele adiciona uma prop a
toda página Inertia:

```json
{
  "lang": {
    "locale": "es",
    "fallback": "en",
    "catalog": {
      "url": "/_suprnova/lang/es.ftl?v=9f2c1ae4",
      "hash": "9f2c1ae4"
    }
  }
}
```

`catalog` é `null` quando nenhum translator está vinculado - a prop
compartilhada nunca falha a renderização de uma página.

### Os wrappers dos kits

Cada starter kit lança um wrapper de ~100 linhas que lê aquela prop,
busca o catálogo uma vez, constrói um bundle `@fluent/bundle`, e
expõe `t()`. Chame `initLang` uma vez no seu ponto de entrada Inertia
(apps com scaffold já fazem isso):

```ts
// frontend/src/main.ts
import { createInertiaApp } from '@inertiajs/svelte'
import { mount } from 'svelte'
import { initLang } from './lib/lang.svelte'

createInertiaApp({
  resolve: (name) => { /* … unchanged … */ },
  async setup({ el, App, props }) {
    await initLang(props.initialPage)
    mount(App, { target: el!, props })
  },
})
```

Depois, em componentes:

```svelte
<!-- Svelte 5 -->
<script lang="ts">
  import { t, currentLocale } from '../lib/lang.svelte'
</script>

<h1>{t('welcome', { app: 'Suprnova' })}</h1>
<p>{currentLocale()}</p>
```

```tsx
// React 19
import { useLang } from '../lib/lang'

export default function Home() {
  const { t, locale } = useLang()
  return <h1>{t('welcome', { app: 'Suprnova' })}</h1>
}
```

```vue
<!-- Vue 3.5 -->
<script setup lang="ts">
import { useLang } from '../lib/lang'
const { t, locale } = useLang()
</script>

<template>
  <h1>{{ t('welcome', { app: 'Suprnova' }) }}</h1>
</template>
```

Formatação de número e data no cliente usa o `Intl` embutido do
navegador - nenhum dado ICU é enviado ao navegador.

### Chaves de mensagem tipadas

`suprnova generate-types` faz parse de `lang/<locale padrão>/*.ftl` e
emite uma union de todo id de mensagem, ao lado dos tipos de
page-props:

```ts
// frontend/src/types/lang-keys.ts
// Generated by `suprnova generate-types` - do not edit.
export type MessageKey =
  | "validation-min"
  | "welcome"
```

Os wrappers tipam `t(key: MessageKey, …)`, então essa é a mesma
promessa de [`inertia-props.ts`](frontend-typescript-types.md):
renomeie uma mensagem em Rust, regenere, e o compilador TypeScript
aponta para todo call site que ainda usa o id antigo. `suprnova serve`
observa `lang/` junto com `src/`, então o arquivo regenera enquanto
você edita catálogos.

Um projeto sem diretório `lang/` e sem ids de mensagem não recebe
**nenhum arquivo** - um app que não é localizado não vê nenhum
artefato novo aparecer.

## Formatação com reconhecimento de locale

Sete funções em `Lang`, todas apoiadas em ICU4X, todas lendo o
locale atual, todas com irmãos `try_*` que retornam
`Result<String, FrameworkError>` em vez de degradar:

```rust
use suprnova::chrono::NaiveDate;
use suprnova::{DateStyle, Lang, ListStyle, RelativeUnit, TimeStyle};

let dt = NaiveDate::from_ymd_opt(2026, 8, 1)
    .and_then(|d| d.and_hms_opt(14, 30, 0))
    .expect("valid datetime");

Lang::number(1_234_567.89);                          // en-US → 1,234,567.89
                                                     // de-DE → 1.234.567,89
Lang::currency(19.99, "USD");                        // en-US → $19.99
Lang::date(&dt, DateStyle::Long);                    // en-US → August 1, 2026
Lang::time(&dt, TimeStyle::Short);                   // en-US → 2:30 PM
Lang::datetime(&dt, DateStyle::Medium, TimeStyle::Short);
Lang::list(&["Ada", "Grace", "Alan"], ListStyle::And); // → Ada, Grace, and Alan
Lang::relative(-3, RelativeUnit::Day);               // → 3 days ago
```

Os enums de estilo: `DateStyle { Full, Long, Medium, Short }`,
`TimeStyle { Medium, Short }`, `ListStyle { And, Or, Unit }`,
`RelativeUnit { Second, Minute, Hour, Day, Week, Month, Year }`.
`Lang::relative` recebe uma quantidade com sinal - negativo é o
passado ("3 days ago"), positivo o futuro ("in 3 days").

> A saída exata vem dos dados CLDR embutidos no ICU4X e pode mudar
> através de um upgrade de ICU, particularmente para datas e moeda.
> Nos seus próprios testes, faça assert sobre forma e
> distinção-por-locale (`de != en`, contém `2026`) em vez de sobre
> bytes exatos.

### Formatando dentro de uma mensagem

Duas funções são chamáveis a partir do FTL:

```ftl
order-total = Your total is { NUMBER($amount, maximumFractionDigits: 2) }.
published = Published { DATETIME($when, dateStyle: "medium", timeStyle: "short") }
```

```rust
use suprnova::__;

let line = __!("published", when: "2026-08-01T14:30:00");
```

`NUMBER()` é embutida do Fluent, registrada explicitamente, e te dá
controle de dígito fracionário dentro da mensagem. `DATETIME()` é do
Suprnova: `$value` aceita uma string ISO-8601 ou milissegundos de
epoch, e `dateStyle` / `timeStyle` recebem os mesmos nomes dos enums
Rust, em minúsculas. Um valor que ela não consegue fazer parse passa
verbatim com um `warn!` - uma função Fluent não pode retornar um
erro, e uma página renderizada com uma data de aparência estranha é
melhor que um 500.

Quando você quer a formatação completa do ICU4X em vez do que uma
função Fluent expõe, formate em Rust e passe a string pronta:

```rust
use suprnova::{__, Lang};

let total = __!("order-total-text", amount: Lang::currency(19.99, "USD"));
```

## Testando suas traduções

Dois helpers fazem o trabalho: `use_lang_path` aponta o loader para
um diretório de fixture, e `scope_locale` fixa o locale atual pela
duração de uma future.

A forma hermética - construir um translator sobre um diretório de
fixture e vinculá-lo em um contêiner com escopo de teste - é o que os
próprios testes do framework usam, porque não toca nenhum estado
global ao processo e sobrevive à execução de teste paralela:

```rust
use std::sync::Arc;
use suprnova::testing::TestContainer;
use suprnova::{scope_locale, FluentTranslator, Lang, Locale, LocalizationConfig, Translator};

#[tokio::test]
async fn spanish_greeting_comes_from_the_catalog() {
    let _guard = TestContainer::fake();

    let config = LocalizationConfig::from_env().expect("locale config");
    let translator = FluentTranslator::from_dir("tests/fixtures/lang", &config)
        .expect("load catalogs");
    TestContainer::bind::<dyn Translator>(Arc::new(translator));

    scope_locale(Locale::parse("es").expect("locale"), async {
        assert_eq!(Lang::get("welcome"), "¡Bienvenido!");
        assert_eq!(Lang::locale().as_str(), "es");
    })
    .await;
}
```

`use_lang_path` é a ferramenta certa quando o teste boota a
aplicação real e você quer o app *inteiro* apontado para fixtures:

```rust
use suprnova::use_lang_path;

#[tokio::test]
async fn app_boots_against_fixture_catalogs() {
    use_lang_path("tests/fixtures/lang");
    // … boote o app; `lang_path("")` agora resolve para o diretório de fixture.
}
```

Ele escreve um override de path global ao processo, então trate-o
como uma configuração por binário em vez de algo sobre o qual dois
testes paralelos poderiam discordar.

A detecção em si - a cadeia sessão/cookie/`Accept-Language` - vale a
pena testar através do pipeline real em vez de chamar o middleware
diretamente, porque os casos interessantes são sobre parsing de
header e sobre qual fonte vence. Monte uma rota cujo handler retorna
`__!("welcome")`, registre `LocaleMiddleware` no
`MiddlewareRegistry`, e conduza-a com o harness de loopback de
[Testes HTTP](http-tests.md), enviando `Accept-Language: fr, es;q=0.8`
e fazendo assert sobre o corpo em espanhol. Os casos que vale a pena
fixar: um header negocia, um cookie vence um header, um locale
indisponível é pulado em vez de errar, e um header malformado ainda
retorna 200.

Veja [Testes](testing.md) para `TestContainer::scope` quando seu
teste roda em um runtime multi-thread - a guard `fake()` thread-local
acima não sobrevive a uma future migrando entre workers.

### Por que Suprnova diverge

**Arquivos FTL, não arrays PHP.** O Laravel tem dois formatos -
arrays aninhados em `lang/en/messages.php`, mais JSON flat em
`lang/en.json` para traduções chaveadas por string - e nenhum dos
dois é carregável por um navegador, nem expressa seleção de plural
no arquivo: isso vive na convenção de pipe-e-range do
`trans_choice` dentro da string. O Fluent nos dá um único formato
que o servidor e o cliente ambos fazem parse, o que é o que torna "o
frontend mostra a mesma string que o validator produziu" uma
propriedade do design em vez de uma convenção que você mantém. Custa
uma sintaxe nova para aprender (este capítulo é a maior parte dela) e
uma mudança de ferramental: o Poedit não consegue editar `.ftl`,
enquanto Crowdin, Weblate, Lokalise e Pontoon conseguem. Também custa
namespacing com ponto - `trans('messages.welcome')` não tem
equivalente, porque ids são um namespace flat por locale. Use prefixo
em vez disso.

**Sem `trans_choice`.** O Laravel seleciona uma forma de plural com
strings separadas por pipe e ranges explícitos:

```php
// Laravel
trans_choice('{1} plik|[2,4] pliki|[5,*] plików', $count);
```

Agora conte até 22 em polonês. O CLDR coloca 22 na categoria `few` -
`22 pliki` - mas `[5,*]` engole isso e produz `22 plików`. A mesma
quebra acontece em 32, 42, 102, e em russo, árabe, tcheco, lituano e
galês, cada um nos seus próprios pontos. Ranges de inteiro não
conseguem expressar regras de plural, porque regras de plural não
são sobre ranges; são sobre o último dígito, os últimos dois dígitos,
e em alguns idiomas se o valor é um inteiro. O Fluent seleciona
diretamente sobre a categoria CLDR, então `$count` é um argumento
comum e o *tradutor* - a pessoa que conhece o idioma - escreve as
quatro categorias do polonês:

```ftl
files =
    { $count ->
        [one] { $count } plik
        [few] { $count } pliki
        [many] { $count } plików
       *[other] { $count } pliku
    }
```

`one` é 1; `few` é 2–4, 22–24, 32–34, 102–104; `many` é 0, 5–21,
25–31; `other` pega as frações (`1,5 pliku`) e carrega o marcador
padrão, pela regra acima.

A forma sem range do Laravel (`plik|pliki|plików`) sai melhor - ela
consulta um índice por idioma e escolhe o *n*-ésimo segmento - mas
esse índice é uma tabela mantida à mão em vez de dados CLDR, ela
oferece três segmentos ao polonês onde o CLDR define quatro
categorias, os segmentos são posicionais sem nomes de categoria para
revisar, e ela só consegue selecionar sobre a contagem.

O que é o segundo benefício, que sai de graça: um seletor Fluent pode
selecionar sobre *qualquer* argumento, não só uma contagem. Gênero,
tier de plano e estado de conexão selecionam da mesma forma, e
nenhum deles precisou de um método novo de facade.

**Marcas de isolamento vêm desligadas por padrão.** O Fluent
normalmente envolve toda interpolação em U+2068 (FIRST STRONG
ISOLATE) e U+2069 (POP DIRECTIONAL ISOLATE), para que um valor
right-to-left embutido em uma frase left-to-right renderize na ordem
certa. Correto - e invisível, o que significa que todo
`assert_eq!("Hello Ada", …)` em um app só-inglês falha com dois
caracteres que ninguém consegue ver no diff. Nós os deixamos
desligados por padrão e tornamos ligá-los uma única chamada:

```rust
let config = LocalizationConfig::from_env()?.use_isolating(true);
```

**Ligue-os quando você lançar um locale RTL** - árabe, hebraico,
persa, urdu - ou qualquer locale onde valores fornecidos pelo
usuário misturam scripts dentro de uma frase. Depois atualize suas
assertions para comparar contra strings que carregam as marcas, ou
remova-as no helper de assertion. O padrão otimiza para o caso comum;
o caso correto está a uma linha de distância e este parágrafo é o
lembrete para tomá-la.

## Próximos passos

- [Validação](validation.md) - regras, a macro `validate!`, e de onde
  vem `ValidationMessage`
- [Tipos TypeScript](frontend-typescript-types.md) - `generate-types`,
  `inertia-props.ts`, e `lang-keys.ts`
- [Middleware](middleware.md) - ordenando `LocaleMiddleware` contra o
  resto da chain global
- [Sessão](session.md) - o store que o primeiro passo de detecção lê
- [Variáveis de ambiente](env-vars.md) - `APP_LOCALE`,
  `APP_FALLBACK_LOCALE`, `APP_LOCALE_PARENTS`, `APP_BASE_PATH`
- [Testes](testing.md) - `TestContainer`, `#[suprnova_test]`, e
  overrides de DI herméticos
