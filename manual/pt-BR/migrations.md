# Migrações

Migrações descrevem como seu esquema evolui - cada arquivo é uma
struct Rust pequena com métodos `up()` e `down()` que o framework
executa em ordem de timestamp. Use-as sempre que você mudar tabelas,
colunas, índices, ou chaves estrangeiras; essa mudança vai do seu
laptop para o staging e para a produção executando o mesmo comando
migrate em cada lugar.

Por baixo dos panos, as migrações do Suprnova são migrações do
SeaORM. A CLI as gera, o `Migrator` as agrega, e
`Application::migrations::<Migrator>()` as encaixa no boot da sua
app. Para a referência completa por comando (flags, exemplos de
saída, exit codes) veja
[Referência de migrações da CLI](cli-migrations.md); este capítulo
cobre o que colocar *dentro* dos arquivos.

## Criando migrações

Gere um novo arquivo de migração:

```bash
suprnova make:migration create_users_table
```

O gerador escreve um arquivo com timestamp em `src/migrations/`
(criando o diretório na primeira vez) e o registra no `Migrator`:

```
src/migrations/
├── mod.rs                              ← o Migrator (gerenciado pela CLI)
└── m20240115_120000_create_users_table.rs
```

O nome do arquivo é `m{YYYYMMDD}_{HHMMSS}_<name>.rs`; a ordenação é
por nome de arquivo, então o prefixo de timestamp é o que impõe uma
ordem de aplicação determinística.

### O que o gerador emite

`make:migration create_users_table` produz este esqueleto:

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

O gerador infere o nome da tabela a partir do nome da migração
(`create_X_table` → `X`, `add_Y_to_X` → `X`, `drop_X_table` → `X`).
Qualquer outro caso se torna o nome literal.

### O Migrator

`src/migrations/mod.rs` coleta toda migração em um único `Migrator`
que o `MigratorTrait` percorre. A CLI mantém este arquivo quando você
executa `make:migration`, então você raramente o toca à mão:

```rust
pub use sea_orm_migration::prelude::*;

mod m20240115_120000_create_users_table;
mod m20240115_130000_create_posts_table;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20240115_120000_create_users_table::Migration),
            Box::new(m20240115_130000_create_posts_table::Migration),
        ]
    }
}
```

Conecte o migrator ao `main.rs` da sua app para que `serve`,
`migrate`, `migrate:status`, `migrate:rollback`, e `migrate:fresh`
todos vejam a mesma lista:

```rust
use suprnova::Application;

#[suprnova::main]
async fn main() {
    Application::new()
        .config(my_app::config::register)
        .bootstrap(my_app::bootstrap::bootstrap)
        .routes(my_app::routes::register)
        .migrations::<my_app::migrations::Migrator>()
        .run()
        .await
}
```

O scaffolder escreve isso para você em `suprnova new`.

### Por que Suprnova diverge

A maior parte do framework deliberadamente esconde o SeaORM - você
escreve `#[suprnova::model]` e `User::query().db_where(...)`, não
`Entity::find().filter(...)`. Migrações são o único lugar em que
deixamos `sea_orm_migration::prelude::*` visível. Dois motivos.

Primeiro, a DSL do construtor de schema é genuinamente boa, e
re-apelidar todo nome nela (`Table`, `ColumnDef`, `Index`,
`ForeignKey`, `Expr`, `ForeignKeyAction`, `DeriveIden`, ...) só
compraria uma linha de import mais longa e nada além disso. Segundo,
arquivos de migração são Rust puro - seu compilador de CI os
verifica - e isso captura mais erros de digitação do que qualquer
re-apelidamento de DSL conseguiria. Tratamos migrações como
esquema-como-código, e os nomes canônicos do SeaORM *são* o
vocabulário do esquema.

Se algum dia você precisar de um tipo do SeaORM que o framework não
re-exportou, a válvula de escape é `use suprnova::sea_orm;`. Você
quase nunca precisa disso.

## Estrutura da migração

Toda migração tem dois métodos:

```rust
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    // Aplica a mudança
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> { /* ... */ }

    // Reverte a mudança
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> { /* ... */ }
}
```

Os dois métodos retornam `Result<(), DbErr>` - propague erros com `?`
e o framework transforma uma migração que falhou em um exit
não-zero para que pipelines de deploy abortem.

## Operações de esquema

### Criando tabelas

```rust
use sea_orm_migration::prelude::*;

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
                .col(ColumnDef::new(Users::Email).string().not_null().unique_key())
                .col(ColumnDef::new(Users::Name).string().not_null())
                .col(ColumnDef::new(Users::PasswordHash).string().not_null())
                .col(ColumnDef::new(Users::CreatedAt).timestamp().not_null())
                .col(ColumnDef::new(Users::UpdatedAt).timestamp().not_null())
                .to_owned(),
        )
        .await
}

// Define os identificadores de tabela e coluna
#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
    Email,
    Name,
    PasswordHash,
    CreatedAt,
    UpdatedAt,
}
```

### Removendo tabelas

