# Desenvolvimento

O loop diário do Suprnova é um comando: `suprnova serve`. Ele executa
o backend Rust, o frontend Vite e um regenerador de tipos TypeScript
em um único processo, cada um monitorando os arquivos corretos. Este capítulo
aborda o servidor de desenvolvimento, como as partes de hot-reload se encaixam
e os comandos que você usará diariamente. Para a configuração inicial, consulte
[Instalação](installation.md); para o tour dos diretórios, consulte
[Estrutura de diretórios](structure.md).

## O servidor de desenvolvimento

A partir da raiz de um projeto com scaffold:

```bash
suprnova serve
```

A CLI imprime duas URLs e depois um fluxo contínuo de saída com prefixo
de cada processo filho:

```
Backend  http://127.0.0.1:8765
Frontend http://127.0.0.1:5765

[backend]  Compiling links v0.1.0
[backend]  Finished `dev` profile [unoptimized + debuginfo] target(s) in 4.21s
[backend]  Running `target/debug/links`
[frontend] VITE v6.0.1  ready in 312 ms
[frontend]   ➜  Local:   http://localhost:5765/
[types]    Watching for Rust file changes to regenerate types
```

Você acessa a URL do backend (`127.0.0.1:8765`). O Vite fornece seu JS/CSS
através da integração de desenvolvimento do Inertia - você não acessa `:5765` diretamente.
Pressione `Ctrl+C` uma vez e a CLI encerra ambos os processos filhos de forma limpa.

### Flags

| Flag | Padrão | O que faz |
|---|---|---|
| `-p`, `--port <N>` | `8765` | Porta do backend |
| `--frontend-port <N>` | `5765` | Porta do Vite |
| `--backend-only` | desligado | Pula o processo filho Vite (trabalho apenas de API) |
| `--frontend-only` | desligado | Pula o processo filho backend (trabalho de componentes contra um backend em execução em outro lugar) |
| `--skip-types` | desligado | Pula o gerador de tipos TypeScript e seu monitor |

As mesmas portas podem ser definidas em `.env` via `SERVER_PORT` e `VITE_PORT`.
Uma flag na linha de comando vence sobre `.env`.

### O que ele verifica previamente

Antes de gerar qualquer coisa, `suprnova serve`:

1. **Verifica se você está em um projeto.** Aborta com um erro claro se não houver
   `Cargo.toml` (ou se não houver `frontend/` ao executar o frontend).
2. **Gera tipos TypeScript uma vez.** Procura em `src/` por
   `#[derive(InertiaProps)]` e escreve
   `frontend/src/types/inertia-props.ts`. Pulado por `--skip-types` ou
   `--frontend-only`.
3. **Instala `cargo-watch` se estiver faltando.** A primeira execução em uma nova máquina
   executa `cargo install cargo-watch` para você, depois continua.
4. **Executa `npm install` se `frontend/node_modules` estiver faltando.** Nenhuma
   etapa manual de instalação em um clone recente.

## Hot reload

Três monitores rodam concorrentemente dentro de `suprnova serve`:

- **`cargo watch -x 'run --bin <pkg>'`** controla o backend. Qualquer mudança em `.rs`
  no projeto dispara uma recompilação e um reinício em processo.
  Erros de compilação são impressos no fluxo `[backend]` e o
  binário anterior permanece ativo até a próxima compilação bem-sucedida.
- **Vite** controla o frontend. Edições de componente, estilo e ativo
  são hot-module-replace na aba do navegador aberta sem uma recarga completa.
- **Monitor de tipo baseado em `notify`** executa novamente o scanner InertiaProps
  sempre que um arquivo `.rs` muda. Ele rebate em 500ms, então uma sequência de
  salvamentos regenera `inertia-props.ts` uma vez. A saída aparece sob o
  prefixo `[types]`.

Esse terceiro é a parte em que você não precisa pensar: renomeie um campo
em um struct `#[derive(InertiaProps)]` e a interface TypeScript correspondente
segue no próximo salvamento. A página Svelte/React/Vue coleta
o novo tipo imediatamente. Nenhuma invocação de `suprnova generate-types`
necessária durante o desenvolvimento normal.

### Por que Suprnova diverge

A maioria das pilhas web Rust torna o hot reload seu problema - escolha seu próprio
monitor de arquivo, escreva seu próprio wrapper de reinício, execute o Vite em um
terminal separado. A maioria das pilhas Laravel tornam os tipos TypeScript seu problema -
declare-os em dois lugares (PHP e TS) e mantenha-os em sincronização.
`suprnova serve` executa ambos os monitores, mais o gerador de tipo que
mantém seus tipos de frontend honestos, como um único processo supervisionado. O
runtime Tokio torna "muitas coisas à vez" barato o suficiente para que um loop de desenvolvimento
possa gastar livremente.

## Comandos dia a dia

Os poucos que você executará por hora:

