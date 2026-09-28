# Configuração

Suprnova lê a configuração a partir de variáveis de ambiente (carregadas do
`.env` em desenvolvimento, do ambiente do processo em produção) e as expõe
ao seu código em duas formas:

1. **Acesso direto a env** - `env::env`, `env_required`, `env_optional`
   para consultas pontuais
2. **Estruturas de configuração tipadas** - `Config::register` / `Config::get` para
   qualquer coisa que você leia mais de uma vez, com tipagem forte

O framework lê algumas variáveis de ambiente por conta própria (`APP_KEY`,
`APP_ENV`, `DATABASE_URL`, etc.); o resto é seu.

## O arquivo `.env`

`suprnova new` escreve um `.env` inicial com os valores que sua app precisa
para inicializar:

```env
APP_NAME="my-app"
APP_ENV=local                # local, development, staging, production, testing, …
APP_DEBUG=true               # detailed error pages + verbose logs
APP_URL=http://localhost:8765

# 32-byte AES-256 key (URL-safe base64, no padding). Encrypts session
# cookies, pagination cursors, and anything via `suprnova::Crypt`.
# Generated at scaffold time. Rotate with `suprnova key:generate`.
APP_KEY=<32-byte base64>

SERVER_HOST=127.0.0.1
SERVER_PORT=8765
VITE_PORT=5765

# Database - SQLite by default; swap to postgres://user:pass@host/db
DATABASE_URL=sqlite://./database.db
DB_MAX_CONNECTIONS=10
DB_MIN_CONNECTIONS=1
DB_CONNECT_TIMEOUT=30
DB_LOGGING=false

# Session
SESSION_LIFETIME=120         # minutes
SESSION_COOKIE=suprnova_session
SESSION_SECURE=false         # set true in production (HTTPS only)
SESSION_PATH=/
SESSION_SAME_SITE=Lax

# Mail - defaults to `log` driver (writes outgoing mail to the
# tracing log, good for dev). Set MAIL_DRIVER to one of
# smtp / ses / mailgun / postmark / sendgrid / resend / log / memory
# for production.
MAIL_DRIVER=log
# SMTP credentials (only read when MAIL_DRIVER=smtp):
MAIL_SMTP_HOST=127.0.0.1
MAIL_SMTP_PORT=587
MAIL_SMTP_USER=
MAIL_SMTP_PASS=
# starttls | tls | none. Left blank it derives from the credentials
# above - starttls with them, none without. Production refuses to boot
# unencrypted; see the Mail chapter.
MAIL_SMTP_ENCRYPTION=
```

Um arquivo `.env.example` complementar envia as mesmas chaves com valores
de espaço reservado - faça commit dele; não faça commit de `.env`. O
`.gitignore` padrão já exclui `.env`.

## Como o carregamento de `.env` funciona

Na inicialização, o framework:

1. Detecta o ambiente a partir de `APP_ENV` (insensível a maiúsculas/minúsculas,
   `prod`/`dev`/`stage`/`stg`/`test` também são reconhecidos).
2. Carrega `.env` da raiz do projeto.
3. Se um arquivo por ambiente existir (`.env.staging`, `.env.production`),
   carrega-o por cima - seus valores substituem `.env`.
4. As variáveis de ambiente do processo real substituem ambas (isso é o que
   orquestração de contêineres depende).

A ordem em uma linha: **env do processo > `.env.<environment>` > `.env`**.

```rust
use suprnova::Config;

let env = Config::environment();           // Environment::Local
let is_prod = Config::is_production();     // false
```

Em uma execução de CI com `APP_ENV=testing`, o framework carrega `.env.testing`
por cima de `.env` para que você possa sobrescrever URLs de banco de dados e
desabilitar drivers de mail sem tocar no `.env` de desenvolvimento.

## Acesso direto a env

Para leituras pontuais de strings, números, bools - qualquer coisa implementando
`std::str::FromStr` - use a família `env::*`:

```rust
use suprnova::config::{env, env_required, env_optional};

let port: u16 = env("SERVER_PORT", 8765);                    // com padrão
let url: String = env_required("APP_URL");                   // entra em pânico se faltar - só na inicialização
let smtp_host: Option<String> = env_optional("MAIL_HOST");   // None se faltar
```

- `env(key, default)` - leitura com coerção de tipo e fallback
- `env_required(key)` - entra em pânico se a chave estiver faltando ou falhar ao
  ser analisada. Use apenas no tempo de inicialização (em `bootstrap()` ou `config::register()`)
  onde um valor obrigatório faltando deve encerrar o processo imediatamente
