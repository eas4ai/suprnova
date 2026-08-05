# Guia de contribuição

O Suprnova é open-source sob a Licença MIT, e a contribuição mais
valiosa é um **bom relatório**. O projeto não aceita pull requests: o
framework é de autoria exclusiva dos mantenedores de ponta a ponta, e
toda mudança chega através dos mantenedores para que toda a superfície
mantenha uma forma única. Essa é uma postura deliberada e permanente -
não uma fase pré-1.0.

MIT significa que você nunca precisa de permissão para levar o código
adiante por conta própria: **faça fork livremente**. Um fork que cresce
em sua própria direção é um resultado saudável, não uma rivalidade.

O que isso significa na prática:

- **Relatórios de bug** - bem-vindos, via
  [issues do GitHub](https://github.com/entrepeneur4lyf/suprnova/issues).
- **Solicitações de recursos** - bem-vindas, via issues. Descreva o caso
  de uso, não a implementação; frequentemente já existe uma forma
  planejada (geralmente o equivalente do Laravel).
- **Bugs de documentação** - bem-vindos, via issues. Se um capítulo diz
  que uma API existe e você não consegue encontrá-la, isso é um bug de
  documentação - diga qual capítulo e o que você esperava.
- **Problemas de segurança** - em particular, por email (veja abaixo).
  Nunca como issues públicas.
- **Pull requests** - não são aceitos. PRs são fechados com uma
  referência a este capítulo; abra uma issue em vez disso para que a
  correção possa chegar ao upstream, ou faça fork e leve a mudança você
  mesmo.

## Enviando um relatório de bug que é corrigido rapidamente

O padrão-ouro é uma reprodução a partir de um scaffold novo:

```bash
suprnova new repro-app --frontend vue --no-interaction
# …a menor mudança que mostra o bug…
```

Inclua:

1. **O que você fez** - os comandos e o código, reduzidos ao mínimo
2. **O que você esperava** - uma frase
3. **O que aconteceu em vez disso** - a saída ou o erro reais, colados
   literalmente
4. **Versões** - a tag do framework (`suprnova --version`, ou o
   `tag =` no seu `Cargo.toml`) e sua versão do Rust
   (`rustc --version`)

Um teste que falha é ainda melhor do que prosa. Se você conseguir
expressar o bug como um teste contra o framework, cole-o na issue - ele
geralmente se tornará o teste de regressão com o qual a correção chega.

## Compilando a partir do código-fonte (para investigar um relatório)

Você não precisa disso para *abrir* uma issue, mas reproduzir contra o
workspace geralmente aprimora um relatório:

```bash
git clone https://github.com/entrepeneur4lyf/suprnova.git
cd suprnova
cargo check --workspace          # verifica os tipos de tudo
cargo test --workspace           # roda a suíte completa (~3400 testes)
```

O layout do workspace: `framework/` (o crate `suprnova`),
`suprnova-cli/` (o binário `suprnova`), `suprnova-macros/` (proc
macros), `app/` (app interno de dogfooding), `crates/` (adaptadores de
pagamentos e web-push), e `manual/` (este manual).

## O padrão exigido do código

Não são regras para contribuidores - mas conhecer o padrão ajuda você a
calibrar relatórios (um panic vindo de código de biblioteca, um teste
de modo de falha ausente, ou uma API que força `unwrap()` sempre vale a
pena reportar):

- **Somente implementações completas.** Sem TODOs, sem scaffolds
  parciais. Uma correção chega junto com o teste de regressão que a
  fixa.
- **Código de superfície pública retorna `Result`, não entra em
  panic.** Onde existir um nome infalível ao estilo Laravel, um irmão
  `try_*` vem junto com ele.
- **Nenhum `unsafe` fora da inicialização do ambiente.** O framework
  tem exatamente dois blocos `unsafe` em código que não é de teste,
  ambos em `config/env.rs::load_dotenv`, ambos envolvendo
  `std::env::set_var` / `remove_var` - que se tornaram `unsafe` na
  edition 2024 - e ambos carregando uma nota SAFETY para o invariante
  de thread única no momento do boot do qual dependem. Tudo o mais é
  somente para testes. Novo `unsafe` em qualquer outro lugar precisa de
  uma justificativa escrita na revisão, e `unsafe` em um driver,
  handler, ou expansão de macro não será aceito.
- **`cargo fmt` e clippy sob `-D warnings` são canônicos.**

Veja [Modelo de erros](error-model.md) para o contrato de erro
completo.

## Segurança

Reporte problemas de segurança em particular para
**shawn@eas4ai.com** (o mantenedor do projeto). Vamos confirmar
o recebimento em alguns dias, trabalhar na correção em um branch
privado, e coordenar a divulgação com você.

Não abra problemas de segurança como issues públicas do GitHub até que
uma correção tenha sido lançada.

### Avisos de dependências

O `cargo audit` roda no gate de release (`scripts/gate.sh --full`). Se
um aviso não tiver correção disponível e o código vulnerável não for
alcançável em um build padrão, ele pode ser adicionado a
`.cargo/audit.toml` - mas toda entrada precisa de três coisas, e
`scripts/check-audit.sh` falha o gate sem elas:

```toml
# OWNER: name <email>
# EXPIRES: YYYY-MM-DD
"RUSTSEC-XXXX-XXXX",
```

- um **proprietário**, para que a exceção pertença a alguém;
- uma **validade**, após a qual o gate se recusa a rodar até que a
  entrada seja renovada com um motivo declarado ou removida;
- um **argumento de alcançabilidade por escrito** - qual caminho a
  traz, e por que um build padrão não a vincula.

Alegações de alcançabilidade são verificadas, não apenas confiadas. Se
seu argumento for "isto está atrás de uma feature desativada por
padrão", adicione a asserção correspondente em
`scripts/check-feature-matrix.sh`, que resolve as árvores de
dependência reais e garante que o crate esteja ausente na árvore
padrão e presente na opcional. Uma exceção cuja justificativa nada
verifica silenciosamente deixa de ser verdadeira na primeira vez que
alguém adiciona uma dependência.

Um ignore é uma decisão de lançar um problema conhecido. Deveria se
ler como uma.

## Licença

MIT, com atribuição ao [projeto Kit](https://github.com/dayemsiddiqui/kit)
upstream do qual fizemos fork.
