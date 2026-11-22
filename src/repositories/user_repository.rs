/// repositories/user_repository.rs — Query SQL per utenti e profili
///
/// REGOLA FONDAMENTALE: qui c'è SOLO SQL. Zero logica di business.
/// Ogni funzione prende il pool (o una connessione, se deve partecipare
/// a una transazione) e restituisce Result<T, AppError>.
///
/// sqlx verifica le query a COMPILE TIME contro il DB reale
/// quando usi la macro `sqlx::query_as!` — errori SQL = errori di compilazione.
use axum::http::StatusCode;
use sqlx::{PgConnection, PgPool};

use crate::errors::AppError;
use crate::models::user::{ProfileUpdate, Role, User, UserProfile};

/// Traduce la violazione dei vincoli UNIQUE di `users` nei relativi errori applicativi.
/// Fare affidamento sul vincolo (e non su un controllo preventivo) evita race condition
/// tra due registrazioni simultanee con la stessa email/username.
fn map_unique_violation(e: sqlx::Error) -> AppError {
    if let sqlx::Error::Database(db) = &e {
        match db.constraint() {
            Some("users_email_key") => {
                return AppError::coded(
                    StatusCode::CONFLICT,
                    "EMAIL_TAKEN",
                    "Email già registrata",
                );
            }
            Some("users_username_key") => {
                return AppError::coded(
                    StatusCode::CONFLICT,
                    "USERNAME_TAKEN",
                    "Username già in uso",
                );
            }
            _ => {}
        }
    }
    AppError::Database(e)
}

/// Cerca un utente per email. Ritorna None se non esiste.
/// Usato durante il login per trovare l'utente da autenticare.
pub async fn find_by_email(pool: &PgPool, email: &str) -> Result<Option<User>, AppError> {
    // `query_as!` mappa automaticamente la riga nel tipo `User`
    // La macro verifica la query contro il DB in fase di compilazione
    let user = sqlx::query_as!(
        User,
        r#"
        SELECT id, email, username, password_hash, role, is_active, created_at, updated_at
        FROM users
        WHERE email = $1
        "#,
        email
    )
    .fetch_optional(pool) // fetch_optional → Ok(Some(user)) o Ok(None)
    .await?; // ? converte sqlx::Error in AppError grazie a #[from]

    Ok(user)
}

/// Cerca un utente per email OPPURE username. Ritorna None se non esiste.
/// Usato per indicare un coach con un identificativo qualsiasi dei due.
pub async fn find_by_email_or_username(
    pool: &PgPool,
    identifier: &str,
) -> Result<Option<User>, AppError> {
    let user = sqlx::query_as!(
        User,
        r#"
        SELECT id, email, username, password_hash, role, is_active, created_at, updated_at
        FROM users
        WHERE email = $1 OR username = $1
        LIMIT 1
        "#,
        identifier
    )
    .fetch_optional(pool)
    .await?;

    Ok(user)
}

/// Cerca un utente per ID. Ritorna None se non esiste.
pub async fn find_by_id(pool: &PgPool, id: i64) -> Result<Option<User>, AppError> {
    let user = sqlx::query_as!(
        User,
        r#"
        SELECT id, email, username, password_hash, role, is_active, created_at, updated_at
        FROM users
        WHERE id = $1
        "#,
        id
    )
    .fetch_optional(pool)
    .await?;

    Ok(user)
}

/// Stato `is_active` dell'utente; `None` se l'utente non esiste.
pub async fn find_is_active(pool: &PgPool, id: i64) -> Result<Option<bool>, AppError> {
    let active = sqlx::query_scalar!("SELECT is_active FROM users WHERE id = $1", id)
        .fetch_optional(pool)
        .await?;

    Ok(active)
}

/// Crea un nuovo utente nel DB (dentro la transazione del chiamante).
/// Riceve già la password hashata — l'hash viene fatto nel service, NON qui.
/// Email/username duplicati → `409 EMAIL_TAKEN` / `409 USERNAME_TAKEN`.
pub async fn create(
    conn: &mut PgConnection,
    email: &str,
    username: &str,
    password_hash: &str,
    role: Role,
) -> Result<User, AppError> {
    // `query_as!` con INSERT ... RETURNING ci restituisce direttamente l'utente creato
    let user = sqlx::query_as!(
        User,
        r#"
        INSERT INTO users (email, username, password_hash, role, is_active, created_at, updated_at)
        VALUES ($1, $2, $3, $4, true, NOW(), NOW())
        RETURNING id, email, username, password_hash, role, is_active, created_at, updated_at
        "#,
        email,
        username,
        password_hash,
        role.as_str()
    )
    .fetch_one(conn)
    .await
    .map_err(map_unique_violation)?;

    Ok(user)
}

/// Crea la riga di profilo con i valori di default del DB (it, Europe/Rome).
pub async fn create_profile(conn: &mut PgConnection, user_id: i64) -> Result<(), AppError> {
    sqlx::query!("INSERT INTO user_profiles (user_id) VALUES ($1)", user_id)
        .execute(conn)
        .await?;

    Ok(())
}

