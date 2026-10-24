# Hashing

O módulo `suprnova::hashing` é a superfície de hashing de senha do
framework, com três drivers de primeira classe - **bcrypt** (padrão,
compatível com o Laravel), **Argon2i** (memory-hard, resistente a
side-channel) e **Argon2id** (recomendação OWASP 2024). Use-o para
armazenar senhas de usuário, fazer hash de tokens verificadores de
remember-me, ou em qualquer lugar em que uma função unidirecional
seja a primitiva certa. A seleção de driver é feita por variável de
ambiente, e a facade é consciente do algoritmo de ponta a ponta
(`info`, `is_hashed`, `needs_rehash`, `verify`), então um hash bcrypt
armazenado ainda verifica depois que você mudar para
`HASH_DRIVER=argon2id`.

## Visão geral

```rust
use suprnova::hashing;

// Assíncrono (preferido dentro de handlers de solicitação do Tokio -
// executa o hash CPU-bound em spawn_blocking para que a worker thread
// continue livre):
let hashed = hashing::hash_async("my_password").await?;
let valid = hashing::verify_async("my_password", &hashed).await?;

// Síncrono (testes, ferramentas de CLI, contextos não assíncronos):
let hashed = hashing::hash("my_password")?;
let valid = hashing::verify("my_password", &hashed)?;
```

A facade de função livre lê o driver ativo a partir de `HASH_DRIVER`
(ou cai de volta para bcrypt). Para chamadas com driver explícito,
construa o tipo do driver diretamente e passe-o para `hash_with` /
`verify_with` / `needs_rehash_with`.

## Configuração

| Variável | Descrição | Padrão | Intervalo |
|----------|-------------|---------|-------|
| `HASH_DRIVER` | Algoritmo ativo | `bcrypt` | `bcrypt` \| `argon` \| `argon2i` \| `argon2id` |
| `HASH_ROUNDS` | Fator de custo do bcrypt | `12` | `4..=31` (somente bcrypt) |
| `HASH_MEMORY` | Custo de memória do Argon em KiB | `65536` (64 MiB) | `>= 8` (somente argon) |
| `HASH_TIME` | Iterações de tempo do Argon | `4` | `>= 1` (somente argon) |
| `HASH_THREADS` | Paralelismo / lanes do Argon | `1` | `>= 1` (somente argon) |
| `HASH_VERIFY` | Quando true, `verify()` rejeita hashes de outro algoritmo | `false` | `true` / `false` |

Uma configuração incorreta (valor inválido, parâmetro fora do
intervalo) surge como um `FrameworkError::param` na primeira chamada
a `hash` / `verify` / `needs_rehash` - não como um padrão silencioso.

### Exemplo de `.env` para argon2id

```env
HASH_DRIVER=argon2id
HASH_MEMORY=65536
HASH_TIME=4
HASH_THREADS=1
```

### Por que os padrões do Argon2 do Suprnova são mais fortes que os do Laravel

| Parâmetro | Padrão do Laravel | Padrão do Suprnova | Fonte |
|-------|-----------------|------------------|--------|
| Memória | 1 024 KiB (1 MiB) | 65 536 KiB (64 MiB) | OWASP 2024 |
| Tempo | 2 iterações | 4 iterações | OWASP 2024 |
| Threads | 2 | 1 | OWASP 2024 / alinhado ao libsodium |

Os padrões do Laravel assumem o modelo request-per-process do PHP -
um worker só pode gastar até certo ponto em cada hash de senha antes
de a caixa ficar cheia. O `spawn_blocking` do Tokio deixa o Suprnova
passar o hash para um thread pool bloqueante sem congelar o loop de
solicitações, então os números da OWASP 2024 são realistas em
hardware de produção real.

## Drivers

### Bcrypt (padrão)

```rust
use suprnova::hashing::{BcryptHasher, BcryptOptions, hash_with, verify_with};

let driver = BcryptHasher::new(BcryptOptions { rounds: 14 });
let hashed = hash_with(&driver, "my_password")?;
assert!(verify_with(&driver, "my_password", &hashed)?);
```

O bcrypt tem um **limite de bloco de 72 bytes** na entrada da senha -
a primitiva subjacente trunca silenciosamente entradas maiores, o que
significa que duas passphrases distintas que compartilham os
primeiros 72 bytes geram o mesmo hash. O Suprnova rejeita de antemão
(o caminho bcrypt do framework retorna erro em `hash()` e retorna
`Ok(false)` em `verify()` para senhas grandes demais, mantendo
uniforme a resposta "credenciais inválidas" do fluxo de auth). O
Argon2 não tem esse teto.

O limite do bcrypt é exposto como
`suprnova::hashing::MAX_BCRYPT_PASSWORD_BYTES` (71 - o limite
utilizável depois do terminador nulo do bcrypt).

