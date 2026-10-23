//! A `mariadb://` URL is accepted wherever a `mysql://` URL is: MariaDB is
//! the manual's recommended production database, and its own scheme is what
//! a MariaDB operator writes.

use suprnova::database::{DatabaseConfig, DatabaseType, DbConnection};

#[test]
fn a_mariadb_url_is_a_mysql_family_database_that_names_mariadb() {
    let mariadb = DatabaseConfig::builder()
        .url("mariadb://app:secret@db.internal:3306/shop")
        .build();
    let mysql = DatabaseConfig::builder()
        .url("mysql://app:secret@db.internal:3306/shop")
        .build();

    assert_eq!(mariadb.database_type(), DatabaseType::Mysql);
    assert!(mariadb.names_mariadb());
    assert_eq!(mysql.database_type(), DatabaseType::Mysql);
    assert!(
        !mysql.names_mariadb(),
        "a mysql:// URL does not say which server answers it"
    );
}

#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn live_mysql_a_mariadb_url_connects_and_queries() {
    use suprnova::sea_orm::ConnectionTrait;

    let url = std::env::var("MYSQL_TEST_URL")
        .expect("set MYSQL_TEST_URL to a disposable MariaDB or MySQL");
    let url = url.replacen("mysql://", "mariadb://", 1);
    assert!(
        url.starts_with("mariadb://"),
        "the test URL uses the mysql scheme"
    );
    let config = DatabaseConfig::builder()
        .url(url)
        .max_connections(1)
        .min_connections(1)
        .logging(false)
        .build();

    let connection = DbConnection::connect(&config)
        .await
        .expect("a mariadb:// URL reaches the server a mysql:// URL reaches");

    connection
        .inner()
        .execute_unprepared("SELECT 1")
        .await
        .expect("the connection answers a query");
}
