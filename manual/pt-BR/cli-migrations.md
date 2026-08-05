# Migrações da CLI

A CLI de desenvolvedor `suprnova` faz shell para o binário da sua
aplicação para conduzir o executor de migrações do SeaORM, então o
mesmo conjunto de migrações executa se você o roda a partir de um
terminal de desenvolvedor, do CI, ou implicitamente no startup do
servidor. Use esses comandos para escrever arquivos de migração,
aplicá-los, revertê-los, e manter suas entidades SeaORM geradas em
sincronia com o esquema.

Para a API de criação de esquema (tipos de coluna, índices, chaves
estrangeiras, o `MigrationTrait` completo), veja
[Migrações](migrations.md). Para inserir dados de teste depois que o
esquema estiver em vigor, veja [Preenchimento de dados](seeding.md).

## make:migration

Gera um novo arquivo de migração em `src/migrations/` e o conecta ao
`Migrator` em `src/migrations/mod.rs`.

```bash
suprnova make:migration <name>
```

`<name>` é normalizado para snake_case. O gerador reconhece os
padrões de nomenclatura padrão e os usa para escolher o enum
`DeriveIden`:

- `create_<table>_table` - faz scaffold de um corpo `create_table`
- `add_<column>_to_<table>` - faz scaffold de um stub para
  `alter_table`
- `drop_<table>_table` - faz scaffold de um corpo `drop_table`
- qualquer outra coisa - usa o nome como o identificador da tabela

### Exemplos

```bash
suprnova make:migration create_users_table
suprnova make:migration add_email_to_users
suprnova make:migration drop_legacy_sessions_table
```

### Arquivo gerado

O arquivo é escrito em
`src/migrations/m{YYYYMMDD}_{HHMMSS}_<name>.rs` (por exemplo
`m20260530_142301_create_users_table.rs`) e adicionado ao vec
`Migrator::migrations()`.

```rust
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Users::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Users::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(Users::CreatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(Users::UpdatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Users::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
    CreatedAt,
    UpdatedAt,
}
```

Edite o arquivo gerado para declarar suas colunas, índices, e
constraints. Veja [Migrações](migrations.md) para a superfície
completa do construtor de esquema.

## migrate

Executa toda migração pendente em `src/migrations/`.

```bash
suprnova migrate
```

A CLI faz shell para `cargo run -- migrate` para que o executor
`Application` do seu app faça o trabalho - mesmo binário, mesmo
`Migrator`, mesma conexão de banco de dados que o `serve` usaria.

```
Running migrations...
Migrations completed successfully!
```

O caminho serve / web:run executa `migrate` automaticamente antes de
vincular o socket, a menos que você opte por não participar com
`--no-migrate` ou defina `SUPRNOVA_AUTO_MIGRATE_BEST_EFFORT=true` para
continuar apesar de uma falha. Um erro de migração durante o
auto-migrate sai com código não-zero antes do servidor inicializar;
veja `framework/src/app/mod.rs` para o contrato fail-closed.

## migrate:status

Imprime o estado aplicado/pendente de toda migração.

```bash
suprnova migrate:status
```

```
Migration status:
...tabela formatada pelo SeaORM de migrações aplicadas/pendentes...
```

O corpo do relatório vem do `MigratorTrait::status` do SeaORM, então a
formatação exata acompanha a versão do SeaORM da qual seu app depende.

## migrate:rollback

Reverte a última migração aplicada (ou as últimas `N`).

```bash
suprnova migrate:rollback [--step <N>]
```

| Opção | Padrão | Descrição |
|---|---|---|
| `--step <N>` | `1` | Número de migrações para reverter |

```bash
# Reverte uma migração
suprnova migrate:rollback

# Reverte as últimas três
suprnova migrate:rollback --step 3
```

```
Rolling back 3 migration(s)...
Rollback completed successfully!
```

O `down()` de cada migração executa em ordem reversa de aplicação. Um
`down()` que falha sai com código não-zero e deixa o resto da cadeia
intocado - nada mais é tentado.

## migrate:fresh

Derruba toda tabela no banco de dados e executa toda migração de novo,
do zero.

```bash
suprnova migrate:fresh
```

```
WARNING: Dropping all tables and re-running migrations...
Database refreshed successfully!
```

Isso destrói todos os dados no banco de dados conectado. É destinado
para configuração de desenvolvimento local e de teste, não para
nenhum ambiente onde os dados importam.

### A salvaguarda de produção

Fora de produção ele executa imediatamente, sem prompt - derrubar um
banco de dados local é rotina, e uma confirmação que você sempre
responde da mesma forma te treina a parar de ler.

Quando `APP_ENV` resolve para produção, ele exige dois tipos
diferentes de prova:

```bash
suprnova migrate:fresh --force   # …depois digite o nome do ambiente quando solicitado
```

1. **`--force`** prova a intenção no momento em que você digitou o
   comando.
2. **Uma confirmação digitada em um terminal interativo** prova que
   um humano está presente.

O requisito de terminal é o ponto da segunda prova. Sem ele,
`echo production | suprnova migrate:fresh --force` em um script de
deploy responderia o prompt automaticamente, e a confirmação seria só
mais uma flag. Então um stdin não interativo é recusado mesmo com
`--force`.