### Argon2id (recomendação OWASP 2024)

```rust
use suprnova::hashing::{Argon2idHasher, Argon2Options, hash_with, verify_with};

let driver = Argon2idHasher::new(Argon2Options {
    memory: 65_536,  // 64 MiB
    time: 4,
    threads: 1,
})?;

let hashed = hash_with(&driver, "my_password")?;
assert!(verify_with(&driver, "my_password", &hashed)?);

// O Argon2 aceita passphrases de comprimento arbitrário - o limite de
// 72 bytes do bcrypt não se aplica.
let long = "x".repeat(500);
let h = hash_with(&driver, &long)?;
assert!(verify_with(&driver, &long, &h)?);
```

### Argon2i

Mesmo formato do Argon2id; `Argon2iHasher::new(opts)`. Use Argon2id
para projetos novos - o Argon2i é suportado por paridade, mas o
Argon2id é a recomendação moderna.

## Bcrypt com um custo explícito (`hash_with_cost`)

`hash_with_cost(password, cost)` e
`hash_with_cost_async(password, cost)` cunham um hash bcrypt com um
fator de custo fornecido pelo chamador, independentemente de
`HASH_DRIVER`. Use-os quando uma política ou uma configuração por
tenant faz um custo fluir até o call site em vez de até o env do
processo - por exemplo, uma classe de conta de alta segurança que usa
custo 14 enquanto o resto da app roda no padrão 12.

```rust
use suprnova::hashing::{hash_with_cost, hash_with_cost_async};

// Síncrono - testes, ferramentas de CLI.
let h = hash_with_cost("my_password", 14)?;

// Assíncrono - dentro de handlers de solicitação do Tokio.
let h = hash_with_cost_async("my_password", 14).await?;
```

Ambos os pontos de entrada rejeitam `cost` fora de
`MIN_BCRYPT_COST..=MAX_BCRYPT_COST` (`4..=31`) com
`FrameworkError::param`, espelhando a validação de `HASH_ROUNDS` do
lado do env:

```rust
use suprnova::hashing::{hash_with_cost, MIN_BCRYPT_COST, MAX_BCRYPT_COST};

assert!(hash_with_cost("pw", MIN_BCRYPT_COST - 1).is_err()); // < 4
assert!(hash_with_cost("pw", MAX_BCRYPT_COST + 1).is_err()); // > 31
```

A verificação de limites importa porque cada incremento de custo
dobra o tempo de CPU. No custo 31, um único hash bcrypt leva horas em
hardware comum - verificar os limites dentro do framework impede que
um erro de digitação em uma política/config prenda acidentalmente uma
worker thread pelo resto do dia. A variante assíncrona passa por
`spawn_blocking`, então mesmo um custo legitimamente alto não congela
o loop de solicitações.

## needs_rehash consciente do algoritmo

`needs_rehash` retorna `true` quando o hash armazenado deveria ser
re-hasheado sob o driver ativo. Isso cobre três casos:

1. **Incompatibilidade de algoritmo** - hash bcrypt armazenado
   enquanto `HASH_DRIVER=argon2id` (ou vice-versa). Dispara uma
   rotação no próximo `verify` bem-sucedido.
2. **Fraqueza de parâmetro** - custo do bcrypt abaixo de
   `HASH_ROUNDS`, ou `m`/`t`/`p` do argon abaixo de
   `HASH_MEMORY`/`HASH_TIME`/`HASH_THREADS`.
3. **Variantes legadas do bcrypt** - `$2a$`, `$2x$`, `$2y$`
   rotacionam para o `$2b$` canônico mesmo no custo configurado.

```rust
if hashing::needs_rehash(&stored_hash) {
    let fresh = hashing::hash_async("plaintext_at_login").await?;
    // Persista `fresh`. É o padrão Laravel de "rehash no login
    // bem-sucedido"; funciona entre algoritmos.
}
```

Entrada malformada retorna `true` - o chamador naturalmente
rotaciona qualquer coisa que não consiga parsear.

## Inspeção de hash (`info` + `is_hashed`)

```rust
use suprnova::hashing::{info, is_hashed};

let h = hashing::hash_async("my_password").await?;
let i = info(&h);
println!("algo: {}", i.algo.as_str());
println!("bcrypt cost: {:?}", i.rounds);
println!("argon memory KiB: {:?}", i.memory);

// True para qualquer hash de algoritmo reconhecido; false para texto puro / lixo.
assert!(is_hashed(&h));
assert!(!is_hashed("plaintext"));
```

`info().algo` é um de: `Bcrypt`, `Argon2i`, `Argon2id`, `Argon2d`
(reconhecido, mas nunca cunhado), `Unknown`.

