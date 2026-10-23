# suprnova serve

`suprnova serve` executa seu backend e o servidor de dev do Vite
juntos, com hot reload dos dois lados, além da regeneração automática
de tipos TypeScript sempre que você toca em uma struct
`#[derive(InertiaProps)]`. É o único comando que você mantém aberto em
um terminal enquanto está construindo.

```bash
suprnova serve
```

Os dois processos transmitem seu stdout para o mesmo terminal com
prefixos coloridos `[backend]` e `[frontend]` para que você saiba
quem disse o quê. `Ctrl+C` encerra os dois de forma limpa.

## Uso

```bash
suprnova serve [OPTIONS]
```

| Opção | Padrão | Descrição |
|---|---|---|
| `-p, --port <PORT>` | `8765` (CLI) / `$SERVER_PORT` (env) | Porta HTTP do backend |
| `--frontend-port <PORT>` | `5765` (CLI) / `$VITE_PORT` (env) | Porta do servidor de dev do Vite |
| `--backend-only` | `false` | Pula o servidor de dev do Vite |
| `--frontend-only` | `false` | Pula o backend, só executa o Vite |
| `--skip-types` | `false` | Não regenera os tipos TypeScript em mudanças no Rust |
| `--no-restart` | `false` | Não reinicia um processo de dev que sofreu crash - encerra toda a sessão (o comportamento antigo) |
| `--restart-tries <N>` | `5` | Desiste de tentar um processo depois deste número de crashes consecutivos. Ignorado com `--no-restart`, que já encerra a sessão no primeiro crash. |
| `--timestamps` | `false` | Prefixa cada linha de saída com um horário `HH:MM:SS` |
| `--json` | `false` | Emite um objeto JSON por linha (NDJSON) em stdout em vez de texto com prefixo - veja [Saída JSON](#saída-json). Combinar com `--timestamps` não é erro; `--timestamps` não tem efeito extra, pois todo evento já traz seu próprio timestamp. |

As flags da CLI têm precedência sobre as variáveis de ambiente, que
têm precedência sobre os padrões embutidos. Um `.env` com scaffold
vem com `SERVER_PORT=8765` e `VITE_PORT=5765`; você verá esses valores
sendo usados a menos que você sobrescreva com `--port`.

## Exemplos

### Padrão - os dois servidores

```bash
suprnova serve
```

Saída:

```
Backend  http://127.0.0.1:8765
Frontend http://127.0.0.1:5765
[backend] Compiling my-app v0.1.0 ...
[frontend] VITE v6.3.0  ready in 312 ms
```

Acesse `http://127.0.0.1:8765` no seu navegador. O backend serve o
shell HTML do Inertia e faz proxy das requisições de assets para o
Vite, então você não precisa visitar a URL do Vite diretamente.

### Portas customizadas

```bash
suprnova serve --port 3000 --frontend-port 3001
```

Ou defina-as em `.env` e execute sem flags:

```env
SERVER_PORT=3000
VITE_PORT=3001
```

### Somente backend

```bash
suprnova serve --backend-only
```

Bom para trabalhar em um projeto somente API, ou quando seu frontend
já está em execução em outro terminal (ou outra máquina, ou um
preview implantado).

### Somente frontend

```bash
suprnova serve --frontend-only
```

Bom para trabalhar na UI sem pagar o custo de uma recompilação Rust a
cada salvamento, ou quando o backend está em execução em outro shell
(ou no Docker).

### Projeto somente API

Um projeto com scaffold feito por `suprnova new --api` não tem
diretório `frontend/`. Execute `serve` exatamente como você faria em
qualquer outro lugar:

```bash
suprnova serve
```

O `serve` não vê nenhum `frontend/package.json`, pula o painel do Vite
e a geração de TypeScript que o alimenta, e executa o backend.
`--frontend-only` continua sendo um erro num projeto desses: ele pede
justamente o painel que não existe.

### Pular a geração de tipos

```bash
suprnova serve --skip-types
```

Desativa o monitor de regeneração TypeScript. Use isso quando você
está gerenciando `frontend/src/types/inertia-props.ts` à mão, ou
quando você está trabalhando longe de qualquer código Inertia e quer
uma saída mais silenciosa.

## O que ele realmente faz

Quando você executa `suprnova serve`, a CLI:

1. Carrega `.env` a partir do diretório atual.
2. Resolve as portas de backend e frontend (flag da CLI → variável de
   ambiente → padrão).
3. Verifica se você está em um projeto Suprnova - `Cargo.toml` deve
   existir (a menos que `--frontend-only`), e `--frontend-only` precisa
   de um diretório `frontend/` com um `package.json`. Um projeto que não
   tem isso é servido em modo somente backend, em vez de ser rejeitado.
4. Regenera os tipos TypeScript a partir de qualquer struct
   `#[derive(InertiaProps)]` que encontrar em `src/`, escrevendo-os em
   `frontend/src/types/inertia-props.ts`.
   Pulado quando o projeto não tem frontend.
5. Instala `cargo-watch` via `cargo install --locked --version "^8.5"
   cargo-watch` se ainda não estiver no PATH (uma única vez, com um
   aviso "Installing..."). Pulado sob `--frontend-only`.
   A versão é limitada porque o `serve` conduz o `cargo watch -x`,
   cujo significado não é garantido entre um bump major; `--locked`
   compila a árvore de dependências que o cargo-watch publicou em vez
   de resolvê-la de novo no momento da instalação. Um comando que
   instala software como efeito colateral de iniciar um servidor de
   dev não deveria também estar escolhendo versões por você.
6. Executa `npm install` em `frontend/` se `node_modules` ainda não
   existir. Pulado sob `--backend-only` e quando o projeto não tem
   frontend.
7. Spawna `cargo watch` para o backend, delimitado com `-w` aos
   caminhos a partir dos quais o servidor é realmente construído:
   `src/`, `cmd/`, `Cargo.toml`, `Cargo.lock`, `.env` e `lang/`. É em
   `cmd/` que o scaffold full-stack coloca o `main.rs` do binário do
   servidor; o scaffold `--api` o coloca em `src/` e não tem `cmd/`.
   Cada caminho só é passado quando existe, porque o cargo-watch se
   recusa a iniciar com um caminho `-w` que não exista - um projeto que
   ainda não foi construído não tem `Cargo.lock`, e ele entra no
   `serve` seguinte.

   O `--no-vcs-ignores` acompanha esses caminhos. O cargo-watch aplica
   o seu `.gitignore` às raízes `-w` nomeadas explicitamente, e não só
   à varredura do próprio projeto, e o scaffold coloca `.env` no
   gitignore - então, sem essa flag, `-w .env` não monitora
   absolutamente nada. Ela não consegue alargar o que reinicia o
   backend, porque o `-w` já estreitou isso aos seis caminhos acima, e
   as únicas coisas no gitignore dentro deles são `.env` e (no `--api`)
   `Cargo.lock`, ambos monitorados de propósito. `target/`,
   `node_modules` e o resto ficam fora de toda raiz monitorada de
   qualquer forma.

   Em um projeto full-stack com scaffold, a invocação completa é
   `cargo watch --no-vcs-ignores -w src -w cmd -w Cargo.toml -w Cargo.lock
   -w .env -w lang -x 'run --bin <package-name>'`. Edições no frontend e
   os `frontend/src/types/*.ts` gerados ficam fora desse escopo, então
   nunca reiniciam o backend.
8. Spawna `npm run dev` em `frontend/` para o Vite, o que oferece HMR
   para componentes Svelte/React/Vue e classes Tailwind. Pulado sob
   `--backend-only` e quando o projeto não tem frontend.
9. Inicia cada processo extra declarado no `Suprnova.toml` do projeto
   (veja [Processos de dev extras](#processos-de-dev-extras) abaixo), cada
   um com seu próprio prefixo `[name]` - workers de fila, tailers de logs,
   qualquer coisa que você teria de manter em outro terminal.
10. Inicia um monitor de arquivos em `src/` que executa o gerador de
    tipos de novo sempre que um arquivo `.rs` muda, uma vez que a
    sequência de salvamentos ficou quieta por 500 ms. Só contam
    mudanças de verdade - uma criação, uma escrita ou uma exclusão.
    Leituras não contam, o que importa porque o gerador lê todo arquivo
    `.rs` dentro da árvore que ele monitora a cada execução.
    Pulado quando o projeto não tem frontend, igual à geração de tipos
    da inicialização no passo 4. O debounce é de borda de descida, então uma
    sequência - `cargo fmt`, format-on-save em vários arquivos, uma
    troca de branch - se funde em exatamente uma regeneração que executa
    *depois* da última escrita, em vez de uma que dispara no primeiro
    arquivo e perde o resto.
    Uma regeneração só escreve o arquivo quando o TypeScript emitido
    difere do que já está lá, e o monitor reporta apenas o que escreveu:
    uma edição que não muda nenhuma forma de prop não imprime nada e não
    emite nenhum evento `types_regenerated`. Silêncio depois de um
    salvamento significa que sua edição não mudou os tipos gerados.
11. Encaminha stdout/stderr de cada filho para seu terminal com um prefixo
    `[name]` (`[backend]`, `[frontend]` ou o nome configurado do processo),
    opcionalmente com timestamp via `--timestamps` - ou, com `--json`, como
    eventos NDJSON (veja [Saída JSON](#saída-json) abaixo).

`Ctrl+C` sinaliza ao gerenciador para definir sua flag de shutdown, matar
todos os filhos e sair. Se um filho sair por conta própria - um erro de
compilação Rust grave demais para o `cargo watch` recuperar, um processo
Vite que sofreu crash, um processo do `Suprnova.toml` que falhou -, ele é
reiniciado após um backoff curto (200 ms, dobrando a cada crash consecutivo,
limitado a 5 s; um processo que fica ativo por 30 s reinicia a contagem),
em vez de derrubar a sessão. Passe `--no-restart` para recuperar o
comportamento antigo: a saída de qualquer filho encerra toda a sessão
imediatamente.

Um processo que continua sofrendo crash não tenta para sempre:
`--restart-tries` (padrão `5`) limita quantos crashes consecutivos o
`serve` tenta novamente antes de desistir daquele processo - 30 s de
execução contínua zeram a contagem, assim como o atraso de backoff. Ao
desistir, imprime uma mensagem acionável e para de tentar *somente* aquele
processo; os demais (e a própria sessão) continuam executando, de acordo
com o padrão `concurrently --restart-tries=5` do Laravel. Veja [Solução de
problemas](#um-processo-continua-em-crash-loop).

### Por que Suprnova diverge

Usuários de Laravel costumam executar `php artisan serve` para o
backend e `npm run dev` em outro terminal, e a maioria das equipes
contorna a divisão em dois terminais com um `Procfile` e
`foreman`/`overmind`. O Suprnova envia esse multiplexador como um
comando de CLI de primeira classe. Você ganha um terminal, um
`Ctrl+C`, bootstrap automático de toolchain (`cargo-watch`, `npm
install`), e uma ponte Inertia tipada que regenera
`frontend/src/types/inertia-props.ts` na hora para que seus
componentes Svelte/React/Vue sempre vejam a forma de prop atual sem
sincronia manual de tipos.

O comando `dev` do Laravel também oferece os modos `--tabs` e `--stream`,
cada um renderizando a saída por uma pequena TUI Node
(`@laravel/multiplex`). O Suprnova não traz a TUI: saída em terminal único
com prefixos é a norma no ecossistema de ferramentas de dev Rust
(`cargo watch`, `bacon`, `just`), e um registro de processos com prefixos
coloridos já fornece o sinal de "qual processo disse isso" que uma TUI
oferece. O job subjacente de `--stream` - um stream de eventos em tempo real
e scriptável - é enviado por `--json` (veja [Saída JSON](#saída-json));
o TUI multipainel de `--tabs` é um não deliberado, não uma lacuna - um
segundo modelo de interação e uma segunda biblioteca para manter entre
terminais para um problema que esta página já resolve. Veja a linha
correspondente em [Paridade](parity.md#what-we-won-t-ship-and-why).

## Hot reload

**Backend.** O `cargo watch` é o loop, delimitado aos caminhos a partir
dos quais o servidor é construído. Ele recompila e reinicia a cada
mudança dentro de `src/` ou `cmd/`, em `Cargo.toml`, `Cargo.lock` ou
`.env`, ou dentro de `lang/` - o `.env` é lido uma única vez pelo
`Config::init` no boot e os catálogos Fluent uma única vez no bootstrap,
então uma mudança em qualquer um dos dois só passa a valer em um
restart. O `.env` é monitorado graças ao `--no-vcs-ignores`, sem o qual
o seu `.gitignore` o esconderia do monitor. Salvar um componente,
ou regenerar `frontend/src/types/inertia-props.ts`, fica fora desse
escopo e deixa o backend rodando. Recompilações frias depois de tocar em
um crate pesado podem levar vários segundos; mudanças incrementais em um
único arquivo geralmente levam menos de um segundo.

**Frontend.** O HMR do Vite injeta mudanças de componente no lugar
sem um reload completo, preservando o estado do componente. As
classes Tailwind se atualizam em tempo real via o monitor do Tailwind
v4.

**Tipos TypeScript.** Sempre que um arquivo `.rs` muda, o monitor de
tipos executa o gerador de novo. Se novas structs
`#[derive(InertiaProps)]` aparecerem (ou as existentes mudarem de
forma), o `frontend/src/types/inertia-props.ts` regenerado dispara o
HMR do Vite para o componente que as importa. Quando o TypeScript
emitido é byte a byte idêntico ao que já está no disco, o arquivo fica
intacto e o monitor não diz nada, então uma regeneração que não mudou
nada não é uma mudança a que nada downstream precise reagir - nem o
Vite, nem o monitor do backend, nem o que quer que esteja lendo
`--json`.

## Processos de dev extras

`suprnova serve` sempre executa o backend e o Vite, mas a maioria dos projetos
tem mais de duas coisas para manter em execução - um worker de fila, um
tailer de logs, um mail-catcher. Declare-os em um `Suprnova.toml` na raiz do
projeto, e `serve` os inicia, prefixa e reinicia automaticamente junto com
backend e frontend:

```toml
[[serve.process]]
name = "queue"
command = "cargo"
args = ["run", "--bin", "console", "--", "queue:work"]
color = "yellow"

[[serve.process]]
name = "logs"
command = "tail"
args = ["-f", "storage/logs/app.log"]
```

Cada entrada precisa de `name` e `command`; `args` assume nenhum por padrão,
`color` assume uma de green/yellow/blue/white atribuída na ordem de declaração
(ou escolha uma das oito cores nomeadas de `console` - black, red, green,
yellow, blue, magenta, cyan, white). Os nomes devem ser únicos.
`Suprnova.toml` é totalmente opcional; um projeto sem ele executa exatamente
como antes.

### Por que Suprnova diverge

O Laravel registra processos `dev` extras a partir do PHP -
`DevCommands::register($command, $name)`, normalmente em `boot()` de um
service provider - porque `php artisan dev` executa um multiplexador de
dentro do mesmo processo que já inicializou a aplicação. `suprnova serve` é
um binário separado da sua app; ele nunca vincula nem executa seu código Rust
e só chama `cargo watch` e `npm`. Não há boot da aplicação ao qual se
conectar, então o registro precisa ser dado que a CLI lê, e não uma chamada
feita pelo seu código - daí `Suprnova.toml` em vez de uma API
`DevProcesses::register()`.

## Saída JSON

Passe `--json` e `suprnova serve` escreve um objeto JSON por linha (NDJSON)
em stdout, em vez de texto colorido com prefixo `[name]` - nada mais vai
para stdout enquanto ele está ativo, então você pode canalizar diretamente
para `jq` ou outro consumidor JSON orientado a linhas. Cada linha tem um
campo `type`:

| `type` | Campos | Significado |
|---|---|---|
| `started` | `ts`, `name`, `pid` | Um processo (backend, frontend ou entrada de `Suprnova.toml`) foi iniciado pela primeira vez. |
| `output` | `ts`, `name`, `stream` (`"stdout"` ou `"stderr"`), `line` | Uma linha da saída de um filho, transportada como campo em vez de ser repassada em estado bruto. |
| `exited` | `ts`, `name`, `code` (nullable) | Um processo saiu. `code` é `null` se ele foi morto por um sinal em vez de retornar um status. |
| `restart_scheduled` | `ts`, `name`, `delay_ms` | Um processo que sofreu crash será reiniciado depois de `delay_ms` (veja o backoff acima). |
| `restart_succeeded` | `ts`, `name`, `pid` | Um reinício agendado teve sucesso; o processo executa novamente sob um novo PID. |
| `gave_up` | `ts`, `name`, `tries` | O processo sofreu `tries` crashes consecutivos (`--restart-tries`) e o `serve` parou de tentar. A sessão e todos os outros processos continuam. |
| `types_regenerated` | `ts`, `artifact` (`"inertia_props"` ou `"lang_keys"`), `count` | O monitor de arquivos reescreveu um artefato TypeScript depois de uma mudança `.rs`/`.ftl`. Dispara somente quando o arquivo gerado realmente mudou: uma edição em `.rs` que deixa o TypeScript emitido byte a byte idêntico não escreve nada e não emite nada, então um evento sempre significa que o arquivo em disco está diferente agora. `count` é o número de structs (ou de ids de mensagem) no arquivo reescrito, não o número dos que mudaram. |
| `shutdown` | `ts` | A sessão está sendo encerrada. Sempre a última linha. |

Por exemplo, um crash do Vite e seu reinício aparecem assim:

```json
{"type":"exited","ts":"2026-08-18T10:15:23.456-07:00","name":"frontend","code":1}
{"type":"restart_scheduled","ts":"2026-08-18T10:15:23.456-07:00","name":"frontend","delay_ms":200}
{"type":"restart_succeeded","ts":"2026-08-18T10:15:23.657-07:00","name":"frontend","pid":48391}
```

`--json` combina com `--timestamps` em vez de entrar em conflito:
combinar não é erro, mas `--timestamps` não tem efeito adicional, pois todo
evento já traz seu próprio campo `ts`.

Esta é uma saída legível por máquina que outras ferramentas analisam -
nomes de campos e valores de `type` não serão renomeados nem removidos sem
uma nota no changelog. Trate um `type` não reconhecido ou um campo extra
inesperado como algo a ignorar, não como erro, para que uma versão futura
possa estender o schema sem quebrar seu consumidor.

## Solução de problemas

### Porta já em uso

```text
[backend] Error: Address already in use (os error 98)
```

Encontre e mate o processo, ou escolha outra porta:

```bash
lsof -i :8765
kill -9 <pid>

# ou
suprnova serve --port 8081
```

### A instalação do `cargo-watch` falha

A CLI executa `cargo install cargo-watch` se ele ainda não estiver no
PATH. Se essa instalação falhar (sem rede, ambiente restrito),
instale-o manualmente uma vez:

```bash
cargo install cargo-watch
```

Depois disso, `suprnova serve` vai encontrá-lo e não vai tentar
instalar de novo.

### Dependências do frontend travadas

Se `npm install` falhar no meio do bootstrap, corrija a causa
(registry do npm alcançável, espaço em disco, lockfile em bom estado)
e execute-o manualmente:

```bash
cd frontend && npm install
```

Depois execute `suprnova serve` de novo. A CLI só executa `npm
install` automaticamente quando `node_modules` está faltando, então
uma instalação manual bem-sucedida faz com que ela pule essa etapa.

### Regeneração de tipos não capturando mudanças

O monitor faz polling a cada 2 segundos (usando `notify` com um
intervalo de poll - escolhido pela confiabilidade cross-platform em
vez das peculiaridades do inotify) e faz debounce da regeneração para
uma vez a cada 500 ms. Se uma mudança não estiver aparecendo:

- Confirme que o arquivo está sob `src/` (o monitor não recorre para
  dentro de `crates/`, `cmd/`, ou `migrations/`).
- Confirme que a struct realmente tem `#[derive(InertiaProps)]`.
- Reinicie `suprnova serve` e observe a mensagem de startup
  `Generated N type(s)` - se você ver `No InertiaProps structs found`,
  o scanner não encontrou nada para emitir.

### Um processo continua em crash loop

Se um filho - backend, frontend ou entrada `Suprnova.toml` - não consegue
iniciar (código ruim, binário ausente, conflito de porta), ele é reiniciado
no cronograma de backoff descrito acima em vez de parar. Procure as linhas
`[name]` imediatamente antes de cada aviso "respawning in …ms" para ver o
erro real (um `error[E…]` do rustc, ENOENT, o que o filho tiver impresso).
Corrija a causa; a próxima tentativa de reinício a capturará
automaticamente. Para parar as tentativas e ver a falha uma vez, execute
novamente com `--no-restart` - a sessão então será encerrada no primeiro
crash, como `suprnova serve` fazia antes disso existir.

Depois de `--restart-tries` (padrão `5`) crashes consecutivos, `serve` para
de tentar esse processo e imprime uma mensagem que o nomeia:

```text
gave up restarting `backend` after 5 attempts; fix the error and run `suprnova serve` again
```

Os outros processos, e a própria sessão, continuam executando - corrija a
causa e execute `suprnova serve` novamente para trazer de volta o processo
que desistiu; não é necessário reiniciar toda a sessão.

## Próximos passos

- [Instalação](installation.md) - coloque a CLI na sua máquina
- [Início rápido](quickstart.md) - um passo a passo completo do
  primeiro app
- [Estrutura de diretórios](structure.md) - o que `suprnova new` fez
  scaffold
- [Geradores](cli-generators.md) - `make:controller`, `make:action`,
  etc.
- [Console](console.md) - o binário `cargo run --bin console` por
  projeto