```rust
async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .drop_table(Table::drop().table(Users::Table).to_owned())
        .await
}
```

### Tipos de coluna

| Método | Tipo no banco de dados | Notas |
|--------|---------------|-------|
| `integer()` | INTEGER | Inteiro de 32 bits |
| `big_integer()` | BIGINT | Inteiro de 64 bits |
| `small_integer()` | SMALLINT | Inteiro de 16 bits |
| `float()` | FLOAT | Ponto flutuante |
| `double()` | DOUBLE | Precisão dupla |
| `decimal()` | DECIMAL | Ponto fixo |
| `string()` | VARCHAR(255) | String de tamanho variável |
| `string_len(n)` | VARCHAR(n) | String de tamanho customizado |
| `text()` | TEXT | Texto longo |
| `boolean()` | BOOLEAN | Verdadeiro/falso |
| `timestamp()` | TIMESTAMP | Data e hora |
| `date()` | DATE | Somente data |
| `time()` | TIME | Somente hora |
| `blob()` | BLOB | Dados binários |
| `json()` | JSON | Dados JSON |
| `uuid()` | UUID | Tipo UUID |

### Modificadores de coluna

```rust
ColumnDef::new(Column::Name)
    .string()
    .not_null()                                // Restrição NOT NULL
    .null()                                    // Permite NULL (padrão)
    .default("value")                          // Valor padrão
    .default(Expr::current_timestamp())        // Padrão por função (ex.: NOW())
    .unique_key()                              // Restrição UNIQUE
    .primary_key()                             // PRIMARY KEY
    .auto_increment()                          // AUTO_INCREMENT
```

Para chaves primárias substitutas, prefira
`big_integer().auto_increment().primary_key()` em tabelas reais -
`INTEGER` (32 bits) é adequado para tabelas de lookup pequenas, mas
as tabelas com scaffold `users`, `sessions`, e semelhantes todas
usam `BIGINT` porque um contador de 4 bytes é o tipo de restrição de
que você vai se arrepender três anos depois.

## Adicionando colunas

```rust
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .alter_table(
            Table::alter()
                .table(Users::Table)
                .add_column(
                    ColumnDef::new(Users::PhoneNumber)
                        .string()
                        .null()
                )
                .to_owned(),
        )
        .await
}

async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .alter_table(
            Table::alter()
                .table(Users::Table)
                .drop_column(Users::PhoneNumber)
                .to_owned(),
        )
        .await
}
```

## Modificando colunas

```rust
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .alter_table(
            Table::alter()
                .table(Users::Table)
                .modify_column(
                    ColumnDef::new(Users::Name)
                        .string_len(500)  // Muda VARCHAR(255) para VARCHAR(500)
                        .not_null()
                )
                .to_owned(),
        )
        .await
}
```

## Renomeando colunas

```rust
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .alter_table(
            Table::alter()
                .table(Users::Table)
                .rename_column(Users::Name, Users::FullName)
                .to_owned(),
        )
        .await
}
```

## Índices

### Criando índices

```rust
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .create_index(
            Index::create()
                .name("idx_users_email")
                .table(Users::Table)
                .col(Users::Email)
                .unique()  // Opcional: torna único
                .to_owned(),
        )
        .await
}
```

### Índices compostos

```rust
manager
    .create_index(
        Index::create()
            .name("idx_posts_user_created")
            .table(Posts::Table)
            .col(Posts::UserId)
            .col(Posts::CreatedAt)
            .to_owned(),
    )
    .await
```

### Removendo índices

```rust
async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .drop_index(Index::drop().name("idx_users_email").to_owned())
        .await
}
```

## Chaves estrangeiras

### Adicionando chaves estrangeiras

```rust
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(Posts::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(Posts::Id)
                        .integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(ColumnDef::new(Posts::UserId).integer().not_null())
                .col(ColumnDef::new(Posts::Title).string().not_null())
                .col(ColumnDef::new(Posts::Content).text().not_null())
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_posts_user")
                        .from(Posts::Table, Posts::UserId)
                        .to(Users::Table, Users::Id)
                        .on_delete(ForeignKeyAction::Cascade)
                        .on_update(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await
}
```

### Ações de chave estrangeira

| Ação | Descrição |
|--------|-------------|
| `Cascade` | Exclui/atualiza linhas filhas automaticamente |
| `SetNull` | Define a chave estrangeira como NULL |
| `SetDefault` | Define a chave estrangeira como o valor padrão |
| `Restrict` | Impede exclusão/atualização se houver referência |
| `NoAction` | Similar a Restrict |

## Fluxo de trabalho de migração

Uma mudança típica passa por quatro etapas:

```bash
# 1. Gera o arquivo (cria src/migrations/m{ts}_create_posts_table.rs
#    e atualiza src/migrations/mod.rs).
suprnova make:migration create_posts_table

# 2. Edite src/migrations/m{ts}_create_posts_table.rs para definir
#    seu esquema.

# 3. Aplica a migração.
suprnova migrate

# 4. Regenera os arquivos de entidade do SeaORM a partir do esquema
#    ativo para que os models compilem contra a nova forma.
#    `db:sync` também executa qualquer migração pendente primeiro
#    (use --skip-migrations para pular essa etapa).
suprnova db:sync
```

