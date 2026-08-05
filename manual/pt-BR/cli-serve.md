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
   existir (a menos que `--frontend-only`) e um diretório `frontend/`
   deve existir (a menos que `--backend-only`).
4. Regenera os tipos TypeScript a partir de qualquer struct
   `#[derive(InertiaProps)]` que encontrar em `src/`, escrevendo-os em
   `frontend/src/types/inertia-props.ts`.
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
   existir. Pulado sob `--backend-only`.
7. Spawna `cargo watch -x 'run --bin <package-name>'` para o backend.
   O `cargo-watch` executa o binário de novo sempre que um arquivo
   `.rs` muda.
8. Spawna `npm run dev` em `frontend/` para o Vite, o que te dá HMR
   para componentes Svelte/React/Vue e classes Tailwind.
9. Inicia um monitor de arquivos em `src/` que executa o gerador de
   tipos de novo sempre que um arquivo `.rs` muda, uma vez que a
   sequência de salvamentos ficou quieta por 500 ms. O debounce é
   trailing-edge, então uma sequência - `cargo fmt`, format-on-save em
   vários arquivos, uma troca de branch - se funde em exatamente uma
   regeneração que executa *depois* da última escrita, em vez de uma
   que dispara no primeiro arquivo e perde o resto.
10. Encaminha o stdout/stderr dos dois filhos para seu terminal com os
    prefixos `[backend]` e `[frontend]`.

`Ctrl+C` sinaliza ao gerenciador para definir sua flag de shutdown,
matar os dois filhos, e sair. Se algum dos processos sair por conta
própria - geralmente por causa de um erro de compilação Rust grave
demais para o `cargo watch` recuperar, ou um conflito de porta - o
gerenciador trata isso como um sinal de shutdown e derruba o outro.

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

## Hot reload

**Backend.** `cargo watch -x 'run --bin <package>'` é o loop. Ele
recompila e reinicia o servidor a cada mudança de `.rs` no projeto.
Recompilações frias depois de tocar em um crate pesado podem levar
vários segundos; mudanças incrementais em um único arquivo geralmente
levam menos de um segundo.

**Frontend.** O HMR do Vite injeta mudanças de componente no lugar
sem um reload completo, preservando o estado do componente. As
classes Tailwind se atualizam em tempo real via o monitor do Tailwind
v4.

**Tipos TypeScript.** Sempre que um arquivo `.rs` muda, o monitor de
tipos executa o gerador de novo. Se novas structs
`#[derive(InertiaProps)]` aparecerem (ou as existentes mudarem de
forma), o `frontend/src/types/inertia-props.ts` regenerado dispara o
HMR do Vite para o componente que as importa.

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

### O backend sai silenciosamente logo depois de iniciar

Quando qualquer um dos processos filhos sai, o gerenciador encerra o
outro também. Se o backend morreu com um erro de compilação, as
linhas `[backend]` bem acima da mensagem "Servers stopped." vão
mostrar o `error[E…]` do rustc. Corrija o erro de compilação e execute
de novo.

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