/// Legge il profilo di un utente. `NotFound` se la riga non esiste.
pub async fn find_profile(pool: &PgPool, user_id: i64) -> Result<UserProfile, AppError> {
    sqlx::query_as!(
        UserProfile,
        r#"
        SELECT language, timezone, birth_date, sex, weight_kg, max_hr,
               resting_hr, ftp_watts, threshold_pace_s_per_km
        FROM user_profiles
        WHERE user_id = $1
        "#,
        user_id
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("Profilo dell'utente {} non trovato", user_id)))
}

/// Fuso orario IANA dell'utente (es. `Europe/Rome`).
/// Usato dai task che interpretano date/ore secondo il fuso dell'utente.
pub async fn timezone_of(pool: &PgPool, user_id: i64) -> Result<String, AppError> {
    sqlx::query_scalar!(
        "SELECT timezone FROM user_profiles WHERE user_id = $1",
        user_id
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("Profilo dell'utente {} non trovato", user_id)))
}

/// Verifica che `name` sia un fuso orario noto a PostgreSQL.
pub async fn timezone_exists(pool: &PgPool, name: &str) -> Result<bool, AppError> {
    let found = sqlx::query_scalar!("SELECT 1 FROM pg_timezone_names WHERE name = $1", name)
        .fetch_optional(pool)
        .await?;

    Ok(found.is_some())
}

/// Aggiorna email, username e/o hash della password (dentro la transazione del chiamante).
/// `None` = campo invariato. Duplicati → `409 EMAIL_TAKEN` / `409 USERNAME_TAKEN`.
pub async fn update_account(
    conn: &mut PgConnection,
    id: i64,
    email: Option<&str>,
    username: Option<&str>,
    password_hash: Option<&str>,
) -> Result<(), AppError> {
    // COALESCE: se il nuovo valore è NULL, mantieni quello esistente.
    // Questo è il pattern per PATCH (aggiornamento parziale).
    sqlx::query!(
        r#"
        UPDATE users
        SET
            email         = COALESCE($2, email),
            username      = COALESCE($3, username),
            password_hash = COALESCE($4, password_hash),
            updated_at    = NOW()
        WHERE id = $1
        "#,
        id,
        email,
        username,
        password_hash
    )
    .execute(conn)
    .await
    .map_err(map_unique_violation)?;

    Ok(())
}

/// Aggiorna il profilo (dentro la transazione del chiamante).
///
/// Per i campi nullable ogni coppia di parametri è (flag "da modificare", nuovo valore):
/// flag falso → colonna invariata; flag vero → colonna = valore (anche NULL).
pub async fn update_profile(
    conn: &mut PgConnection,
    user_id: i64,
    u: &ProfileUpdate,
) -> Result<(), AppError> {
    sqlx::query!(
        r#"
        UPDATE user_profiles
        SET
            language                = COALESCE($2, language),
            timezone                = COALESCE($3, timezone),
            birth_date              = CASE WHEN $4 THEN $5::date ELSE birth_date END,
            sex                     = CASE WHEN $6 THEN $7::text ELSE sex END,
            weight_kg               = CASE WHEN $8 THEN $9::float8 ELSE weight_kg END,
            max_hr                  = CASE WHEN $10 THEN $11::int4 ELSE max_hr END,
            resting_hr              = CASE WHEN $12 THEN $13::int4 ELSE resting_hr END,
            ftp_watts               = CASE WHEN $14 THEN $15::int4 ELSE ftp_watts END,
            threshold_pace_s_per_km = CASE WHEN $16 THEN $17::int4 ELSE threshold_pace_s_per_km END
        WHERE user_id = $1
        "#,
        user_id,
        u.language.as_deref(),
        u.timezone.as_deref(),
        u.birth_date.is_some(),
        u.birth_date.flatten(),
        u.sex.is_some(),
        u.sex.clone().flatten(),
        u.weight_kg.is_some(),
        u.weight_kg.flatten(),
        u.max_hr.is_some(),
        u.max_hr.flatten(),
        u.resting_hr.is_some(),
        u.resting_hr.flatten(),
        u.ftp_watts.is_some(),
        u.ftp_watts.flatten(),
        u.threshold_pace_s_per_km.is_some(),
        u.threshold_pace_s_per_km.flatten(),
    )
    .execute(conn)
    .await?;

    Ok(())
}

/// Elimina un utente per ID (il profilo sparisce in cascata).
/// Ritorna il numero di righe eliminate (0 se non esiste).
pub async fn delete(pool: &PgPool, id: i64) -> Result<u64, AppError> {
    let result: sqlx::postgres::PgQueryResult = sqlx::query!("DELETE FROM users WHERE id = $1", id)
        .execute(pool)
        .await?;

    Ok(result.rows_affected())
}
