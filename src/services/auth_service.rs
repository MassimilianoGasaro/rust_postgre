/// services/auth_service.rs — Logica di business per l'autenticazione
///
/// Questo layer si occupa di:
/// - Validare i dati di registrazione
/// - Hashare le password con bcrypt (mai in chiaro nel DB!)
/// - Verificare la password al login
/// - Generare token JWT alla registrazione/login
/// - Validare i JWT nelle richieste protette
///
/// NON sa nulla di HTTP: non usa Request/Response di Axum.
/// Riceve tipi semplici (stringhe, struct) e restituisce Result<T, AppError>.
use axum::http::StatusCode;
use chrono::{Duration, Utc};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use sqlx::PgPool;

use crate::dto::auth::{AuthResponse, Claims, LoginRequest, RegisterRequest, UserInfo};
use crate::errors::AppError;
use crate::models::user::{Role, User};
use crate::repositories::user_repository;

/// Lunghezza minima della password.
pub const MIN_PASSWORD_LEN: usize = 8;

/// Errore di validazione: `400 VALIDATION_FAILED` con il nome del campo nel messaggio.
pub fn validation_error(message: impl Into<String>) -> AppError {
    AppError::coded(StatusCode::BAD_REQUEST, "VALIDATION_FAILED", message)
}

/// Email: contiene `@` ed è lunga al massimo 255 caratteri (limite della colonna).
pub fn validate_email(email: &str) -> Result<(), AppError> {
    if !email.contains('@') || email.chars().count() > 255 {
        return Err(validation_error(
            "email non valida: deve contenere '@' e al massimo 255 caratteri",
        ));
    }
    Ok(())
}

/// Username: 3–50 caratteri tra lettere ASCII, cifre, `_`, `.`, `-`.
pub fn validate_username(username: &str) -> Result<(), AppError> {
    let len = username.chars().count();
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-');
    if !(3..=50).contains(&len) || !username.chars().all(allowed) {
        return Err(validation_error(
            "username non valido: 3-50 caratteri tra lettere, cifre, '_', '.', '-'",
        ));
    }
    Ok(())
}

/// Password: almeno `MIN_PASSWORD_LEN` caratteri.
pub fn validate_password(password: &str) -> Result<(), AppError> {
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err(validation_error(format!(
            "password troppo corta: minimo {} caratteri",
            MIN_PASSWORD_LEN
        )));
    }
    Ok(())
}

/// Hash bcrypt della password.
/// bcrypt::hash è CPU-intensive: spawn_blocking evita di bloccare il thread asincrono.
pub async fn hash_password(password: String) -> Result<String, AppError> {
    tokio::task::spawn_blocking(move || bcrypt::hash(password, bcrypt::DEFAULT_COST))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .map_err(AppError::from)
}

/// Confronta una password con l'hash bcrypt salvato.
/// bcrypt::verify è CPU-intensive: spawn_blocking evita di bloccare il thread asincrono.
pub async fn verify_password(password: String, hash: String) -> Result<bool, AppError> {
    tokio::task::spawn_blocking(move || bcrypt::verify(password, &hash))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .map_err(AppError::from)
}

/// Credenziali errate: `401 INVALID_CREDENTIALS`.
pub fn invalid_credentials() -> AppError {
    AppError::coded(
        StatusCode::UNAUTHORIZED,
        "INVALID_CREDENTIALS",
        "Credenziali non valide",
    )
}

/// Costruisce la risposta di login/registrazione (token + info utente).
fn auth_response(
    user: User,
    jwt_secret: &str,
    jwt_expiration_hours: i64,
) -> Result<AuthResponse, AppError> {
    let role = Role::parse(&user.role)
        .ok_or_else(|| AppError::Internal(format!("Ruolo non valido nel DB: {}", user.role)))?;
    let token = generate_jwt(user.id, &user.email, role, jwt_secret, jwt_expiration_hours)?;

    Ok(AuthResponse {
        token,
        user: UserInfo {
            id: user.id,
            email: user.email,
            username: user.username,
            role,
        },
    })
}