- `env_optional(key)` - retorna `Option<T>`; `None` para valores faltando ou
  que não podem ser analisados

Cada chave única também é registrada uma vez na primeira leitura, então você
pode auditar exatamente quais variáveis de ambiente sua app toca.

## Estruturas de configuração tipadas

Para qualquer coisa que sua app leia mais de uma vez, defina uma estrutura
tipada e registre-a. O padrão é:

```rust
// src/config/database.rs
use suprnova::Config;
use suprnova::config::{env, env_required, env_optional};

#[derive(Clone, Debug)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
    pub min_connections: u32,
    pub connect_timeout_secs: u32,
    pub logging: bool,
}

pub fn register() {
    Config::register(DatabaseConfig {
        url: env_required("DATABASE_URL"),
        max_connections: env("DB_MAX_CONNECTIONS", 10),
        min_connections: env("DB_MIN_CONNECTIONS", 1),
        connect_timeout_secs: env("DB_CONNECT_TIMEOUT", 30),
        logging: env("DB_LOGGING", false),
    });
}
```

Depois leia em qualquer lugar com uma linha:

```rust
let db = Config::get::<DatabaseConfig>().expect("DB config registered at boot");
println!("Pool size: {}", db.max_connections);
```

O registro é indexado por `TypeId`, então cada estrutura é armazenada uma
vez. Chamar `Config::register` novamente com o mesmo tipo substitui a
entrada anterior - conveniente para testes.

### Conectando o registro à sua app

O `cmd/main.rs` do scaffold inclui um passo `.config(…)` no pipeline de
inicialização fluente:

```rust
use suprnova::Application;

#[suprnova::main]
async fn main() {
    Application::new()
        .config(my_app::config::register)   // ← isto chama o seu registro
        .bootstrap(my_app::bootstrap::register)
        .routes(my_app::routes::register)
        .migrations::<my_app::migrations::Migrator>()
        .run()
        .await
}
```

`my_app::config::register` normalmente delega para cada módulo de seção:

```rust
// src/config/mod.rs
pub mod database;
pub mod mail;

pub fn register() {
    database::register();
    mail::register();
}
```

### Desserializando estruturas inteiras a partir de env

Para configurações maiores, você pode desserializar diretamente de variáveis
de ambiente via `serde`. Suprnova expõe dois auxiliares:

```rust
use suprnova::Config;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

// Lê SERVER_HOST / SERVER_PORT do ambiente
let cfg = Config::resolve_prefixed::<ServerConfig>("SERVER_")?;
```

- `Config::resolve::<T>()` - desserializar de todas as variáveis de ambiente do processo
- `Config::resolve_prefixed::<T>("PREFIX_")` - desserializar apenas
  variáveis com o prefixo dado (o prefixo é removido antes da
  desserialização)

Ambas retornam `Result<T, FrameworkError>` então um campo obrigatório
faltando aparece como um `FrameworkError::Internal` carregando o diagnóstico
do envy em vez de um pânico.

## Configuração específica de ambiente

O enum `Environment` cobre o conjunto padrão:

| Variante | Valores reconhecidos de `APP_ENV` |
|---|---|
| `Local` | `local` |
| `Development` | `development`, `dev` |
| `Staging` | `staging`, `stage`, `stg` |
| `Production` | `production`, `prod` |
| `Testing` | `testing`, `test` |
| `Custom(String)` | qualquer outra coisa (preserva suas maiúsculas/minúsculas, usada para busca de `.env.<custom>`) |

Ramificações comuns:

```rust
use suprnova::{Config, Environment};

if Config::is_production() {
    // cookies estritos, driver de mail real, etc.
}

if Config::is_debug() {
    // páginas de erro detalhadas, logs de consulta
}

match Config::environment() {
    Environment::Production => { /* … */ },
    Environment::Staging    => { /* … */ },
    _ => { /* dev/test path */ },
}
```

`is_debug()` retorna `true` quando `APP_DEBUG=true` é definido explicitamente,
ou - quando `APP_DEBUG` não está definido - quando o ambiente detectado é
`Local`, `Development`, ou `Testing`. Production, staging, e qualquer ambiente
customizado não reconhecido padrão para `false`. Mantenha desativado em
produção; ele controla o detalhe da página de erro e alguns padrões internos.

### `APP_KEY` é obrigatório em não-desenvolvimento

Em produção (qualquer `APP_ENV` diferente de `local`/`development`/
`testing`), Suprnova requer que `APP_KEY` seja definido como uma string
base64 URL-segura válida de 32 bytes. Inicializar sem isso falha de forma
fechada com uma mensagem de erro descritiva - não há fallback silencioso.

