/// dto/user.rs — DTO per la gestione utenti
///
/// Separare i DTO utente da quelli auth ci permette di:
/// - Avere risposte diverse per contesti diversi
/// - Controllare esattamente quali campi esporre
/// - Aggiungere validazioni specifiche per ogni operazione

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Risposta standard per un utente — usata in GET /users e GET /users/:id.
/// Notare: nessun campo sensibile (password_hash, ecc.)
#[derive(Debug, Serialize)]
pub struct UserResponse {
    pub id: Uuid,
    pub email: String,
    pub username: String,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Dati per aggiornare un utente — tutti i campi sono `Option` perché
/// in un PATCH non è obbligatorio mandare tutto, solo ciò che cambia.
#[derive(Debug, Deserialize)]
pub struct UpdateUserRequest {
    pub email: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
}

/// DTO per la lista paginata di utenti.
#[derive(Debug, Serialize)]
pub struct UserListResponse {
    pub users: Vec<UserResponse>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
}

/// Query params per la paginazione: GET /users?page=1&per_page=20
#[derive(Debug, Deserialize)]
pub struct PaginationQuery {
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}