`is_hashed` é o que o cast eloquent `AsHashed` usa para pular o
re-hash de uma coluna já hasheada - funciona nos três drivers, então
trocar `HASH_DRIVER` no meio do projeto não causa um loop de
hash-de-hash no próximo save.

## Gate de verificação entre algoritmos (`HASH_VERIFY`)

Por padrão, `verify()` verifica a senha contra o hash
independentemente de qual algoritmo produziu o hash - é isso que
permite que hashes bcrypt legados ainda verifiquem depois que você
mudar para `HASH_DRIVER=argon2id` (para que você possa rotacioná-los
no login). Defina `HASH_VERIFY=true` quando todo usuário já tiver
sido rotacionado, para impor estritamente o algoritmo ativo:

```env
HASH_VERIFY=true
```

Com o gate ativado, `verify()` retorna `Ok(false)` para qualquer
hash cujo algoritmo seja diferente do driver ativo - o mesmo formato
da `RuntimeException` do Laravel, mas o Suprnova retorna false em vez
de lançar, porque o chamador do fluxo de auth espera um
`Result<bool>` de todo jeito.

## Assíncrono vs síncrono

Tanto o bcrypt em custo 12 (~250 ms) quanto o Argon2id em memory=64
MiB (~80 ms) são intencionalmente CPU-bound - esse é todo o objetivo
do hashing lento. Chamar o `hash` / `verify` síncrono diretamente de
um handler de solicitação do Tokio bloqueia a worker thread pela
duração inteira do hash, causando inanição nas outras solicitações do
mesmo worker.

Use os irmãos `*_async` dentro de handlers `async fn`. Eles envolvem
a chamada CPU-bound em `tokio::task::spawn_blocking` para que o
worker permaneça livre para outras solicitações:

```rust
// BOM - dentro de um handler assíncrono
let hashed = hashing::hash_async(&form.password).await?;

// MAU - bloqueia o worker por ~250 ms
let hashed = hashing::hash(&form.password)?;
```

As variantes síncronas são para testes, ferramentas de CLI, e outros
contextos não assíncronos onde bloquear não é um problema.

## Integração com o Eloquent: cast `AsHashed`

O cast eloquent `#[cast(AsHashed)]` faz hash de um campo em texto
puro na escrita, usando o driver ativo, e é **idempotente entre
todos os drivers** - salvar um model cuja coluna `password` já
contém um hash reconhecido (bcrypt ou argon) deixa o valor passar
inalterado. Sem essa salvaguarda de idempotência,
`User::find(id).await?.save().await?` faria hash do hash já
existente a cada save, quebrando a autenticação.

```rust
use suprnova::eloquent::casts::AsHashed;

#[suprnova::model]
struct User {
    #[cast(AsHashed)]
    pub password: String,
    // ...
}
```

A verificação de idempotência usa `hashing::is_hashed`, então trocar
`HASH_DRIVER` no meio do projeto é seguro - tanto os hashes bcrypt
legados quanto os hashes argon2id novos são reconhecidos e pulados no
re-save.

## Uso com `Auth::attempt`

`Auth::attempt(&credentials)` chama
`UserProvider::validate_credentials`, que por sua vez chama
`hashing::verify_async` contra o hash armazenado do usuário. O verify
decide com base no algoritmo do hash *armazenado*, não no driver
configurado - então depois que você mudar para
`HASH_DRIVER=argon2id`, todo hash bcrypt existente ainda verifica, e
`needs_rehash` retorna `true`, de modo que o padrão de
rotação-no-login carrega a base de usuários para o novo algoritmo um
login por vez.

## Sobrescrevendo o driver em testes

`set_default_driver(Box<dyn Hasher>)` instala um driver
programaticamente para testes e ferramentas de CLI embarcadas que
constroem o driver sem passar por `HASH_DRIVER`. É de execução única -
a primeira chamada vence, e uma segunda chamada retorna
`FrameworkError::internal` em vez de trocar o driver no meio do
processo. Use-o na inicialização da suíte, antes que qualquer caminho
de código resolva o padrão.

## Próximos passos

- [Autenticação](authentication.md) - `Auth::attempt`, a trait de
  provedor de usuário, e como o hashing se integra com o login
- [Fluxos de autenticação](auth-flows.md) - `PasswordReset::complete`
  rotaciona o hash de senha armazenado através do driver ativo;
  tokens de remember-me são hasheados antes do armazenamento via
  `hash_async`
- [Eloquent](eloquent.md) - referência de `#[cast(AsHashed)]` e a
  superfície mais ampla de casts
- [Criptografia](encryption.md) - criptografia autenticada
  bidirecional para dados em repouso; o complemento do hashing
  unidirecional
- [Modelo de erros](error-model.md) - como é o
  `FrameworkError::param` quando um valor de configuração de hashing
  é rejeitado
