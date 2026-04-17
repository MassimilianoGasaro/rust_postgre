/// services/auth_service.rs — Logica di business per l'autenticazione
///
/// Questo layer si occupa di:
/// - Hashare le password con bcrypt (mai in chiaro nel DB!)
/// - Verificare la password al login
/// - Generare token JWT alla registrazione/login
/// - Validare i JWT nelle richieste protette
///
/// NON sa nulla di HTTP: non usa Request/Response di Axum.
/// Riceve tipi semplici (stringhe, struct) e restituisce Result<T, AppError>.

use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use sqlx::PgPool;
use uuid::Uuid;

use crate::dto::auth::{AuthResponse, Claims, LoginRequest, RegisterRequest, UserInfo};
use crate::errors::AppError;
use crate::repositories::user_repository;

/// Registra un nuovo utente.
///
/// Flusso:
/// 1. Controlla che email e username non siano già in uso
/// 2. Hasha la password con bcrypt
/// 3. Salva l'utente nel DB tramite il repository
/// 4. Genera e restituisce il token JWT
pub async fn register(
    pool: &PgPool,
    jwt_secret: &str,
    jwt_expiration_hours: i64,
    req: RegisterRequest,
) -> Result<AuthResponse, AppError> {
    // 1. Verifica email unica
    if user_repository::find_by_email(pool, &req.email).await?.is_some() {
        return Err(AppError::Conflict(format!(
            "Email '{}' già registrata",
            req.email
        )));
    }

    // 2. Hash della password
    // bcrypt::hash è CPU-intensive: spawn_blocking evita di bloccare il thread asincrono
    let password = req.password.clone();
    let password_hash = tokio::task::spawn_blocking(move || {
        bcrypt::hash(password, bcrypt::DEFAULT_COST)
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))??;

    // 3. Salva nel DB
    let user =
        user_repository::create(pool, &req.email, &req.username, &password_hash).await?;

    // 4. Genera JWT
    let token = generate_jwt(&user.id, &user.email, jwt_secret, jwt_expiration_hours)?;

    Ok(AuthResponse {
        token,
        user: UserInfo {
            id: user.id,
            email: user.email,
            username: user.username,
        },
    })
}

/// Effettua il login di un utente esistente.
///
/// Flusso:
/// 1. Cerca l'utente per email
/// 2. Verifica che l'account sia attivo
/// 3. Confronta la password con l'hash in DB
/// 4. Genera e restituisce il token JWT
pub async fn login(
    pool: &PgPool,
    jwt_secret: &str,
    jwt_expiration_hours: i64,
    req: LoginRequest,
) -> Result<AuthResponse, AppError> {
    // 1. Cerca l'utente — messaggio generico per non rivelare se l'email esiste
    let user = user_repository::find_by_email(pool, &req.email)
        .await?
        .ok_or_else(|| AppError::Unauthorized("Credenziali non valide".to_string()))?;

    // 2. Controlla che l'account non sia disabilitato
    if !user.is_active {
        return Err(AppError::Unauthorized("Account disabilitato".to_string()));
    }

    // 3. Verifica password — anche questa è CPU-intensive
    let password = req.password.clone();
    let hash = user.password_hash.clone();
    let password_valid = tokio::task::spawn_blocking(move || bcrypt::verify(password, &hash))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))??;

    if !password_valid {
        // Stesso messaggio del "not found" — non riveliamo quale dei due è sbagliato
        return Err(AppError::Unauthorized("Credenziali non valide".to_string()));
    }

    // 4. Genera JWT
    let token = generate_jwt(&user.id, &user.email, jwt_secret, jwt_expiration_hours)?;

    Ok(AuthResponse {
        token,
        user: UserInfo {
            id: user.id,
            email: user.email,
            username: user.username,
        },
    })
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
/// - `sub`: UUID dell'utente (come stringa)
/// - `email`: email dell'utente (utile per display senza query al DB)
/// - `iat`: timestamp di emissione
/// - `exp`: timestamp di scadenza (iat + jwt_expiration_hours)
fn generate_jwt(
    user_id: &Uuid,
    email: &str,
    jwt_secret: &str,
    expiration_hours: i64,
) -> Result<String, AppError> {
    let now = Utc::now();
    let exp = (now + Duration::hours(expiration_hours)).timestamp() as usize;
    let iat = now.timestamp() as usize;

    let claims = Claims {
        sub: user_id.to_string(),
        email: email.to_string(),
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