Se você ainda não tiver um `APP_KEY`:

```bash
suprnova key:generate          # imprime a chave com uma dica lembrando de adicioná-la ao .env
suprnova key:generate --show   # imprime só a chave, adequado para `APP_KEY=$(suprnova key:generate --show)`
```

Nenhuma forma edita `.env` para você - copie a chave impressa para seu
`.env` (ou seu gerenciador de secrets) você mesmo.

Para rotação de chave (onde dados criptografados antigos ainda devem
descriptografar durante a janela de migração), veja [Criptografia](encryption.md#key-rotation).

## Configuração em testes

Em testes, registre a configuração na configuração de teste em vez de
depender de `.env`:

```rust
use suprnova::suprnova_test;

#[suprnova_test]
async fn test_with_custom_db() {
    suprnova::Config::register(DatabaseConfig {
        url: "sqlite::memory:".to_string(),
        max_connections: 1,
        min_connections: 1,
        connect_timeout_secs: 5,
        logging: false,
    });

    // … seu teste
}
```

O atributo `#[suprnova_test]` também configura estado de contêiner isolado
para que testes concorrentes não vejam bindings uns dos outros - veja
[Testes](testing.md).

## Variáveis de ambiente comuns que o Suprnova lê

Uma lista não exaustiva - essas são variáveis que o framework em si procura.
Sua app lê mais por cima.

| Var | Padrão | O que faz |
|---|---|---|
| `APP_NAME` | `"app"` | Registrada na inicialização, usada em algumas mensagens de erro padrão |
| `APP_ENV` | `local` | Dirige `Environment::detect` e busca de `.env.<suffix>` |
| `APP_DEBUG` | consciente de env (`false` em produção) | Páginas de erro detalhadas + logging extra |
| `APP_URL` | `http://localhost:8765` | URL base para geração de URL absoluta, URLs assinadas |
| `APP_KEY` | nenhuma (obrigatório em prod) | Chave AES-256 para `Crypt`, sessions, cursores |
| `APP_KEY_PREVIOUS` | nenhuma | Chaves anteriores separadas por vírgula para rotação (máx 8) |
| `SERVER_HOST` | `127.0.0.1` | Endereço de bind |
| `SERVER_PORT` | `8765` | Porta de bind |
| `DATABASE_URL` | nenhuma | Obrigatório se sua app usar o banco de dados |
| `DB_MAX_CONNECTIONS` | `10` | Máximo de pool sqlx |
| `DB_MIN_CONNECTIONS` | `1` | Mínimo de pool sqlx |
| `DB_CONNECT_TIMEOUT` | `30` (segundos) | Tempo limite de conexão do pool sqlx |
| `SESSION_LIFETIME` | `120` (minutos) | Expiração de sessão |
| `SESSION_TOUCH_INTERVAL` | `300` (segundos) | Cadência mínima de escrita de expiração deslizante |
| `SESSION_GC_INTERVAL` | `3600` (segundos) | Cadência de limpeza de sessão expirada supervisionada |
| `SESSION_COOKIE` | `suprnova_session` | Nome do cookie |
| `SESSION_SECURE` | `true` | Defina a flag de cookie `Secure`. Sobrescreva para `false` para desenvolvimento local-HTTP. |
| `SESSION_SAME_SITE` | `Lax` | `Strict`, `Lax`, ou `None` |
| `MAIL_DRIVER` | `log` | Uma de `smtp`, `ses`, `mailgun`, `postmark`, `sendgrid`, `resend`, `log`, `memory` |
| `CACHE_DRIVER` | `memory` | Uma de `memory`, `redis`, `database` |
| `QUEUE_DRIVER` | `memory` | Uma de `memory`, `redis`, `database` (valores desconhecidos avisar e fallback para `memory`) |
| `RATE_LIMIT_DRIVER` | `memory` | Uma de `memory`, `redis` |
| `LOG_FORMAT` | consciente de env (`pretty` em dev/local, `json` em produção) | `pretty` ou `json` |
| `LOG_LEVEL` | `info` | Uma de `error`, `warn`, `info`, `debug`, `trace` |

A lista completa auditada vive em [Variáveis de ambiente](env-vars.md).

## Próximos passos

- [Inicialização da aplicação](bootstrap.md) - onde o registro de configuração
  tipada é chamado
- [Contêiner de serviços](container.md) - como a configuração registrada é lida
  junto com serviços vinculados
- [Variáveis de ambiente](env-vars.md) - a lista de referência completa
- [Implantação](deployment.md) - configuração de env de produção
