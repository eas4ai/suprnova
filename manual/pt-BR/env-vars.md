# Variáveis de ambiente

Esta é a lista auditada de toda variável de ambiente que o framework
Suprnova lê em runtime, agrupada pelo subsistema que a consulta. Toda
entrada foi validada contra o código-fonte do framework - padrões,
tipos e comportamento refletem o que o código realmente faz, e não
necessariamente o que o `.env` inicial traz.

A lista também cobre as variáveis que o binário da CLI `suprnova` lê
(servidor de dev, worker SSR), já que elas aparecem no `.env` inicial
e o leitor vai procurá-las aqui.

Veja [Configuração](configuration.md) para as regras de carregamento
(`.env` → `.env.<environment>` → env do processo), os helpers `env*`
(`env`, `env_required`, `env_optional`), e o padrão de registro
tipado `Config::*`.

## Convenções

- **Padrão** - o valor que o framework usa quando a variável não está
  definida. `none` significa que não há padrão; o framework ou dá
  erro no boot, ou recai para um padrão de feature (ex.: driver
  `Memory`), ou trata o valor como `None`.
- **Tipo** - o tipo Rust para o qual a variável é parseada. Valores
  `bool` aceitam `true`/`false`/`1`/`0`/`yes`/`no`/`on`/`off`
  (insensível a maiúsculas/minúsculas). Valores fora do intervalo ou
  não parseáveis para knobs tipados do framework são limitados a um
  intervalo (workflow), registrados com `warn!` e então definidos com
  o padrão (`env()` / `env_optional()` leniente), ou falham o boot
  (`try_from_env` estrito).
- **Obrigatório** - `boot` significa que o framework se recusa a
  iniciar sem ela nos ambientes listados. `driver` significa que só é
  obrigatória quando o driver pai é selecionado (ex.:
  `MAIL_SES_REGION` é irrelevante a menos que `MAIL_DRIVER=ses`). Todo
  o resto é opcional.

Onde um `.env` inicial traz uma chave que o framework nunca lê
(`MAIL_FROM_ADDRESS`, `FILESYSTEM_DISK`), isso é apontado no final
deste capítulo.

## Aplicação

A família `APP_*` é a identidade e a raiz criptográfica do framework.
Essas são as variáveis que toda app Suprnova define; o resto do
arquivo se torna relevante conforme você opta por subsistemas.