/// Registra un nuovo utente.
///
/// Flusso:
/// 1. Valida email, username e password
/// 2. Hasha la password con bcrypt
/// 3. In UNA transazione crea l'utente e il suo profilo (con i default);
///    email/username duplicati sono intercettati dal vincolo UNIQUE
/// 4. Genera e restituisce il token JWT
pub async fn register(
    pool: &PgPool,
    jwt_secret: &str,
    jwt_expiration_hours: i64,
    req: RegisterRequest,
) -> Result<AuthResponse, AppError> {
    // 1. Validazione
    validate_email(&req.email)?;
    validate_username(&req.username)?;
    validate_password(&req.password)?;

    // 2. Hash della password
    let password_hash = hash_password(req.password).await?;

    // 3. Utente + profilo nella stessa transazione: o entrambi o nessuno
    let mut tx = pool.begin().await?;
    let user =
        user_repository::create(&mut tx, &req.email, &req.username, &password_hash, req.role)
            .await?;
    user_repository::create_profile(&mut tx, user.id).await?;
    tx.commit().await?;

    // 4. Genera JWT
    auth_response(user, jwt_secret, jwt_expiration_hours)
}

/// Effettua il login di un utente esistente.
///
/// Flusso:
/// 1. Cerca l'utente per email
/// 2. Confronta la password con l'hash in DB
/// 3. Verifica che l'account sia attivo
/// 4. Genera e restituisce il token JWT
///
/// Lo stato dell'account si controlla DOPO la password: chi non la conosce
/// non deve poter capire se un account esiste ed è disabilitato.
pub async fn login(
    pool: &PgPool,
    jwt_secret: &str,
    jwt_expiration_hours: i64,
    req: LoginRequest,
) -> Result<AuthResponse, AppError> {
    let invalid = invalid_credentials;

    // 1. Cerca l'utente — stesso errore per email sconosciuta e password errata
    let user = user_repository::find_by_email(pool, &req.email)
        .await?
        .ok_or_else(invalid)?;

    // 2. Verifica password — anche questa è CPU-intensive
    if !verify_password(req.password, user.password_hash.clone()).await? {
        return Err(invalid());
    }

    // 3. Account disabilitato
    if !user.is_active {
        return Err(AppError::coded(
            StatusCode::UNAUTHORIZED,
            "ACCOUNT_DISABLED",
            "Account disabilitato",
        ));
    }

    // 4. Genera JWT
    auth_response(user, jwt_secret, jwt_expiration_hours)
}

/// Valida un token JWT e restituisce i Claims in esso contenuti.
///
/// Usata dal middleware di autenticazione per proteggere gli endpoint.
/// Se il token è scaduto, malformato o firmato con secret sbagliato → Unauthorized.
pub fn validate_jwt(token: &str, jwt_secret: &str) -> Result<Claims, AppError> {
    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(jwt_secret.as_bytes()),
        &Validation::default(), // valida firma + scadenza automaticamente
    )
    .map_err(|e| AppError::Unauthorized(format!("Token non valido: {}", e)))?;

    Ok(token_data.claims)
}

/// Genera un JWT firmato con HMAC-SHA256.
///
/// Il token contiene:
/// - `sub`: ID dell'utente
/// - `email`: email dell'utente (utile per display senza query al DB)
/// - `role`: ruolo (`athlete` / `coach`) per i controlli di autorizzazione
/// - `iat`: timestamp di emissione
/// - `exp`: timestamp di scadenza (iat + jwt_expiration_hours)
fn generate_jwt(
    user_id: i64,
    email: &str,
    role: Role,
    jwt_secret: &str,
    expiration_hours: i64,
) -> Result<String, AppError> {
    let now = Utc::now();
    let exp = (now + Duration::hours(expiration_hours)).timestamp() as usize;
    let iat = now.timestamp() as usize;

    let claims = Claims {
        sub: user_id,
        email: email.to_string(),
        role: role.as_str().to_string(),
        exp,
        iat,
    };

    // Header::default() usa algoritmo HS256
    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(jwt_secret.as_bytes()),
    )?;

    Ok(token)
}
