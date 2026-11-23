pub mod ledger;
pub mod onboarding;
pub mod reports;
pub mod tasks;

/// Helpers shared by the database tests of the services. They use the DATABASE_URL from .env
/// and skip themselves (printing why) when the database is not reachable.
#[cfg(test)]
pub(crate) mod test_support {
    use sqlx::PgPool;
    use uuid::Uuid;

    use crate::{
        db,
        models::{
            business::Business,
            user::{User, ROLE_EMPLOYEE, ROLE_OWNER},
        },
    };

    pub async fn pool() -> Option<PgPool> {
        dotenvy::dotenv().ok();
        let url = match std::env::var("DATABASE_URL") {
            Ok(url) => url,
            Err(_) => {
                eprintln!("skipping database test: DATABASE_URL is not set");
                return None;
            }
        };
        let pool = match db::connect(&url).await {
            Ok(pool) => pool,
            Err(e) => {
                eprintln!("skipping database test: cannot connect ({e})");
                return None;
            }
        };
        if let Err(e) = db::run_migrations(&pool).await {
            eprintln!("skipping database test: migrations failed ({e})");
            return None;
        }
        Some(pool)
    }

    fn random_phone() -> String {
        format!("92{:010}", Uuid::new_v4().as_u128() % 10_000_000_000)
    }

    pub async fn business_with_owner(pool: &PgPool) -> (Business, User) {
        let business = Business::create(pool, "Test Shop", "ur").await.unwrap();
        let owner = User::create(pool, business.id, &random_phone(), "Owner", ROLE_OWNER, "ur")
            .await
            .unwrap();
        (business, owner)
    }

    pub async fn add_employee(pool: &PgPool, business_id: Uuid, name: &str) -> User {
        User::create(pool, business_id, &random_phone(), name, ROLE_EMPLOYEE, "ur")
            .await
            .unwrap()
    }

    /// Deleting the business removes its users, parties, transactions and tasks too.
    pub async fn cleanup(pool: &PgPool, business_id: Uuid) {
        sqlx::query("DELETE FROM businesses WHERE id = $1")
            .bind(business_id)
            .execute(pool)
            .await
            .ok();
    }
}