| Var | Padrão | Tipo | Propósito |
|---|---|---|---|
| `APP_NAME` | `"Suprnova Application"` | `String` | Nome da aplicação. Usado como o issuer do TOTP (2FA), o realm do `WWW-Authenticate` do HTTP Basic, a marca do assunto de email, e campos do log estruturado. |
| `APP_ENV` | `local` | `String` | Dirige `Environment::detect()` e a busca de `.env.<suffix>`. Aliases reconhecidos (insensível a maiúsculas/minúsculas): `local`, `development`/`dev`, `staging`/`stage`/`stg`, `production`/`prod`, `testing`/`test`. Qualquer outro valor é preservado como `Environment::Custom(...)` com a caixa original. |
| `APP_DEBUG` | consciente do ambiente (veja Obrigatório) | `bool` | Páginas de erro verbosas + logs extras. O padrão é `true` em `local`/`development`/`testing` e `false` em todo o resto (incluindo `staging`, `production`, e qualquer ambiente customizado não reconhecido). Um valor explícito sempre vence; um valor não parseável recai para o padrão consciente do ambiente com um `warn!`. A variante estrita `try_from_env` aborta o boot numa falha de parse. |
| `APP_URL` | `"http://localhost:8765"` (AppConfig) / `"http://localhost"` (fallback de URL) | `String` | URL base para geração de URL absoluta, URLs assinadas, e redirecionamentos do Inertia. Barras finais são removidas na leitura. |
| `APP_KEY` | nenhuma - obrigatória fora de dev | `String` (base64-url sem padding, 32 bytes) | Chave AES-256-GCM para `Crypt`, sessões criptografadas, cursores de paginação, URLs assinadas, e qualquer outro caminho de criptografia em repouso. O boot **falha de forma fechada** quando estiver faltando ou malformada fora de `local`/`development`/`testing`. Gere com `suprnova key:generate`. |
| `APP_KEY_PREVIOUS` | nenhuma | `String` (chaves base64 separadas por vírgula, máx. 8) | Chaves anteriores separadas por vírgula, usadas durante a rotação. `Crypt::decrypt` tenta primeiro a `APP_KEY` atual, depois cada entrada em ordem. Limite rígido de 8 entradas - `crypto::MAX_PREVIOUS_KEYS`. Uma entrada parcialmente rotacionada que falha ao decodificar aborta o boot. Veja [Criptografia](encryption.md#key-rotation). |
| `APP_PREVIOUS_KEYS` | nenhuma | `String` (alias de `APP_KEY_PREVIOUS`) | Alias de compatibilidade com Laravel, aceito para que um `.env` do Laravel solto num deploy Suprnova ainda consiga descriptografar dados legados sem quebrar. Quando as duas estão definidas com valores diferentes, `APP_KEY_PREVIOUS` vence, com um `warn!` para expor a duplicidade; valores idênticos são aceitos silenciosamente. |
| `APP_BASE_PATH` | diretório de trabalho atual | `Path` | Diretório raiz que o resolvedor de caminhos usa para `config/`, `database/`, `public/`, `storage/`, `resources/`, `lang/`. Útil ao executar o binário a partir de um CWD diferente da raiz do projeto (ex.: unit do systemd cujo `WorkingDirectory=` não aponta para o projeto). Recai para o CWD e, se o CWD não estiver disponível, para `.`. |
| `APP_TRUSTED_PROXIES` | nenhum - allowlist vazia | `String` (IPs separados por vírgula) | Endereços de peer TCP cujos headers `X-Forwarded-*` / `X-Real-IP` o `Request::ip()` e os acessadores de host / esquema / porta podem confiar. **Vazio por padrão, então headers de proxy são ignorados e o peer TCP sempre vence** - veja a nota abaixo antes de fazer deploy detrás de um proxy. Uma entrada não parseável falha o boot (`try_from_env`). |
| `AUTH_GUARD` | `"web"` | `String` | Nome do guard padrão lido por `Auth::*`. Espelha o Laravel - somente o padrão é selecionável via env; guards nomeados vivem no código via `AuthConfig::guard(name, …)`. |

Duas outras variáveis `APP_*` - `APP_LOCALE` e `APP_FALLBACK_LOCALE` -
são lidas pelo subsistema de localização, e não por `AppConfig`, então
elas estão listadas em **Localização** abaixo.

### Detrás de um proxy reverso, defina `APP_TRUSTED_PROXIES`

Ignorar headers de proxy é o padrão seguro - `X-Forwarded-For` é
fornecido pelo chamador, e confiar nele incondicionalmente deixa
qualquer um reivindicar qualquer endereço. Mas no momento em que um
proxy terminador está na sua frente (nginx, Traefik, um ALB,
Cloudflare), o peer TCP é *o proxy*, em toda solicitação, e deixar
isso sem definir não apenas perde o endereço do cliente:

- **Limites de taxa por IP colapsam em um único bucket.** A chave
  padrão do `ThrottleRequestsMiddleware` é `request.ip()`, então
  `ThrottleRequestsMiddleware::with(20, 1, "login")` deixa de
  significar "20 tentativas de login por cliente por minuto" e passa
  a significar 20 *no total, entre todo mundo*. Isso é ao mesmo tempo
  mais fraco (sem orçamento por atacante) e ativamente perigoso:
  qualquer chamador isolado pode gastar a cota e trancar todo usuário
  legítimo fora do formulário de login. Veja
  [Limitação de taxa](rate-limiting.md).
- `Request::host()`, `scheme()` e `port()` recaem para a conexão em
  vez de `X-Forwarded-Host` / `-Proto` / `-Port`, então URLs absolutas
  geradas podem nomear o endereço e o esquema internos em vez dos
  públicos.

Liste os endereços a partir dos quais os hops do proxy alcançam você -
não os do cliente:

```bash
APP_TRUSTED_PROXIES=10.0.0.5,10.0.0.6
```

Nada detecta isso por você: uma app detrás de um proxy com a variável
não definida parece saudável, serve corretamente, e silenciosamente
limita a taxa de todo mundo como se fosse um único usuário.

### Matriz de obrigatoriedade do app-key

| Ambiente | `APP_KEY` obrigatória no boot |
|---|---|
| `local` | não (gera uma chave efêmera se estiver faltando) |
| `development` | não |
| `testing` | não |
| `staging` | sim - o boot sai com código não-zero e uma mensagem de remediação |
| `production` | sim |
| `Custom(...)` | sim - qualquer coisa que não esteja na safe-list é tratada como produção para essa verificação |

## Servidor

O listener HTTP e os limites de corpo de solicitação.

| Var | Padrão | Tipo | Propósito |
|---|---|---|---|
| `SERVER_HOST` | `"127.0.0.1"` | `String` | Endereço de bind. Defina como `0.0.0.0` para expor fora da interface de loopback (ex.: em contêineres). |
| `SERVER_PORT` | `8765` | `u16` | Porta de bind. O parse leniente avisa e usa o padrão; o `try_from_env` estrito aborta o boot num erro de digitação. |
| `SERVER_MAX_BODY_SIZE` | `8388608` (8 MiB) | `usize` (bytes) | Tamanho máximo de corpo de solicitação, global ao processo. Sobrescritas por `FormRequest::max_body_bytes` ainda se aplicam a endpoints individuais. O valor configurado é conectado ao limite global durante `Server::from_config`. |
| `SERVER_MAX_CONNECTIONS` | não definido (ilimitado) | `usize` | Limite de conexões TCP ativas concorrentes. Não definido significa sem limite. Um valor zero ou não parseável recai para um valor finito de `10000` com um aviso, em vez de silenciosamente reverter para ilimitado - um limite malformado ainda é um pedido de limite. |
| `SERVER_HEADER_READ_TIMEOUT` | `30` | `u64` (segundos) | Prazo para ler o head completo de uma solicitação. A mitigação de slowloris. Zero é tratado como inválido, não como "desabilitar", e recai para o padrão. Não se aplica a conexões WebSocket/SSE já estabelecidas. |
| `SERVER_HEALTH_READINESS_TOKEN` | não definido (a prontidão é pública) | `String` | Segredo compartilhado necessário para alcançar `/_suprnova/health/ready` e `/_suprnova/health?db=true`, enviado como `X-Suprnova-Health-Token`. Sem ele, esses caminhos respondem 404, de forma indistinguível de qualquer caminho não roteado; a vivacidade permanece pública. Veja [Implantação](deployment.md#health-check). |

## Banco de dados

URL de conexão e o ajuste do pool do sqlx. `DATABASE_URL` é
obrigatória para todo subcomando que toca o banco de dados
(`migrate*`, `db:sync`, `db:seed`, `queue:work` com
`QUEUE_DRIVER=database`, `workflow:work`, o store de sessão em BD) e
para `serve` quando a app tem migrações registradas.

| Var | Padrão | Tipo | Propósito |
|---|---|---|---|
| `DATABASE_URL` | nenhuma - obrigatória quando há migrações | `String` | URL de conexão. O esquema seleciona o driver: `sqlite://path`, `postgres://...` / `postgresql://...`, `mysql://...`, `mariadb://...`. O framework cria automaticamente o diretório pai para caminhos SQLite. `serve` pula inteiramente a conexão de banco de dados quando o `Migrator` configurado não tem migrações. |
| `DB_MAX_CONNECTIONS` | `10` | `u32` | Teto do pool do sqlx. |
| `DB_MIN_CONNECTIONS` | `1` | `u32` | Piso do pool do sqlx (mantido aquecido). |
| `DB_CONNECT_TIMEOUT` | `30` (segundos) | `u32` | Quanto tempo o sqlx espera por uma conexão inicial antes de dar erro. |
| `DB_LOGGING` | `false` | `bool` | Quando `true`, o sqlx registra todo statement (use com moderação em produção - fica verboso). |
| `SUPRNOVA_AUTO_MIGRATE_BEST_EFFORT` | `false` | `bool` | Quando `true`, uma auto-migração que falha durante o boot do `serve` é registrada no log mas não aborta. O padrão é fail-closed: o boot sai com código não-zero em vez de iniciar contra um schema parcialmente migrado. Passe `--no-migrate` para pular a auto-migração por completo. |

## Sessão

Atributos de cookie e tempo de vida para o subsistema de sessão. Note
que `SESSION_SECURE` tem padrão **`true`** - seguro para produção por
padrão; desative apenas para desenvolvimento HTTP local.

| Var | Padrão | Tipo | Propósito |
|---|---|---|---|
| `SESSION_LIFETIME` | `120` (minutos) | `u64` | Tempo de vida da sessão em minutos. Parseado via `env_optional`; recai silenciosamente para o padrão se não for parseável. |
| `SESSION_TOUCH_INTERVAL` | `300` (segundos) | `u64` | Cadência mínima de persistência da expiração deslizante. A aplicação em runtime limita isso à metade do tempo de vida da sessão. |
| `SESSION_GC_INTERVAL` | `3600` (segundos) | `u64` | Cadência do coletor supervisionado de sessões expiradas instalado por `SessionMiddleware::install`. |
| `SESSION_COOKIE` | `"suprnova_session"` | `String` | Nome do cookie de sessão. |
| `SESSION_PATH` | `"/"` | `String` | Atributo `Path=` do cookie. |
| `SESSION_DOMAIN` | não definido | `String` | Atributo `Domain=` do cookie. Deixe não definido para cookies host-only (o padrão mais seguro para a maioria das apps). |
| `SESSION_SECURE` | `true` | `bool` | Atributo `Secure` do cookie. O padrão é `true`; defina como `false` apenas em desenvolvimento HTTP local. `cookie_http_only` é sempre `true` e não é configurável via env. |
| `SESSION_SAME_SITE` | `"Lax"` | `String` | Atributo `SameSite`. Aceita `Strict`, `Lax`, `None` (insensível a maiúsculas/minúsculas). |
| `SESSION_COOKIE_PREFIX` | não definido | `String` (`__Host-` / `__Secure-`) | Prefixo aplicado aos nomes de rede de sessão e remember-me. `Config::init` valida o valor e as restrições de `SESSION_DOMAIN` / `SESSION_PATH` no boot; combinações inválidas falham antes de servir. |
| `SESSION_PARTITIONED` | `false` | `bool` | Emite o atributo de cookie `Partitioned` / CHIPS para cookies isolados de terceiros. |
| `SESSION_EXPIRE_ON_CLOSE` | `false` | `bool` | Quando `true`, remove o `Max-Age` para que o navegador exclua o cookie ao fechar (semântica de cookie de sessão). |
| `SESSION_CONNECTION` | não definido | `String` | Conexão de BD nomeada para o store de sessão. Não definido significa a conexão padrão. |
| `REMEMBER_LIFETIME` | `43200` (30 dias, em minutos) | `u64` | Tempo de vida do cookie/token de "lembrar de mim" em minutos. |

## Localização

As três variáveis `APP_*` que o subsistema de localização lê. Todo o
resto sobre ele - a chain de detecção, a chave de sessão e o nome do
cookie que consulta, as marcas de isolamento Unicode - é configuração
em nível de código em `LocalizationConfig`, não env. Veja
[Localização](localization.md).

| Var | Padrão | Tipo | Propósito |
|---|---|---|---|
| `APP_LOCALE` | `"en"` | `String` (BCP-47) | Locale usado quando a chain de detecção (sessão → cookie → `Accept-Language`) não encontra nada. Também é o locale do qual `suprnova generate-types` extrai as chaves de mensagem para `lang-keys.ts`. Um valor que não é um identificador BCP-47 válido falha o boot em vez de recair silenciosamente para o padrão. |
| `APP_FALLBACK_LOCALE` | `"en"` | `String` (BCP-47) | Locale consultado quando uma chave está faltando no catálogo do locale atual. Uma chave faltando nos dois é renderizada como a própria chave, mais um `warn!` único; `Lang::try_get` retorna `Err` em vez disso. Mesmo parse estrito que `APP_LOCALE`. |
| `APP_LOCALE_PARENTS` | nenhum - mapa vazio | `String` (pares `child=parent` separados por vírgula, BCP-47 em cada lado) | Pais de fallback por locale, consultados antes de `APP_FALLBACK_LOCALE`, ex.: `APP_LOCALE_PARENTS=pt-PT=pt-BR,en-AU=en-GB`. A chain de fallback do `Lang` percorre esses pais transitivamente, e o `FluentTranslator` achata a chain de pais configurada de cada locale no catálogo que serve. Um par malformado, um locale inválido, um filho nomeado mais de uma vez, ou um ciclo (incluindo um locale se nomeando como seu próprio pai) falha o boot em vez de degradar em tempo de solicitação. Veja [Fallback chains](localization.md#fallback-chains). |

Os próprios catálogos são arquivos, não env: `lang/<locale>/*.ftl` sob
`APP_BASE_PATH`. Um diretório `lang/` faltando não é um erro - a app
inicializa com o catálogo de validação em inglês embutido no
framework.

## Cache

| Var | Padrão | Tipo | Propósito |
|---|---|---|---|
| `CACHE_DRIVER` | `memory` | `String` (`memory`/`in-memory`/`inmemory`, `redis`) | Seleciona o alvo de inicialização. `memory` mantém tudo em processo; `redis` exige `REDIS_URL` e falha o boot se inalcançável. Valores desconhecidos falham o boot com um erro claro. |
| `REDIS_URL` | `"redis://127.0.0.1:6379"` | `String` | URL de conexão do Redis (consultada somente quando `CACHE_DRIVER=redis`). |
| `REDIS_PREFIX` | `"suprnova_cache:"` | `String` | Prefixo de chave para entradas de cache (evita colisão em Redis compartilhado). |
| `CACHE_DEFAULT_TTL` | `3600` (segundos) | `u64` | TTL padrão em segundos. `0` significa "sem expiração". Aplicado a `Cache::put(None)` / `Cache::tags_put(None)`; `Cache::forever` e `Cache::remember_forever` sempre ignoram isso. |

## Fila

| Var | Padrão | Tipo | Propósito |
|---|---|---|---|
| `QUEUE_DRIVER` | `memory` | `String` (`memory`, `redis`, `database`, `failover`) | Backend de fila ativo. Valores desconhecidos registram um `warn!` e recaem para `memory`. `failover` embrulha uma lista ordenada dos outros - veja `QUEUE_FAILOVER_CONNECTIONS`. |
| `QUEUE_FAILOVER_CONNECTIONS` | - | `String` (separado por vírgula, ex.: `redis,database`) | Lista de conexões ordenada por prioridade para `QUEUE_DRIVER=failover`. Obrigatória quando esse driver é selecionado; um valor ausente ou em branco é um erro de boot, assim como uma entrada que nomeie `failover` (sem aninhamento) ou um driver que não existe. Cada entrada lê as variáveis do próprio driver. Só os pushes caem pela lista; toda leitura e todo reconhecimento vão para a primeira conexão, então cada fallback precisa do próprio worker. |
| `QUEUE_REDIS_URL` | `"redis://127.0.0.1:6379"` | `String` | URL do Redis (obrigatória pelo driver quando `QUEUE_DRIVER=redis`). |
| `QUEUE_REDIS_STREAM` | `"suprnova-queue"` | `String` | Chave do Redis Stream usada para fan-out. |
| `QUEUE_REDIS_GROUP` | `"default"` | `String` | Nome do consumer group. |
| `QUEUE_REDIS_CONSUMER` | `"consumer-1"` | `String` | Nome do consumer dentro do group. Defina por worker para workers paralelos. |
| `QUEUE_VISIBILITY_TIMEOUT_SECS` | `60` | `u64` | Por quanto tempo um job reclamado (claimed) fica invisível antes de outro consumer poder reclamá-lo de volta. Ajuste ao seu job mais lento. |
| `QUEUE_DB_TABLE` | `"jobs"` | `String` | Nome da tabela para o driver de banco de dados. Validado como um identificador SQL - um valor malformado falha no boot, não no momento de composição do SQL. Obrigatório pelo driver quando `QUEUE_DRIVER=database`; o driver também exige que `DB::init()` já tenha executado antes. |
| `QUEUE_FAILED_DB_TABLE` | `"failed_jobs"` | `String` | Tabela na qual o store de dead-letter escreve. Vinculada automaticamente quando `QUEUE_DRIVER=database` - `queue:retry` a lê e `Queue::retry_failed` precisa dela, então a tabela é parte do contrato daquele driver. Não usada por `memory` (efêmero por construção) nem por `redis` (sem tabela para escrever). Diferente de `QUEUE_DB_TABLE`, um identificador malformado aqui **não** falha o boot: ele registra em `error!` e não deixa nenhum store vinculado, então jobs mandados para dead-letter são registrados por completo em vez de persistidos. Recuperável à mão, mas não por `queue:retry`. |

## Agendamento

| Var | Padrão | Tipo | Propósito |
|---|---|---|---|
| `SCHEDULE_ALLOW_MEMORY_LOCK_IN_PRODUCTION` | não definido | `bool`-ish | Reconhece que uma tarefa marcada `on_one_server()` está eleitando um líder através de um cache **por processo**. Essa eleição só é tão compartilhada quanto o cache por trás dela, então em produção `CACHE_DRIVER=memory` mais uma tarefa de servidor único é uma falha de boot rígida, nomeando as tarefas culpadas, em vez de um downgrade silencioso para "toda réplica executa". Defina isso apenas onde o deploy genuinamente executa um único agendador; caso contrário, defina `CACHE_DRIVER=redis`. Veja [Agendamento](scheduling.md). |

## Fluxo de trabalho

O worker `#[workflow]`, com estado, de longa duração. Todos os valores
são limitados (clamped) a mínimos seguros em vez de honrados
cegamente - um `WORKFLOW_CONCURRENCY=0` deixaria o semáforo do worker
parado para sempre, então o framework avisa e limita em vez de
aceitar uma config obviamente quebrada.

| Var | Padrão | Tipo | Propósito |
|---|---|---|---|
| `WORKFLOW_CONCURRENCY` | `4` | `usize` | Máximo de execuções de fluxo de trabalho concorrentes por processo worker. Limitado a `>= 1`. |
| `WORKFLOW_POLL_INTERVAL_MS` | `1000` (ms) | `u64` | Com que frequência o worker faz poll por fluxos de trabalho recém-vencidos. |
| `WORKFLOW_LOCK_TIMEOUT_SECS` | `30` (segundos) | `u64` | Timeout de reclamação para uma linha de fluxo de trabalho reclamada cujo worker morreu. |
| `WORKFLOW_MAX_ATTEMPTS` | `3` | `i32` | Máximo de tentativas por execução de fluxo de trabalho antes de ser marcada como falha. Limitado a `>= 1`. |
| `WORKFLOW_RETRY_BACKOFF_SECS` | `5` | `i64` | Backoff linear por tentativa. Limitado a `>= 0` - um backoff negativo agendaria retries no passado e produziria uma reclamação em loop apertado. |

## Correio

`MAIL_DRIVER` tem padrão **`log`** - mail de saída é impresso no
subscriber de tracing configurado em vez de alcançar a rede. Mude para
`memory` em testes, `file` para previews `.eml` que você pode abrir em um
cliente de mail, e `smtp`/`ses`/etc. em produção. As chaves/tokens
específicos de provedor são obrigatórios somente quando o driver é
selecionado; um valor de driver desconhecido registra um `warn!` e recai
para `log`.

| Var | Padrão | Tipo | Propósito |
|---|---|---|---|
| `MAIL_DRIVER` | `"log"` | `String` (`log`, `memory`, `file`, `smtp`, `ses`, `sendgrid`, `mailgun`, `postmark`, `resend`) | Seleciona o alvo de inicialização. |
| `MAIL_FROM` | nenhum - obrigatório pelas facades de auth-flow | `String` | Endereço de remetente padrão para as facades de auth-flow (`EmailVerification`, `PasswordReset`, `TwoFactor`). Obrigatório para esses caminhos; se estiver ausente, dá erro no call site em vez de recair silenciosamente para um placeholder que quebraria DMARC/SPF. |
| `MAIL_FROM_NAME` | não definido | `String` | Nome de exibição opcional para o `From` do auth-flow (desde a **0.5.9**). Quando definido, o header renderiza `Nome <MAIL_FROM>`; `MAIL_FROM` permanece um endereço puro. Lido no momento do envio, então também se aplica a mail de auth-flow enfileirado. |

### File (`MAIL_DRIVER=file`)

| Var | Padrão | Tipo | Propósito |
|---|---|---|---|
| `MAIL_FILE_PATH` | `storage_path("mail")` | `String` | Diretório no qual um arquivo `.eml` RFC 5322 é escrito por envio. Nunca é limpo. Caminhos absolutos são usados como fornecidos; caminhos relativos usam o diretório base da aplicação (veja `APP_BASE_PATH`). |

### SMTP (`MAIL_DRIVER=smtp`)

| Var | Padrão | Tipo | Propósito |
|---|---|---|---|
| `MAIL_SMTP_HOST` | `"127.0.0.1"` | `String` | Host SMTP. |
| `MAIL_SMTP_PORT` | `587` | `u16` | Porta SMTP. |
| `MAIL_SMTP_USER` | não definido | `String` | Usuário SMTP. Tanto `MAIL_SMTP_USER` quanto `MAIL_SMTP_PASS` precisam estar definidos para um transporte criptografado; sem nenhum dos dois, a conexão recai para o modo local-catcher não criptografado. Definir exatamente um deles avisa no boot. |
| `MAIL_SMTP_PASS` | não definido | `String` | Senha SMTP. Veja `MAIL_SMTP_USER` para o comportamento com credenciais parciais. |
| `MAIL_SMTP_ENCRYPTION` | derivado | `starttls` \| `tls` \| `none` | Como a conexão é criptografada. Não definido deriva das credenciais: `starttls` quando as duas estão definidas, `none` quando nenhuma está. `tls` seleciona TLS implícito (porta 465). `ssl` e `null` são aceitos como aliases compatíveis com Laravel. Um valor não reconhecido falha o boot em **todo** ambiente - um erro de digitação não pode degradar para texto claro. |
| `MAIL_ALLOW_INSECURE_SMTP_IN_PRODUCTION` | não definido | `bool`-ish | Produção se recusa a inicializar numa conexão SMTP não criptografada. Defina como `1`/`true`/`yes`/`on` para reconhecer o uso de texto claro - defensável somente quando o relay é alcançável exclusivamente por uma rede privada. |

### Postmark (`MAIL_DRIVER=postmark`)

| Var | Padrão | Tipo | Propósito |
|---|---|---|---|
| `MAIL_POSTMARK_TOKEN` | obrigatório pelo driver | `String` | Token de servidor do Postmark. |
| `MAIL_POSTMARK_ENDPOINT` | padrão do Postmark | `String` | Sobrescreve o endpoint da API (regional ou mock server). |

### Amazon SES (`MAIL_DRIVER=ses`)

| Var | Padrão | Tipo | Propósito |
|---|---|---|---|
| `MAIL_SES_ACCESS_KEY` | obrigatório pelo driver | `String` | Chave de acesso AWS. |
| `MAIL_SES_SECRET_KEY` | obrigatório pelo driver | `String` | Chave secreta AWS. |
| `MAIL_SES_REGION` | `"us-east-1"` | `String` | Região AWS. |
| `MAIL_SES_ENDPOINT` | padrão da AWS para a região | `String` | Sobrescreve o endpoint do SES (regional ou mock server). |

### SendGrid (`MAIL_DRIVER=sendgrid`)

| Var | Padrão | Tipo | Propósito |
|---|---|---|---|
| `MAIL_SENDGRID_API_KEY` | obrigatório pelo driver | `String` | Chave de API do SendGrid. |
| `MAIL_SENDGRID_ENDPOINT` | padrão do SendGrid | `String` | Sobrescreve o endpoint da API. |

### Mailgun (`MAIL_DRIVER=mailgun`)

| Var | Padrão | Tipo | Propósito |
|---|---|---|---|
| `MAIL_MAILGUN_API_KEY` | obrigatório pelo driver | `String` | Chave de API do Mailgun. |
| `MAIL_MAILGUN_DOMAIN` | obrigatório pelo driver | `String` | Domínio de envio do Mailgun. |
| `MAIL_MAILGUN_ENDPOINT` | padrão do Mailgun | `String` | Sobrescreve o endpoint da API (ex.: EU vs US). |

### Resend (`MAIL_DRIVER=resend`)

| Var | Padrão | Tipo | Propósito |
|---|---|---|---|
| `MAIL_RESEND_API_KEY` | obrigatório pelo driver | `String` | Chave de API do Resend. |
| `MAIL_RESEND_ENDPOINT` | padrão do Resend | `String` | Sobrescreve o endpoint da API. |

## Limitação de taxa

| Var | Padrão | Tipo | Propósito |
|---|---|---|---|
| `RATE_LIMIT_DRIVER` | `memory` | `String` (`memory`, `redis`) | Seleciona o backend do rate-limiter. Fora de produção, um valor desconhecido registra um `warn!` e recai para `memory`; **em produção, `memory` - incluindo via um valor desconhecido - falha o boot** a menos que `RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION` esteja definida. |
| `RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION` | não definido | `bool`-ish | Reconhece buckets de rate-limit por processo em produção. Só é preciso se você executa exatamente um processo: detrás de N réplicas, toda cota é efetivamente N× e reseta a cada deploy. |
| `RATE_LIMIT_REDIS_URL` | `"redis://127.0.0.1:6379"` | `String` | URL do Redis (obrigatória pelo driver quando `RATE_LIMIT_DRIVER=redis`). |
| `RATE_LIMIT_PREFIX` | `"suprnova:"` | `String` | Prefixo de chave no Redis. |

## Imagens

Seleção do driver de imagens e os limites de decodificação que contêm
entrada hostil. Limites fora do intervalo são limitados com um `warn!` em
vez de falhar o boot: um limite de zero rejeitaria toda imagem da
aplicação. Um `IMAGE_DRIVER` desconhecido falha no primeiro uso, nomeando
os valores válidos.

| Var | Padrão | Tipo | Propósito |
|---|---|---|---|
| `IMAGE_DRIVER` | `oxideav` | `String` (`oxideav`, `magick`) | Seleciona o backend de imagens. `oxideav` é Rust puro, sem dependência do host; `magick` chama um ImageMagick 7 instalado no host para um suporte mais amplo de entrada. Insensível a maiúsculas/minúsculas. |
| `IMAGE_MAX_DIMENSION` | `16384` | `u32` | Limite de largura e altura de uma imagem decodificada, conferido contra o cabeçalho da própria entrada antes de qualquer coisa ser alocada. Também limita os alvos de redimensionamento. Mínimo `1`. |
| `IMAGE_MAX_ALLOC_BYTES` | `268435456` (256 MiB) | `u64` | Limite da pegada RGBA decodificada (`width * height * 4`). Também limita o tamanho do próprio arquivo de origem, venha ele de um caminho, de um disco ou de `Image::from_stream` (que confere enquanto coleta). Mínimo `4`. |
| `IMAGE_MAGICK_BINARY` | `magick` | `String` | Binário que o driver `magick` invoca. Somente ImageMagick 7; o nome `convert` do ImageMagick 6 não é aceito. Um binário ausente é um erro claro no primeiro uso. |
| `IMAGE_MAGICK_TIMEOUT_SECS` | `30` | `u32` | Teto de tempo de relógio para uma única invocação do ImageMagick. É ao mesmo tempo o argumento `-limit time` do próprio ImageMagick e o prazo do lado Rust que mata o grupo de processos inteiro do filho dois segundos depois, porque `-limit time` é aplicado por um monitor que um filho travado dentro de um delegate nunca aciona. Limita um delegate travado que de outra forma seguraria um worker bloqueante pela vida do processo. Somente driver `magick`. Mínimo `1`. |

Veja [Imagens](images.md) para a aplicação de limites em duas camadas e
para saber como escolher entre os drivers.

## Hashing

Driver de hashing de senha e parâmetros por algoritmo. Valores
inválidos retornam um `FrameworkError::param` no primeiro hash,
trazendo a configuração incorreta à tona imediatamente em vez de
recair silenciosamente para o padrão.

| Var | Padrão | Tipo | Propósito |
|---|---|---|---|
| `HASH_DRIVER` | `bcrypt` | `String` (`bcrypt`, `argon`/`argon2i`, `argon2id`) | Algoritmo de hashing ativo. Insensível a maiúsculas/minúsculas. |
| `HASH_ROUNDS` | `12` | `u32` | Custo do Bcrypt (intervalo `4..=31`). Valores fora do intervalo falham com um erro claro. |
| `HASH_MEMORY` | `65536` (64 MiB, em unidades KiB) | `u32` | Memória do Argon2 em KiB. Mínimo `8`. Somente Argon. |
| `HASH_TIME` | `4` | `u32` | Tempo / iterações do Argon2. Mínimo `1`. Somente Argon. |
| `HASH_THREADS` | `1` | `u32` | Paralelismo do Argon2 (corresponde a OWASP / libsodium). Mínimo `1`. Somente Argon. |
| `HASH_VERIFY` | `false` | `bool` | Quando `true`, `verify()` rejeita hashes de um algoritmo diferente de `HASH_DRIVER` (retorna `Ok(false)`). O padrão é `false` para que hashes bcrypt legados ainda verifiquem depois de uma troca de driver, até serem rotacionados. |

## Validação

| Var | Padrão | Tipo | Propósito |
|---|---|---|---|
| `HIBP_TIMEOUT_SECS` | `30` (segundos) | `u64` | Timeout de solicitação para a verificação de intervalo do Have I Been Pwned de `Password::uncompromised()`, lido de novo a cada vez que um `HibpVerifier` padrão é construído. Um HIBP lento ou inalcançável ainda falha aberto - veja [Validação](validation.md). |

## Fluxos de autenticação

A autenticação de dois fatores usa `APP_NAME` (coberta em Aplicação)
como a string de issuer do TOTP - não existe uma variável de ambiente
dedicada `2FA_ISSUER`. O issuer recai para `"Suprnova"` quando
`APP_NAME` não está definida.

## Inertia / Frontend

| Var | Padrão | Tipo | Propósito |
|---|---|---|---|
| `SUPRNOVA_FRONTEND` | `svelte` | `String` (`svelte`, `react`, `vue`) | Frontend ativo. Insensível a maiúsculas/minúsculas. Dirige `Frontend::detect_from_env()`, o ponto de entrada padrão do Vite, e a ordem de busca de extensão de componente de página em tempo de compilação. Valores desconhecidos ou não definidos recaem para `svelte`. |

## Modo de manutenção

| Var | Padrão | Tipo | Propósito |
|---|---|---|---|
| `MAINTENANCE_DRIVER` | `file` | `String` (`file`, `cache`) | Seleciona como o estado de `down`/`up` é armazenado. `file` escreve no caminho de storage do framework; `cache` roda sobre o driver de cache configurado (útil quando muitas instâncias de app precisam coordenar o estado de manutenção). Qualquer outro valor recai para `file`. |

## Eventos

| Var | Padrão | Tipo | Propósito |
|---|---|---|---|
| `EVENT_MAX_CONCURRENCY` | `256` | `usize` | Teto de tasks de listener enfileiradas concorrentes. Valores `<= 0` ou não parseáveis recaem para o padrão. Aplica-se a `Event::queue` / listeners enfileirados; listeners síncronos não estão sujeitos a esse limite. |

## Logs

`LOG_FORMAT` é **consciente do ambiente**: em produção
(`APP_ENV=production`) o padrão é `json`, amigável para agregadores de
log; em todo o resto o padrão é `pretty`, para saída local/dev legível
por humanos. Um valor explícito sempre vence.

| Var | Padrão | Tipo | Propósito |
|---|---|---|---|
| `LOG_LEVEL` | `"info"` | `String` (`error`, `warn`, `info`, `debug`, `trace` - insensível a maiúsculas/minúsculas) | Nível de filtro do tracing-subscriber. |
| `LOG_FORMAT` | consciente do ambiente (`json` em produção, `pretty` no resto) | `String` (`json`, `pretty`) | Formato de saída do tracing-subscriber. |

## Observabilidade (OpenTelemetry)

| Var | Padrão | Tipo | Propósito |
|---|---|---|---|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | não definido (telemetria desabilitada) | `String` | Endpoint do collector OTLP. Quando não definido (ou só espaços em branco), exportadores não são instalados e o framework continua usando o subscriber padrão do `tracing`. |
| `OTEL_SERVICE_NAME` | `"suprnova"` | `String` | Atributo de recurso `service.name` em todo span / métrica / registro de log. |
| `OTEL_SERVICE_VERSION` | `CARGO_PKG_VERSION` em tempo de build | `String` | Atributo de recurso `service.version`. |
| `OTEL_SDK_DISABLED` | `false` | `bool` | Kill switch padrão do OTel. Quando `true`, exportadores não são instalados independentemente de `OTEL_EXPORTER_OTLP_ENDPOINT`. |

## CLI / servidor de desenvolvimento

Estas são lidas pelo binário da CLI `suprnova` (servidor de dev,
worker SSR), e não pelo framework em runtime - elas aparecem no `.env`
inicial ou são honradas por `suprnova serve` / `suprnova ssr:*`.

| Var | Padrão | Tipo | Propósito |
|---|---|---|---|
| `VITE_PORT` | `5765` | `u16` | Porta em que o Vite faz bind dentro de `suprnova serve`. A flag `--frontend-port` da CLI sobrescreve. |
| `SUPRNOVA_SSR_RUNTIME` | `"node"` | `String` | Runtime sob o qual lançar o worker SSR (`suprnova ssr:start`). A flag `--runtime` da CLI sobrescreve. |
| `SUPRNOVA_SSR_BUNDLE` | `frontend/bootstrap/ssr/ssr.js` | `Path` | Caminho para o bundle SSR já construído. A flag `--bundle` da CLI sobrescreve. |
| `SUPRNOVA_SSR_URL` | `"http://127.0.0.1:13714"` | `String` | URL do worker SSR, para `suprnova ssr:check`. A flag `--url` da CLI sobrescreve. |

## Subsistemas sem variáveis de ambiente

Alguns subsistemas são configurados inteiramente em código Rust via o
contêiner ou registro de serviço - eles têm **zero** variáveis de
ambiente que o framework lê:

- **Sistema de arquivos / Storage.** Discos são registrados com
  `FilesystemRegistry::add_disk(name, driver)` em `bootstrap()`. Não
  existe uma variável de ambiente `FILESYSTEM_DISK` (o nome aparece
  em alguns arquivos `.env` iniciais, mas não é consultado pelo
  framework - veja "Variáveis que o framework não lê" abaixo).
- **Transmissão e WebSockets.** Canais são registrados com a macro
  `ws!()` e a configuração de `BroadcastHub` no código. O próprio
  driver roda sobre o que quer que o `CACHE_DRIVER` configurado
  selecione.
- **CORS, CSRF, Idempotência, Timeout.** Configurados via structs
  construtoras passadas aos construtores de middleware em
  `bootstrap()`. Os padrões são conservadores o bastante para que uma
  app típica nunca precise tocá-los.
- **Magnetar e OAuth.** `MagnetarConfig` é construído no bootstrap da
  aplicação. O starter de API lê `PASSKEY_RP_ID` e `PASSKEY_RP_ORIGIN`,
  mas o framework não. IDs de provedor OAuth, secrets, URLs de callback,
  escopos, transportes e valores de política são fornecidos
  programaticamente pelo registro de provedores do Magnetar. Aplicações
  podem obter esses valores de variáveis de ambiente ou de um gerenciador
  de secrets.
- **Busca vetorial, Notificações, Pagamentos, Sinalizadores de
  recursos.** Cada um registra drivers concretos via `App::bind` em
  `bootstrap()`. Escolha seu driver em Rust; passe qualquer URL/chave
  que ele precise como suas próprias variáveis de ambiente.

## Variáveis que o framework não lê

O `.env` inicial gerado por scaffold lista algumas chaves por
conveniência de quem escreve o arquivo à mão, que o framework nunca
consulta. Elas estão documentadas aqui para que um leitor que as
procure não fique se perguntando:

- `MAIL_FROM_ADDRESS` - um placeholder no estilo Laravel que o
  framework nunca consulta. O endereço de remetente que as facades de
  auth-flow de fato usam é `MAIL_FROM` (coberta em Mail). Seus
  próprios tipos `Mailable` podem lê-lo via `env_optional` se você
  quiser manter o nome do Laravel, mas nada em `suprnova::*` faz isso.
  (`MAIL_FROM_NAME` **é** lida desde a 0.5.9 - veja o capítulo de
  Mail - então ela não está mais listada aqui.)
- `FILESYSTEM_DISK` - placeholder para o nome do disco padrão. Em vez
  disso, defina o padrão no código via
  `FilesystemRegistry::set_default(name)`.

## Como os valores são parseados

Uma referência rápida para as três variantes do helper de env - veja
[Configuração](configuration.md#direct-env-access) para o tratamento
completo:

| Helper | Comportamento quando faltando | Comportamento quando não parseável |
|---|---|---|
| `env(key, default)` | retorna `default` | `warn!` + retorna `default` |
| `env_required(key)` | **entra em pânico** | **entra em pânico** |
| `env_optional(key)` | retorna `None` | `warn!` + retorna `None` |
| `env_strict(key)` (interno, usado por `try_from_env`) | retorna `Ok(None)` | retorna `Err(FrameworkError)` - o boot aborta |

As variantes estritas (`AppConfig::try_from_env`,
`ServerConfig::try_from_env`) são o que `Config::init` chama, então um
erro de digitação em `APP_DEBUG=tru` ou `SERVER_PORT=80a0` aborta o
boot com um erro estruturado em vez de reverter silenciosamente para
o padrão. As variantes lenientes existem para a população mais ampla
de call sites (incluindo `impl Default`) onde uma falha de parse não
pode entrar em pânico.

## Sobrescritas por ambiente

O loader lê arquivos nesta ordem, cada um sobrescrevendo o anterior:

1. `.env`
2. `.env.<environment>` (ex.: `.env.production`, `.env.staging`,
   `.env.testing`, `.env.<custom>` para `APP_ENV=<custom>`)
3. Ambiente do processo

Isso significa que um deploy de produção em contêiner pode enviar um
`.env.production` mínimo, sobrescrevendo apenas as chaves que diferem
de `.env` (nomes de driver, URLs, material de chave), e o ambiente
real do contêiner sobrescreve os dois para segredos que nunca devem
cair num arquivo commitado.

Veja [Configuração](configuration.md#how-env-loading-works) para o
comportamento exato do loader e o rastreamento de `LOADED_KEYS` que
impede que valores obsoletos do `.env` sejam promovidos para o nível
de "env real do sistema" entre reloads.

## Próximos passos

- [Configuração](configuration.md) - registro tipado de `Config::*`,
  os helpers `env*`, detecção de ambiente
- [Implantação](deployment.md) - o que definir em produção
- [Criptografia](encryption.md) - rotação de `APP_KEY` via
  `APP_KEY_PREVIOUS`
- [Inicialização da aplicação](bootstrap.md) - onde a ordem de boot
  dirigida por env é estabelecida
