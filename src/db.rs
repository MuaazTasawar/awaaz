use std::time::Duration;

use sqlx::{postgres::PgPoolOptions, PgPool};

/// Open the Postgres connection pool.
pub async fn connect(database_url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(10))
        .connect(database_url)
        .await
}

/// Apply every SQL file in ./migrations that has not been applied yet.
/// The files are embedded into the binary at compile time.
pub async fn run_migrations(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await
}