`db:sync` escreve a cola de entidade auto-gerada em
`src/models/entities/<table>.rs` e um stub editável pelo usuário em
`src/models/<table>.rs`. Executá-lo de novo atualiza os arquivos de
entidade; seus stubs de usuário são deixados intactos a menos que
você passe `--regenerate-models` (que os sobrescreve - mantenha
métodos customizados em outro lugar ou faça version-control antes de
executá-lo).

### Migração automática no serve

`suprnova serve` e `suprnova web:run` aplicam qualquer migração
pendente antes de abrir o socket HTTP. A política padrão é
**fail-closed**: se `up()` retornar erro, o processo aborta com
código não-zero antes do bind, então uma migração quebrada nunca
pode alcançar o tráfego.

Duas válvulas de escape:

| Flag / env | Efeito |
|---|---|
| `--no-migrate` (em `serve` / `web:run`) | Pula inteiramente a etapa de migração automática. Útil quando as migrações rodam a partir de uma etapa de deploy separada. |
| `SUPRNOVA_AUTO_MIGRATE_BEST_EFFORT=true` | Retorna ao comportamento legado de log-e-continue. O processo continua inicializando mesmo com um erro de migração. Não recomendado em produção. |

Workers em background (`queue:work`, `workflow:work`,
`schedule:run`) *não* fazem migração automática - eles assumem que o
esquema já está em vigor quando inicializam, já que executar
migrações a partir de N workers concorrentemente causaria uma
corrida.

### Executando migrações em testes

`TestDatabase::fresh::<Migrator>()` levanta um banco de dados SQLite
em memória isolado, executa toda migração, e vincula a conexão ao
contêiner de teste para que `DB::connection()` e `#[inject]`
resolvam a ela:

```rust
use suprnova::testing::TestDatabase;
use crate::migrations::Migrator;

#[tokio::test]
async fn users_table_is_created() {
    let db = TestDatabase::fresh::<Migrator>().await.unwrap();
    // `db` dropa ao fim do teste, limpando o contêiner.
}
```

Veja [Testes de banco de dados](database-testing.md) para o padrão
completo (factories, segurança em paralelo, escolher um driver real
em vez de SQLite em memória).

## Boas práticas

### Sempre escreva o `down()`

Sempre implemente `down()` para permitir rollbacks:

```rust
// Bom: migração reversível
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager.create_table(/* ... */).await
}

async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager.drop_table(/* ... */).await
}
```

### Use nomes descritivos

```bash
# Bom: descreve a mudança
suprnova make:migration add_email_verified_to_users
suprnova make:migration create_order_items_table
suprnova make:migration add_index_to_posts_slug

# Ruim: nomes vagos
suprnova make:migration update_users
suprnova make:migration change_table
```

### Uma mudança por migração

Mantenha as migrações focadas em uma única mudança:

```bash
# Bom: migrações separadas
suprnova make:migration create_categories_table
suprnova make:migration add_category_id_to_posts

# Evite: múltiplas mudanças não relacionadas em uma migração
```

### Teste as migrações nos dois sentidos

Antes de commitar, verifique que as duas direções funcionam:

```bash
suprnova migrate           # Aplica
suprnova migrate:rollback  # Reverte
suprnova migrate           # Aplica de novo
```

## Comandos da CLI de uma olhada

| Comando | Descrição |
|---------|-------------|
| `suprnova make:migration <name>` | Cria uma nova migração |
| `suprnova migrate` | Executa todas as migrações pendentes |
| `suprnova migrate:status` | Mostra o status das migrações |
| `suprnova migrate:rollback` | Reverte a última migração |
| `suprnova migrate:rollback --step 3` | Reverte as últimas 3 migrações |
| `suprnova migrate:fresh` | Remove todas as tabelas e executa toda migração de novo |
| `suprnova db:sync` | Executa as migrações e regenera os arquivos de entidade |
| `suprnova db:sync --skip-migrations` | Regenera os arquivos de entidade sem aplicar migrações |
| `suprnova db:sync --regenerate-models` | Também sobrescreve os stubs de model editáveis pelo usuário |

Veja [Referência de migrações da CLI](cli-migrations.md) para a
referência completa por comando (flags, exemplos de saída, exit
codes).

## Próximos passos

- [Referência de migrações da CLI](cli-migrations.md) - referência
  flag por flag para `migrate*` e `db:sync`
- [Banco de dados](database.md) - configuração de conexão,
  transações, divisão leitura/escrita
- [Eloquent](eloquent.md) - a camada de model que suas migrações
  alimentam
- [Preenchimento de dados](seeding.md) - preenchendo tabelas uma vez
  que o esquema delas existe
- [Testes de banco de dados](database-testing.md) -
  `TestDatabase::fresh::<Migrator>()` e padrões seguros em paralelo
