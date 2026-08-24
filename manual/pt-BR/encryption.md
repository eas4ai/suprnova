# Criptografia

O Suprnova traz criptografia no nível da aplicação como uma facade
global ao processo chamada `Crypt`. Ela criptografa strings ou qualquer
valor `Serialize` sob AES-256-GCM, chaveado pela sua `APP_KEY`. Recorra a
ela sempre que precisar colocar algo sensível em um armazenamento em que
você não confia totalmente - uma coluna, um cookie, um cursor de
paginação - e precisar lê-lo de volta intacto depois.

```rust
use suprnova::{Crypt, CryptPurpose};

let wire = Crypt::encrypt_string(CryptPurpose::Cast, "ssn-123-45-6789")?;
let plain = Crypt::decrypt_string(CryptPurpose::Cast, &wire)?;
assert_eq!(plain, "ssn-123-45-6789");
```

O próprio framework usa `Crypt` para cookies criptografados, cursores de
paginação criptografados, secrets de 2FA, códigos de recuperação, e os
casts Eloquent `AsEncrypted*`. A mesma facade está disponível para o seu
código sem nenhuma conexão extra, uma vez que `APP_KEY` esteja configurada
(veja [configuration.md](configuration.md#the-env-file)).

## O formato de wire

`encrypt_string` e `encrypt` retornam ambos base64 URL-safe (sem
padding) sobre `nonce || ciphertext_with_tag`:

```
base64url( [nonce aleatório de 12 bytes] || [texto cifrado] || [tag GCM de 16 bytes] )
```

Cada chamada amostra um nonce novo de 12 bytes a partir do RNG do SO,
então duas criptografias do mesmo texto puro sob a mesma chave produzem
textos cifrados distintos. Não há padding oracle para vazar informação de
comprimento além do próprio texto puro.

A saída é segura para colocar em query strings de URL, corpos JSON,
headers, e cookies sem codificação adicional. Um wire válido mínimo tem 28
bytes (12 de nonce + 16 de tag) - qualquer coisa mais curta é rejeitada de
antemão.

## `APP_KEY` - o único segredo que importa

O Suprnova lê uma única chave simétrica de 32 bytes a partir da variável
de ambiente `APP_KEY`. O formato esperado é base64 URL-safe, sem padding,
decodificando para exatamente 32 bytes (43 caracteres base64):

```env
APP_KEY=hQ7rW0X9_NkSi8Cw5fF8j6V_K6JzgB3y2Hq9LpL9-Wo
```

Gere uma com a CLI:

```bash
suprnova key:generate
# Gerou uma nova APP_KEY (AES-256, base64 segura para URL, sem padding):
#
#     hQ7rW0X9_NkSi8Cw5fF8j6V_K6JzgB3y2Hq9LpL9-Wo
#
# Adicione-a ao seu .env (ou ao seu gerenciador de segredos):
#
#     APP_KEY=hQ7rW0X9_NkSi8Cw5fF8j6V_K6JzgB3y2Hq9LpL9-Wo
```

Ou direcione direto para o ambiente:

```bash
echo "APP_KEY=$(suprnova key:generate --show)" >> .env
```

### Validação no boot - fail closed

`Server::from_config` valida `APP_KEY` **em todo boot**, não só no
primeiro. As regras:

| Ambiente | `APP_KEY` não definida | `APP_KEY` malformada |
|---|---|---|
| `local`, `development`, `testing` | Gera uma chave transitória, avisa nos logs | Erro rígido - falha o boot |
| `staging`, `production`, qualquer outra coisa | Erro rígido - falha o boot | Erro rígido - falha o boot |

Uma chave malformada é **sempre** um erro rígido, mesmo em `local` - é
melhor falhar o boot do que mascarar um erro de digitação. Um valor de
ambiente `Custom` que o framework não reconhece (por exemplo,
`APP_ENV=k8s`) é tratado como semelhante a produção: sem `APP_KEY`, sem
boot.

O diagnóstico aponta para a correção:

```
APP_KEY is required when APP_ENV=production. Generate one with
`suprnova key:generate` and set it in your environment (e.g. .env
or your secrets manager). Suprnova refuses to boot without an
encryption key outside of local/development/testing because session
cookies and pagination cursors would otherwise be unsigned and
forgeable.
```

## `CryptPurpose` - separação de domínio via AAD

Toda chamada a `Crypt::*` recebe um `CryptPurpose`. A variante mapeia
para um label de bytes estável que é vinculado à tag de autenticação
AES-GCM como Associated Data (AAD):

```rust
pub enum CryptPurpose {
    Cookie,            // suprnova:cookie:v1
    Cursor,            // suprnova:cursor:v1
    TwoFactorSecret,   // suprnova:2fa:secret:v1
    TwoFactorRecovery, // suprnova:2fa:recovery:v1
    Cast,              // suprnova:cast:v1
}
```

O label **não** é armazenado no wire. O GCM mistura o AAD na tag de
autenticação sem incluí-lo no texto cifrado, então:

- O formato on-wire permanece inalterado - ainda
  `base64(nonce || ciphertext || tag)`.
- Um wire produzido sob `CryptPurpose::Cookie` é **rejeitado** por
  qualquer chamada de decrypt que forneça um purpose diferente. A
  verificação da tag GCM falha antes de qualquer parsing pós-decrypt
  executar.
- Adicionar uma nova superfície (uma futura criptografia de payload de
  fila, um header de arquivo criptografado) significa adicionar uma nova
  variante - não mudar o formato do wire.

```rust
use suprnova::{Crypt, CryptPurpose};

let wire = Crypt::encrypt_string(CryptPurpose::Cookie, "session-id")?;

// Mesma chave, mesmo wire, purpose diferente - falha.
let result = Crypt::decrypt_string(CryptPurpose::Cursor, &wire);
assert!(result.is_err());

// Mesmo purpose - sucesso.
let plain = Crypt::decrypt_string(CryptPurpose::Cookie, &wire)?;
```

### Por que Suprnova diverge

O `Crypt::encryptString` do Laravel não recebe um purpose. A única
`APP_KEY` é reutilizada entre cookies, URLs assinadas, tokens de
expiração assinados, e qualquer chamada de usuário a `Crypt::encrypt`,
sem separação de domínio na camada de criptografia. Se duas superfícies
aceitarem, por acaso, texto cifrado do mesmo formato de texto puro, um
valor cunhado para uma superfície pode sofrer replay na outra.

O Suprnova reutiliza a mesma `APP_KEY` pelo mesmo motivo - operadores
gerenciam um único segredo - mas vincula cada superfície ao seu próprio
label de AAD. O replay de texto cifrado entre superfícies é rejeitado na
verificação da tag GCM, antes de qualquer parsing executar. O custo para
o chamador é um parâmetro de enum extra; o ganho é uma propriedade que o
formato do wire por si só não consegue quebrar.

O sufixo `:v1` em cada label é reservado para uma futura rotação por
superfície: subir `suprnova:cookie:v1` para `suprnova:cookie:v2` invalida
**somente** o texto cifrado antigo de cookie - deixa cursores, secrets de
2FA, e colunas de cast intactos.

## AAD vinculada ao nome do cookie (v2)

Cookies criptografados usam uma segunda geração de AAD quando o chamador
conhece o nome lógico do cookie. `Cookie::encrypted("suprnova_session", value)`
vincula `suprnova:cookie:v2:suprnova_session` à tag GCM, e
`Cookie::read_encrypted_for("suprnova_session", wire)` fornece o mesmo
contexto no caminho de volta:

```rust
use suprnova::Cookie;

let cookie = Cookie::encrypted("suprnova_session", "session-id")?;
let wire = cookie.value().to_string();
assert_eq!(
    Cookie::read_encrypted_for("suprnova_session", &wire)?,
    "session-id"
);
assert!(Cookie::read_encrypted_for("other_cookie", &wire).is_err());
```

O nome vinculado é lógico, não o nome renderizado. Portanto, um prefixo de
nome na rede como `__Host-` ou `__Secure-` não muda a AAD nem desconecta
usuários. O prefixo é uma preocupação do navegador e do header; o nome do
cookie é o domínio criptográfico.

### A janela de compatibilidade

O formato de rede permanece inalterado e sem versão: ele ainda carrega apenas
o nonce, o texto cifrado e a tag de autenticação. Não há byte de versão que
permita ao leitor escolher um ramo. `decrypt_string_for` usa uma tentativa
cega de decrypt com o mesmo formato da rotação de chave: tenta a AAD v2 com
contexto em todo o key ring, e depois a AAD v1 sem contexto em todo o ring.
Isso mantém legíveis os cookies escritos antes da vinculação de nome enquanto
a rotação de `APP_KEY` também está em andamento.

A janela preserva a antiga fraqueza de replay durante toda a sua duração.
Um cookie v1 de um slot ainda pode sofrer replay em outro enquanto existir o
fallback sem contexto; o benefício da vinculação de nome começa quando esse
fallback for removido em 1.4.0. Nada remove o fallback automaticamente:
`Crypt::encrypt_string(CryptPurpose::Cookie, ...)` ainda cunha v1, e a
entrada sem contexto foi substituída, com remoção agendada para 1.4.0. Mova as
escritas de cookie para `Cookie::encrypted` e as leituras para
`read_encrypted_for` antes desse prazo.

Há um custo mensurável durante a janela. Um decrypt de cookie malsucedido
paga duas passagens de tentativa pelo ring. O middleware de sessão faz duas
leituras criptografadas por solicitação quando há um cookie de sessão e um
cookie remember-me, então uma solicitação anônima com um cookie remember
obsoleto paga `2 × (1 + N)` duas vezes, onde `N` é o número de chaves
anteriores.

### Lendo `DecryptOrigin`

`Crypt::decrypt_string_for_inner` retorna um `DecryptOrigin` com dois eixos
independentes:

- `origin.key = KeyOrigin::Previous(index)` significa que o valor ainda
  depende de `APP_KEY_PREVIOUS[index]`. Re-criptografe o valor sob a chave
  atual e remova essa chave anterior somente depois que a cauda da rotação
  desaparecer.
- `origin.aad = AadVersion::Legacy` significa que o valor usou o fallback
  v1 sem contexto. Para um cookie, emita-o novamente pela API vinculada ao
  nome; o fallback está agendado para remoção em 1.4.0.

Os dois eixos podem estar obsoletos juntos. O leitor público registra os
avisos correspondentes sem incluir texto claro ou texto cifrado. Trate o
aviso de chave como uma tarefa de limpeza de rotação e o aviso de AAD como
uma tarefa de migração; combinar em um eixo não deve ocultar o outro.

## Os dois pares de encrypt / decrypt

Há dois formatos para dois casos de uso.

### Strings - `encrypt_string` / `decrypt_string`

Para strings UTF-8:

```rust
use suprnova::{Crypt, CryptPurpose};

let wire: String =
    Crypt::encrypt_string(CryptPurpose::Cast, "alice@example.com")?;

let plain: String =
    Crypt::decrypt_string(CryptPurpose::Cast, &wire)?;
```

O caminho de decrypt retorna uma `String` - bytes não-UTF-8 (que uma
execução normal de encrypt não pode produzir, mas que um wire corrompido
ou fornecido por um atacante pode) surgem como um
`FrameworkError::Internal` claro.

### Qualquer coisa `Serialize` - `encrypt` / `decrypt`

Para valores estruturados, codifique em JSON e depois criptografe em uma
única chamada:

```rust
use serde::{Serialize, Deserialize};
use suprnova::{Crypt, CryptPurpose};

#[derive(Serialize, Deserialize)]
struct Secret {
    api_key: String,
    last_rotated_at: chrono::DateTime<chrono::Utc>,
}

let value = Secret {
    api_key: "sk_live_…".into(),
    last_rotated_at: chrono::Utc::now(),
};

let wire = Crypt::encrypt(CryptPurpose::Cast, &value)?;
let round_trip: Secret = Crypt::decrypt(CryptPurpose::Cast, &wire)?;
```

O formato do wire é o mesmo - base64 sobre `nonce || ciphertext ||
tag` - a única diferença é que o texto puro são bytes `serde_json` de
`value`, em vez de UTF-8 de uma string. Use isso para qualquer formato de
registro: um blob de config, um payload de sessão, uma tupla de argumento
de fila.

### `appears_encrypted` - verificação de formato, não de violação

Para middleware que precisa pular valores já criptografados na passagem
de saída (correspondendo ao comportamento do `EncryptCookies` do
Laravel), `Crypt::appears_encrypted` faz uma verificação heurística
barata:

```rust
if Crypt::appears_encrypted(cookie_value) {
    // passa direto - já envolvido
} else {
    // criptografa antes de enviar
}
```

Ela retorna `true` quando a entrada decodifica como base64 URL-safe e o
comprimento decodificado é de pelo menos 28 bytes (nonce + tag). Ela
nunca chama o AES-GCM, então **não consegue** distinguir um texto cifrado
válido de bytes aleatórios do formato certo. Chamadores que precisam de
autenticação devem chamar `decrypt_string` / `decrypt` e tratar o erro.

## Rotação de chave - o keyring

O Suprnova suporta rotação sem downtime através de um *ring* de chaves:
uma chave atual (usada para toda criptografia nova) mais uma lista
ordenada de chaves anteriores (tentadas como fallback no decrypt). Você
rotaciona `APP_KEY` sem precisar re-criptografar toda coluna em
lock-step.

Defina `APP_KEY_PREVIOUS` como uma lista separada por vírgulas de chaves
base64, da mais antiga para a mais nova:

```env
APP_KEY=<new key>
APP_KEY_PREVIOUS=<old key>
# Ou para rotação em vários passos (mais antiga → mais nova):
APP_KEY_PREVIOUS=<oldest>,<middle>,<previous>
```

`APP_KEY_PREVIOUS` é o nome canônico do Suprnova.
`APP_PREVIOUS_KEYS` é aceito como um alias compatível com Laravel. Se ambas as
variáveis estiverem definidas, `APP_KEY_PREVIOUS` vence. Quando seus valores
aparados diferirem, o boot registra um aviso e ignora `APP_PREVIOUS_KEYS`.


A criptografia **sempre** usa a chave atual. A descriptografia tenta a
chave atual primeiro; se isso falhar, cada chave anterior é tentada em
ordem. Em um acerto de chave anterior, `Crypt` emite um `tracing::warn!`:

```
WARN previous_index=0 Crypt decrypted a value with APP_KEY_PREVIOUS[0];
re-encrypt (load + save) this row under the current APP_KEY and remove
the corresponding APP_KEY_PREVIOUS entry once the rotation completes.
```

A linha de log deliberadamente exclui tanto o texto puro quanto o texto
cifrado - só o fato-da-rotação mais uma dica acionável viajam. Operadores
que rodam uma busca de log por `APP_KEY_PREVIOUS` caem em toda coluna que
ainda depende de uma chave antiga.

### O teto - `MAX_PREVIOUS_KEYS = 8`

`APP_KEY_PREVIOUS` tem um teto de 8 entradas. Uma cadeia de rotação
realista tem de 1 a 3 entradas (uma rotação em andamento, talvez uma
rotação anterior travada que o operador não limpou); 8 deixa uma margem
generosa. Além do teto, o boot **falha de forma explícita** com um
diagnóstico que nomeia tanto a contagem quanto o teto:

```
APP_KEY_PREVIOUS holds 12 keys; the maximum is 8. A realistic
rotation chain is 1-3 entries - a longer list is almost always a
config-templating accident. Trim the list to the keys still needed
for in-flight rotation; once a re-encrypt job has migrated every
row off an old key, drop that entry.
```

Um truncamento silencioso derrubaria uma chave da qual o operador ainda
pode depender, deixando colunas indescriptografáveis sem nenhum
diagnóstico. O teto rígido é intencional.

Entradas vazias são toleradas:
`APP_KEY_PREVIOUS=,,,old1,,,old2,,,` é parseado para duas chaves reais.
Uma entrada malformada (erro de digitação, comprimento errado, base64
inválido) é um erro rígido - segredos parcialmente rotacionados falham o
boot, em vez de descartar silenciosamente um fallback.

### Procedimento de rotação

```bash
# 1. Cunhe uma chave nova.
NEW=$(suprnova key:generate --show)

# 2. Mova a chave atual para APP_KEY_PREVIOUS, instale a nova.
#    Edite seu .env ou secrets manager:
#
#      APP_KEY_PREVIOUS=<old_value_of_APP_KEY>
#      APP_KEY=<NEW>

# 3. Faça o deploy. Escritas novas usam a chave nova; linhas
#    existentes continuam a descriptografar via o fallback de
#    chave anterior. Os logs identificam as colunas ainda na
#    chave antiga.

# 4. Execute uma passagem de re-encrypt. Para cada model com
#    casts criptografados:
#
#      User::query().chunk(500, |batch| async {
#          for mut row in batch { row.save().await?; }
#          Ok(())
#      }).await?;
#
#    `Cast::to_storage` sempre usa a chave atual, então um
#    load-then-save no-op migra a linha.

# 5. Quando os warnings pararem de aparecer nos logs, remova
#    APP_KEY_PREVIOUS e faça o deploy de novo.
```

O procedimento inteiro é online - em nenhum momento há uma janela em que
solicitações novas falhem.

### Observando o ring

Para dashboards de operador ou health checks:

```rust
use suprnova::Crypt;

if Crypt::has_previous_keys() {
    let n = Crypt::previous_key_count();
    tracing::info!(previous_keys = n, "APP_KEY rotation in progress");
}
```

Os bytes da chave em si nunca são acessíveis a partir da API pública. O
impl de `Debug` de `EncryptionKey` imprime `"[REDACTED]"`, e não há
nenhum acessor que exponha uma chave bruta fora do crate.

## Integração com o Eloquent - os casts `AsEncrypted*`

A criptografia no nível da aplicação é mais útil na fronteira da coluna.
A família de casts `AsEncrypted*` envolve `Crypt::encrypt_string` para
que os campos do seu model permaneçam texto puro tipado em runtime e
texto cifrado em repouso:

```rust
use suprnova::{model, Model};
use suprnova::eloquent::casts::{
    AsEncrypted, AsEncryptedArray, AsEncryptedObject, AsEncryptedCollection,
};
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct ApiKey {
    pub provider: String,
    pub secret: String,
}

#[model(table = "users", casts = {
    api_token     = AsEncrypted,
    api_keys      = AsEncryptedArray<ApiKey>,
    billing       = AsEncryptedObject<BillingDetails>,
    ssh_keys      = AsEncryptedCollection<String>,
})]
pub struct User {
    pub id: i64,
    pub api_token: String,
    pub api_keys: Vec<ApiKey>,
    pub billing: BillingDetails,
    pub ssh_keys: suprnova::eloquent::Collection<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

| Cast | Tipo em runtime | Formato de armazenamento |
|---|---|---|
| `AsEncrypted` | `String` | string criptografada |
| `AsEncryptedArray<T>` | `Vec<T>` | JSON → string criptografada |
| `AsEncryptedObject<T>` | `T` | JSON → string criptografada |
| `AsEncryptedCollection<T>` | `Collection<T>` | JSON → string criptografada |

Todos os quatro passam por `CryptPurpose::Cast`. Um wire cunhado por um
cast criptografado é rejeitado por qualquer código que tente
descriptografá-lo como um cookie ou cursor - mesmo que a `APP_KEY` seja a
mesma, o label de AAD é diferente.

Para a superfície completa de casts, a tabela de modos de falha, e as
receitas de re-encryption, veja [eloquent.md](eloquent.md). A mecânica de
criptografia é a mesma da facade acima - o cast é açúcar que executa
`Crypt::encrypt_string(CryptPurpose::Cast, …)` na fronteira de
armazenamento.

### Criptografia vs hashing - escolha a ferramenta certa

`AsEncrypted` é **reversível**. O texto puro pode ser recuperado com
`APP_KEY`. Use-o para dados que a sua aplicação precisa ler de volta:
tokens de API que você exibe em uma página de configurações, secrets de
terceiros que você repassa para serviços upstream, endereços para os
quais você envia pedidos.

Para dados que a sua aplicação só precisa *verificar* - senhas, prefixos
de chave de API que você compara contra tokens de entrada - use um hash
em vez disso. Hashes são unidirecionais: não há texto puro para vazar,
mesmo que `APP_KEY` seja comprometida. Veja [hashing.md](hashing.md)
para a facade Bcrypt / Argon2id e o cast `AsHashed`.

## Onde mais o `Crypt` é usado dentro do framework

Você não precisa fazer nada para optar por isso - eles são conectados
automaticamente uma vez que `APP_KEY` esteja configurada.

- **Cookies criptografados** - `Cookie::encrypted(...)` /
  `Cookie::read_encrypted(...)` usam `CryptPurpose::Cookie`. O cookie de
  sessão, o cookie de remember-me, e o cookie de bypass do modo de
  manutenção todos andam sobre isso. Veja [responses.md](responses.md) e
  [session.md](session.md).
- **Paginação por cursor** - `CursorPaginator` codifica o cursor sob
  `CryptPurpose::Cursor` para que o valor on-wire `?cursor=…` não possa
  ser forjado ou sofrer replay entre superfícies. Veja
  [eloquent.md](eloquent.md#cursor-pagination).
- **Secrets de 2FA** - o secret TOTP em base32 criptografado em
  `two_factor_authentications.secret` usa
  `CryptPurpose::TwoFactorSecret`; códigos de recuperação usam
  `CryptPurpose::TwoFactorRecovery`. Purposes distintos previnem replay
  de texto cifrado entre colunas dentro da mesma linha. Veja
  [auth-flows.md](auth-flows.md).
- **Assinatura derivada de HMAC** - URLs assinadas e tokens de
  redefinição de senha derivam uma chave HMAC a partir de `APP_KEY`, em
  vez de criptografar sob ela. Os bytes brutos da chave não são
  exportados; a derivação vive dentro do framework. Veja
  [routing.md](routing.md#signed-urls).

## Testando com `Crypt`

A facade `Crypt` é apoiada em `OnceLock`, então o primeiro instalador em
um binário de teste vence. Os helpers de teste cuidam do boilerplate:

```rust
use suprnova::testing::install_test_encryption_key;

#[tokio::test]
async fn encrypts_and_round_trips() {
    install_test_encryption_key(); // idempotente - seguro chamar em todo teste

    let wire = suprnova::Crypt::encrypt_string(
        suprnova::CryptPurpose::Cast,
        "hello",
    ).unwrap();

    let plain = suprnova::Crypt::decrypt_string(
        suprnova::CryptPurpose::Cast,
        &wire,
    ).unwrap();

    assert_eq!(plain, "hello");
}
```

A chave de teste é determinística, então os testes podem descriptografar fixtures
estáveis e exercitar a rotação contra uma chave conhecida. Strings de texto
cifrado não devem ser comparadas para igualdade entre chamadas ou execuções: cada
criptografia ainda usa um nonce aleatório novo.

Para testes de rotação, instale um keyring diretamente e cunhe texto
cifrado histórico com `_test_encrypt_with`:

```rust
use suprnova::testing::install_test_encryption_keyring;
use suprnova::EncryptionKey;

let current = EncryptionKey::generate();
let old = EncryptionKey::generate();

install_test_encryption_keyring(current, vec![old.clone()]);

// Simula um valor escrito quando `old` era a chave atual.
let legacy_wire = suprnova::crypto::_test_encrypt_with(
    &old,
    suprnova::CryptPurpose::Cast,
    "legacy",
).unwrap();

// O ring atual o descriptografa via o fallback de chave anterior,
// emitindo a linha de warn de rotação.
let plain = suprnova::Crypt::decrypt_string(
    suprnova::CryptPurpose::Cast,
    &legacy_wire,
).unwrap();

assert_eq!(plain, "legacy");
```

Ambos os helpers são compilados fora dos binários de produção quando a
feature `testing` está desabilitada (`default-features = false`).

## Modos de falha - como os erros se parecem

Toda chamada falível de `Crypt::*` retorna `Result<_, FrameworkError>`.
Os cinco erros que você pode ver:

| Causa | Onde | Superfície |
|---|---|---|
| `Crypt` não inicializada | Qualquer chamada antes do boot | `FrameworkError::Internal("Crypt is not initialized - set APP_KEY before serving")` |
| Wire não é base64 válido | `decrypt_string`, `decrypt` | `FrameworkError::Internal("Crypt base64 decode failed: …")` |
| Wire curto demais (< 28 bytes) | `decrypt_string`, `decrypt` | `FrameworkError::Internal("AEAD wire too short …")` |
| Verificação da tag falha - chave errada, AAD errado, bytes violados | `decrypt_string`, `decrypt` | `FrameworkError::Internal("AEAD decrypt failed: …")` |
| Encode / decode de JSON falha | `encrypt`, `decrypt` | `FrameworkError::Internal("Crypt JSON {encode,decode} failed: …")` |

Não há fallback silencioso para lixo. Uma chave errada contra um texto
cifrado existente é sempre um erro rígido, tanto no nível da facade
quanto no nível do cast. Isso corresponde ao comportamento do
`Encrypter` do Laravel e é a propriedade que torna a rotação segura: uma
coluna esquecida surgiria imediatamente, em vez de retornar um texto
puro plausível-mas-errado.

Quando uma chave anterior descriptografa um wire com sucesso, a chamada
ainda retorna `Ok(...)` - mas a linha de `tracing::warn!` dispara junto,
então alertas dirigidos por log capturam a cauda da rotação antes de
`APP_KEY_PREVIOUS` ser removida.

## Próximos passos

- [configuration.md](configuration.md) - `APP_KEY`, `APP_ENV`, e o
  resto do ambiente de boot.
- [eloquent.md](eloquent.md) - os casts `AsEncrypted*`, a tabela
  completa de casts, e o procedimento de rotação para colunas de model.
- [hashing.md](hashing.md) - a alternativa unidirecional para quando
  você precisa *verificar*, não *recuperar*; as facades bcrypt e
  Argon2id, mais `AsHashed`.
- [auth-flows.md](auth-flows.md) - armazenamento de secret de 2FA e de
  código de recuperação, que andam sobre `Crypt` sob seus próprios
  purposes.
- [session.md](session.md) - o cookie de sessão, criptografado e
  assinado por `Crypt` via `CryptPurpose::Cookie`.
