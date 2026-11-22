/// repositories/user_repository.rs — Query SQL per gli utenti
///
/// REGOLA FONDAMENTALE: qui c'è SOLO SQL. Zero logica di business.
/// Ogni funzione prende il pool come parametro e restituisce
/// Result<T, sqlx::Error> oppure Result<T, AppError>.
///
/// sqlx verifica le query a COMPILE TIME contro il DB reale
/// quando usi la macro `sqlx::query_as!` — errori SQL = errori di compilazione.

use sqlx::PgPool;

use crate::errors::AppError;
use crate::models::user::User;

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
    .await?;              // ? converte sqlx::Error in AppError grazie a #[from]

    Ok(user)
}

/// Cerca un utente per ID. Ritorna None se non esiste.
/// Usato per GET /users/:id e per la validazione del JWT.
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

/// Crea un nuovo utente nel DB.
/// Riceve già la password hashata — l'hash viene fatto nel service, NON qui.
/// TODO(Task 2): il ruolo è fisso `athlete` finché la registrazione non lo riceve in input.
pub async fn create(
    pool: &PgPool,
    email: &str,
    username: &str,
    password_hash: &str,
) -> Result<User, AppError> {
    // `query_as!` con INSERT ... RETURNING ci restituisce direttamente l'utente creato
    let user = sqlx::query_as!(
        User,
        r#"
        INSERT INTO users (email, username, password_hash, role, is_active, created_at, updated_at)
        VALUES ($1, $2, $3, 'athlete', true, NOW(), NOW())
        RETURNING id, email, username, password_hash, role, is_active, created_at, updated_at
        "#,
        email,
        username,
        password_hash
    )
    .fetch_one(pool) // fetch_one → Ok(user) o Err (es. violazione UNIQUE)
    .await?;

    Ok(user)
}

/// Recupera tutti gli utenti con paginazione.
/// `page` e `per_page` vengono dal query param: GET /users?page=1&per_page=20
pub async fn find_all(pool: &PgPool, page: i64, per_page: i64) -> Result<Vec<User>, AppError> {
    let offset = (page - 1) * per_page;

    let users = sqlx::query_as!(
        User,
        r#"
        SELECT id, email, username, password_hash, role, is_active, created_at, updated_at
        FROM users
        ORDER BY created_at DESC
        LIMIT $1 OFFSET $2
        "#,
        per_page,
        offset
    )
    .fetch_all(pool) // fetch_all → Ok(Vec<User>) anche se vuoto
    .await?;

    Ok(users)
}

/// Conta il totale degli utenti — usato per la paginazione.
pub async fn count(pool: &PgPool) -> Result<i64, AppError> {
    let row = sqlx::query!("SELECT COUNT(*) as count FROM users")
        .fetch_one(pool)
        .await?;

    Ok(row.count.unwrap_or(0))
}

/// Aggiorna email e/o username di un utente.
/// Ritorna l'utente aggiornato.
pub async fn update(
    pool: &PgPool,
    id: i64,
    email: Option<&str>,
    username: Option<&str>,
    password_hash: Option<&str>,
) -> Result<User, AppError> {
    // COALESCE: se il nuovo valore è NULL, mantieni quello esistente.
    // Questo è il pattern per PATCH (aggiornamento parziale).
    let user = sqlx::query_as!(
        User,
        r#"
        UPDATE users
        SET
            email        = COALESCE($2, email),
            username     = COALESCE($3, username),
            password_hash = COALESCE($4, password_hash),
            updated_at   = NOW()
        WHERE id = $1
        RETURNING id, email, username, password_hash, role, is_active, created_at, updated_at
        "#,
        id,
        email as Option<&str>,
        username as Option<&str>,
        password_hash as Option<&str>
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => AppError::NotFound(format!("Utente {} non trovato", id)),
        other => AppError::Database(other),
    })?;

    Ok(user)
}

/// Elimina un utente per ID.
/// Ritorna il numero di righe eliminate (0 se non esiste).
pub async fn delete(pool: &PgPool, id: i64) -> Result<u64, AppError> {
    let result: sqlx::postgres::PgQueryResult = sqlx::query!("DELETE FROM users WHERE id = $1", id)
        .execute(pool)
        .await?;

    Ok(result.rows_affected())
}