Qualquer coisa diferente do nome exato do ambiente aborta antes de uma
única tabela ser derrubada.

A mesma salvaguarda se aplica ao próprio subcomando do binário da sua
aplicação (`./app migrate:fresh --force`), que é o que um deploy de
produção de fato executa.

## db:sync

Regenera os arquivos de entidade SeaORM em `src/models/entities/` a
partir do esquema atual do banco de dados, e (quando um
`src/bin/migrate.rs` existe) executa as migrações pendentes primeiro.

```bash
suprnova db:sync [--skip-migrations] [--regenerate-models]
```

| Opção | Descrição |
|---|---|
| `--skip-migrations` | Pula a passagem de migração e só regenera as entidades |
| `--regenerate-models` | Sobrescreve também os arquivos `src/models/<table>.rs`, não só `src/models/entities/<table>.rs` |

### O que faz

1. (Opcional) Executa as migrações pendentes. O scaffold padrão não
   distribui um `src/bin/migrate.rs`, então esta etapa é um no-op e
   imprime `Migration binary not found, skipping migrations`. Em um
   projeto padrão, execute `suprnova migrate` primeiro, depois
   `suprnova db:sync --skip-migrations`.
2. Conecta a `DATABASE_URL`, introspecta toda tabela de usuário
   (pulando `seaql_migrations` e qualquer nome que comece com `_`), e
   escreve um arquivo de entidade por tabela em
   `src/models/entities/<table>.rs`.
3. Escreve um arquivo de model fino voltado ao usuário em
   `src/models/<table>.rs` - mas só se esse arquivo ainda não existir,
   para que seus acessadores, scopes, e observer hooks escritos à mão
   sobrevivam.
4. `--regenerate-models` sobrepõe a proteção da etapa 3 e sobrescreve
   esses arquivos de usuário. Use isso quando você ainda não os
   customizou, ou quando você tem um backup.

### Fluxo de trabalho típico

```bash
# 1. Escreva uma migração
suprnova make:migration create_posts_table
# (edite src/migrations/m..._create_posts_table.rs)

# 2. Aplique-a
suprnova migrate

# 3. Regenere as entidades para a nova tabela ficar alcançável no código
suprnova db:sync --skip-migrations
```

### Por que Suprnova diverge

O Laravel tem um `artisan` global que possui todo comando do
framework, incluindo `db:seed`. O Suprnova divide isso em dois:

- A CLI de desenvolvedor `suprnova` (este capítulo) possui o scaffold
  de projetos, os geradores, e os comandos de migração. É instalada
  uma vez por máquina de desenvolvedor via `cargo install` e faz shell
  para o binário do seu app para fazer o trabalho que precisa do
  `Migrator` do app.
- Um binário `console` por projeto, construído a partir do
  `src/bin/console.rs` do seu projeto, possui `db:seed`, seus handlers
  anotados com `#[command]`, `queue:work`, `schedule:run`,
  `workflow:work`, e outras tarefas de execução única que precisam do
  bootstrap, das vinculações de contêiner, e dos observers registrados
  do seu app.

Comandos de migração vivem na CLI de desenvolvedor porque eles têm
uma forma determinística que não depende do seu bootstrap. Tudo que
precisa do seu contêiner de serviços ou dos seus seeders registrados
vive no binário console por projeto. Veja [Console](console.md) para
a superfície completa do console.

## db:seed

Não é um comando da CLI `suprnova`. Execute os seeders através do
binário console por projeto:

```bash
cargo run --bin console -- db:seed
cargo run --bin console -- db:seed --class=UsersSeeder
```

O registro de seeders, as regras de ordenação, e a correspondência de
`--class` são cobertos em [Preenchimento de dados](seeding.md). O
framework distribui `db:seed` como um comando de console embutido -
seu scaffold o recebe sem nenhuma fiação da sua parte, mas você o
invoca através do `console`, não através do `suprnova`.

## Resumo

| Comando | O que faz |
|---|---|
| `suprnova make:migration <name>` | Faz scaffold de um novo arquivo de migração e o registra no `Migrator` |
| `suprnova migrate` | Executa as migrações pendentes |
| `suprnova migrate:status` | Mostra o status aplicado/pendente |
| `suprnova migrate:rollback [--step N]` | Reverte as últimas `N` migrações (padrão 1) |
| `suprnova migrate:fresh` | Derruba todas as tabelas e executa toda migração de novo |
| `suprnova db:sync [--skip-migrations] [--regenerate-models]` | Regenera as entidades SeaORM a partir do esquema ativo |
| `cargo run --bin console -- db:seed` | Executa os seeders registrados (console por projeto, não a CLI `suprnova`) |

## Próximos passos

- [Migrações](migrations.md) - API do construtor de esquema: tabelas,
  colunas, índices, chaves estrangeiras
- [Preenchimento de dados](seeding.md) - escrevendo seeders e o
  comando de console `db:seed`
- [Console](console.md) - o binário `console` por projeto e os
  handlers `#[command]`
- [Banco de dados](database.md) - conexões, drivers, transações, o
  construtor de consultas
- [Visão geral da CLI](cli.md) - todo subcomando `suprnova` de uma
  olhada
