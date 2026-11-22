/// dto/auth.rs — DTO per l'autenticazione
///
/// I DTO (Data Transfer Objects) definiscono la "forma" dei dati
/// che entrano ed escono dalla API. Sono SEPARATI dai models perché:
/// - Il client non deve mai vedere `password_hash`
/// - Le validazioni di input sono diverse da quelle del DB
/// - Possiamo cambiare il DB senza cambiare il contratto API
use serde::{Deserialize, Serialize};

use crate::models::user::Role;

/// Dati ricevuti dal client per il login.
/// `#[derive(Deserialize)]` permette ad axum di costruire questo struct
/// automaticamente dal body JSON della request.
#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

/// Dati ricevuti dal client per la registrazione.
/// Un ruolo sconosciuto (o assente) fa fallire la deserializzazione
/// → `400 VALIDATION_FAILED` tramite `AppJson`.
#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    pub username: String,
    pub password: String,
    pub role: Role,
}

/// Risposta restituita al client dopo login/registrazione.
/// Contiene il token JWT e le info base dell'utente.
/// NON contiene password_hash — mai.
#[derive(Debug, Serialize)]
pub struct AuthResponse {
    pub token: String,
    pub user: UserInfo,
}

/// Info pubbliche dell'utente da includere nella risposta auth.
#[derive(Debug, Serialize)]
pub struct UserInfo {
    pub id: i64,
    pub email: String,
    pub username: String,
    pub role: Role,
}

/// Struct che rappresenta i claims dentro il JWT.
/// Viene serializzata/deserializzata da `jsonwebtoken`.
/// - `sub` (subject): identificatore univoco dell'utente
/// - `role`: ruolo (`athlete` / `coach`), letto da `AuthUser`
/// - `exp` (expiration): timestamp Unix di scadenza
/// - `iat` (issued at): timestamp Unix di emissione
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: i64, // ID intero dell'utente
    pub email: String,
    pub role: String,
    pub exp: usize,
    pub iat: usize,
}