```bash
suprnova serve                    # inicia o dev (backend + Vite + monitor de tipo)
suprnova make:controller orders   # scaffold de um controlador
suprnova make:migration add_idx   # scaffold de uma migração
suprnova db:sync                  # executa migrações, regenera entidades SeaORM
suprnova migrate:status           # vê o que foi aplicado
suprnova migrate:fresh            # descarta tabelas + re-executa do zero
suprnova key:generate --show      # rotaciona APP_KEY
cargo run --bin console <cmd>     # qualquer handler de console anotado com `#[command]`
cargo test                        # executa a suíte de testes
```

`db:sync` é o atalho de desenvolvimento para "migração + regeneração de entidade em uma
etapa." Na produção, você usa `suprnova migrate` simples porque você
não quer que a regeneração aconteça em uma máquina de release. A superfície geradora completa
está em [Geradores de código](cli-generators.md) e os
verbos de migração estão em [Migrações](migrations.md).

## Depuração

### Logs

Suprnova usa `tracing` de ponta a ponta. Filtre o que é impresso com
`LOG_LEVEL` (a mesma sintaxe do `EnvFilter` de `tracing-subscriber`):

```bash
# Saída de framework detalhada
LOG_LEVEL=debug suprnova serve

# Silencia hyper mas detalhado seu crate
LOG_LEVEL=info,my_app=debug,hyper=warn suprnova serve
```

O formato de saída é controlado por `LOG_FORMAT` (`pretty` para legível por humanos,
`json` para legível por máquina). O padrão de desenvolvimento é `pretty`. Consulte
[Observabilidade](observability.md) para a superfície de logs completa.

### Consultas SQL

Ativa logs por consulta com uma variável de ambiente:

```env
DB_LOGGING=true
```

Isso roteia cada consulta SeaORM através de `tracing` em `info` para que você possa
ver exatamente o que está sendo executado. Deixe desligado em produção, a menos que você esteja
perseguindo uma consulta lenta específica - o volume fica ruidoso rapidamente.

### Rastreamentos de pilha

Rust padrão:

```bash
RUST_BACKTRACE=1 suprnova serve
```

Um panic em um handler é capturado e transformado em uma resposta 500
estruturada; o rastreamento de pilha cai em seus logs sem levar o servidor
para baixo. Consulte [Modelo de erros](error-model.md) para saber como esse contrato funciona.

## Testes no loop

```bash
cargo test                        # espaço de trabalho inteiro
cargo test -p my_app              # apenas seu crate de app
cargo test some_test_name         # filtro por nome
cargo test -- --nocapture         # mostra saída println!/tracing
```

A execução de teste é Cargo simples. Os auxiliares do lado do framework
(`#[suprnova_test]`, `TestDatabase`, `expect!`, fakes para Mail/Queue/
Storage/etc.) são documentados em [Testes](testing.md) e
[Testes de banco de dados](database-testing.md). Eles rodam sob o mesmo
`cargo test` que você já conhece.

## Trabalhando com o worker SSR

Se seu app usa renderização de servidor Inertia, você vai querer o worker
SSR junto com `suprnova serve` durante o desenvolvimento:

```bash
# Terminal 1
suprnova serve

# Terminal 2
suprnova ssr:start
```

`ssr:start` executa o worker SSR empacotado sob Node, Bun ou Deno
(`--runtime`). `ssr:check` verifica se um worker em execução é alcançável.
Ambos são documentados sob o capítulo frontend - consulte
[Frontend](frontend.md).

## Quando algo parece errado

Uma lista curta de triagem para os soluços mais comuns do loop de desenvolvimento:

- **Porta já em uso.** Outro `suprnova serve` ainda está ativo, ou um
  backend anterior travou. `lsof -i :8765` para encontrá-lo, ou simplesmente passe
  `--port 8001`.
- **`cargo-watch` continua recompilando.** Algum editor está reescrevendo arquivos
  ao salvar (formatadores, linters com autofix). Desabilite o formato ao salvar
  para o projeto, ou escopo seu monitor com padrões de `CARGO_WATCH_IGNORE`.
- **Tipos TypeScript não atualizando.** Ou `--skip-types` foi passado,
  ou o monitor tropeçou em um erro de parse `.rs`. Veja as
  linhas `[types]` - imprime um aviso e continua em vez de
  falhar todo o serve.
- **Erros do Vite mas o backend está bem.** Execute `npm install` em
  `frontend/` uma vez (a CLI faz isso no primeiro serve, mas se você
  remover `node_modules` não refará até esse diretório estar
  faltando novamente em um início recente).

Qualquer outra coisa, o capítulo [Erros](errors.md) cobre padrões de triagem mais profundos.

## Próximos passos

- [Instalação](installation.md) - configuração inicial da CLI e de um
  projeto
- [Início rápido](quickstart.md) - construa um pequeno app de ponta a ponta
- [Estrutura de diretórios](structure.md) - o que cada diretório contém
- [Geradores de código](cli-generators.md) - cada comando `make:*`
- [Testes](testing.md) - `#[suprnova_test]`, fakes e o banco de dados
  de testes
