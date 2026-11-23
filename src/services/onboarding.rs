use sqlx::PgPool;

use crate::{
    error::{AppError, Result},
    models::{
        business::Business,
        user::{normalize_phone, User, ROLE_EMPLOYEE},
    },
    nlp::intent::is_valid_pk_mobile,
};

/// What happened when an owner tried to add a staff member.
#[derive(Debug)]
pub enum AddEmployeeOutcome {
    Created(User),
    /// The number already belongs to someone (here or in another business).
    PhoneTaken,
    /// The name is too short or the number is not a Pakistani mobile number.
    Invalid,
}

fn clean_name(raw: Option<&str>, fallback: &str) -> String {
    let joined = raw
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let cleaned: String = joined.chars().take(40).collect();
    if cleaned.chars().count() >= 2 {
        cleaned
    } else {
        fallback.to_string()
    }
}

/// First contact from a number on the owner allowlist: create their business and their account
/// in one step. The name comes from their WhatsApp profile when available.
pub async fn register_owner(
    pool: &PgPool,
    phone: &str,
    display_name: Option<&str>,
) -> Result<User> {
    let name = clean_name(display_name, "Owner");
    let mut tx = pool.begin().await?;

    let business = sqlx::query_as::<_, Business>(
        "INSERT INTO businesses (name, language)
         VALUES ($1, 'ur')
         RETURNING id, name, language, created_at",
    )
    .bind(format!("{name} Business"))
    .fetch_one(&mut *tx)
    .await?;

    let user = sqlx::query_as::<_, User>(
        "INSERT INTO users (business_id, phone, name, role, language)
         VALUES ($1, $2, $3, 'owner', 'ur')
         RETURNING id, business_id, phone, name, role, language, is_active, created_at",
    )
    .bind(business.id)
    .bind(normalize_phone(phone))
    .bind(&name)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(user)
}

/// Is this number already registered with Awaaz (as anyone, active or not)?
pub async fn phone_in_use(pool: &PgPool, phone: &str) -> Result<bool> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE phone = $1)")
        .bind(normalize_phone(phone))
        .fetch_one(pool)
        .await?;
    Ok(exists)
}

/// Owners add staff by name and mobile number. The employee starts with the owner's language.
pub async fn add_employee(
    pool: &PgPool,
    owner: &User,
    name: &str,
    phone: &str,
) -> Result<AddEmployeeOutcome> {
    if !owner.is_owner() {
        return Err(AppError::Unauthorized);
    }

    let name = clean_name(Some(name), "");
    if name.chars().count() < 2 || !is_valid_pk_mobile(phone) {
        return Ok(AddEmployeeOutcome::Invalid);
    }
    if phone_in_use(pool, phone).await? {
        return Ok(AddEmployeeOutcome::PhoneTaken);
    }

    match User::create(
        pool,
        owner.business_id,
        phone,
        &name,
        ROLE_EMPLOYEE,
        &owner.language,
    )
    .await
    {
        Ok(user) => Ok(AddEmployeeOutcome::Created(user)),
        // Someone registered the same number between the check and the insert.
        Err(sqlx::Error::Database(db)) if db.is_unique_violation() => {
            Ok(AddEmployeeOutcome::PhoneTaken)
        }
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::test_support;
    use uuid::Uuid;

    fn random_phone() -> String {
        format!("923{:09}", Uuid::new_v4().as_u128() % 1_000_000_000)
    }

    #[test]
    fn cleans_names() {
        assert_eq!(clean_name(Some("  Ali   Khan "), "Owner"), "Ali Khan");
        assert_eq!(clean_name(Some("A"), "Owner"), "Owner");
        assert_eq!(clean_name(None, "Owner"), "Owner");
        assert_eq!(clean_name(Some(&"x".repeat(100)), "Owner").chars().count(), 40);
    }

    #[tokio::test]
    async fn owner_registration_and_staff() {
        let Some(pool) = test_support::pool().await else {
            return;
        };

        let phone = random_phone();
        let owner = register_owner(&pool, &phone, Some("Ahmed Khan")).await.unwrap();
        assert!(owner.is_owner());
        assert_eq!(owner.name, "Ahmed Khan");
        let found = User::find_active_by_phone(&pool, &phone).await.unwrap().unwrap();
        assert_eq!(found.id, owner.id);

        // Registering the same number twice is refused by the database.
        assert!(register_owner(&pool, &phone, None).await.is_err());

        // Adding staff.
        let staff_phone = random_phone();
        let created = add_employee(&pool, &owner, "Bilal", &staff_phone).await.unwrap();
        let bilal = match created {
            AddEmployeeOutcome::Created(user) => user,
            other => panic!("expected Created, got {other:?}"),
        };
        assert!(!bilal.is_owner());
        assert_eq!(bilal.business_id, owner.business_id);

        // Same number again, bad number, bad name.
        assert!(matches!(
            add_employee(&pool, &owner, "Bilal 2", &staff_phone).await.unwrap(),
            AddEmployeeOutcome::PhoneTaken
        ));
        assert!(matches!(
            add_employee(&pool, &owner, "Usman", "123").await.unwrap(),
            AddEmployeeOutcome::Invalid
        ));
        assert!(matches!(
            add_employee(&pool, &owner, "U", &random_phone()).await.unwrap(),
            AddEmployeeOutcome::Invalid
        ));

        // Staff cannot add staff.
        let attempt = add_employee(&pool, &bilal, "Usman", &random_phone()).await;
        assert!(matches!(attempt, Err(AppError::Unauthorized)));

        test_support::cleanup(&pool, owner.business_id).await;
    }
}