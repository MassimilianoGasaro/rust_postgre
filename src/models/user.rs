/// models/user.rs — Il Model dell'utente
///
/// I models rispecchiano ESATTAMENTE le colonne delle tabelle nel database.
/// sqlx li usa per mappare automaticamente le righe del DB in struct Rust.
/// NON esporre mai direttamente i models nelle risposte HTTP (c'è il DTO per quello):
/// ad esempio, `password_hash` non deve mai uscire nelle risposte API!

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Mappa la tabella `users` nel database.
/// `#[derive(sqlx::FromRow)]` permette a sqlx di costruire questo struct
/// automaticamente dai risultati delle query.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct User {
    pub id: i64,
    pub email: String,
    pub username: String,

    /// ATTENZIONE: questo campo non viene mai serializzato nelle risposte API.
    /// Contiene l'hash bcrypt della password, mai la password in chiaro.
    #[serde(skip_serializing)]
    pub password_hash: String,

    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Mappa la tabella `users_info` nel database.
/// `user_id` è sia PRIMARY KEY che FOREIGN KEY → non possono esistere righe orfane.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct UserInfo {
    /// FK → users.id (1-a-1)
    pub user_id: i64,
    pub bio: String,
}

/// Mappa la tabella `users_config` nel database.
/// Relazione 1-a-1 con `users`: ogni utente ha esattamente una configurazione.
/// `user_id` è sia PRIMARY KEY che FOREIGN KEY → non possono esistere righe orfane.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct UserConfig {
    /// FK → users.id (1-a-1)
    pub user_id: i64,
    pub can_edit: bool,
}

/// Risultato di una JOIN tra `users` e `users_info`.
/// Usata quando serve l'utente insieme al suo profilo in un'unica query.
/// Non mappa direttamente una tabella — è uno "view model" da query.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct UserWithInfo {
    // ── Campi da users ──────────────────────────────────────────────────────
    pub id: i64,
    pub email: String,
    pub username: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    // ── Campi da users_info (NULL se il profilo non esiste ancora) ───────────
    pub bio: Option<String>,
}


