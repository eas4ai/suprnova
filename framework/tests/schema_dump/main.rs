//! Integration tests for `schema:dump`: the dump, the load before newer
//! migrations, and pruning (PAR-038 to PAR-040). SQLite runs everywhere;
//! Postgres, MySQL and MariaDB run with `--run-ignored all` and their
//! `PG_TEST_URL`, `MYSQL_TEST_URL` and `MARIADB_TEST_URL`.

pub mod cases;
pub mod engines;
pub mod prune;
pub mod sqlite;
