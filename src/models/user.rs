/// models/user.rs — Il Model dell'utente
///
/// I models rispecchiano ESATTAMENTE le colonne delle tabelle nel database.
/// sqlx li usa per mappare automaticamente le righe del DB in struct Rust.
/// NON esporre mai direttamente i models nelle risposte HTTP (c'è il DTO per quello):
/// ad esempio, `password_hash` non deve mai uscire nelle risposte API!
use chrono::{DateTime, NaiveDate, Utc};
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

    /// Ruolo: `athlete` oppure `coach` (vincolo CHECK nel DB).
    pub role: String,

    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Ruolo dell'utente: un solo ruolo per account.
/// In JSON e nel DB è la stringa minuscola (`athlete` / `coach`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Athlete,
    Coach,
}

impl Role {
    /// Valore testuale usato nel DB (vincolo CHECK) e nei claims JWT.
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Athlete => "athlete",
            Role::Coach => "coach",
        }
    }

    /// Operazione inversa di `as_str`; `None` se il valore non è un ruolo noto.
    pub fn parse(value: &str) -> Option<Role> {
        match value {
            "athlete" => Some(Role::Athlete),
            "coach" => Some(Role::Coach),
            _ => None,
        }
    }
}

/// Mappa la tabella `user_profiles` (1-a-1 con `users`).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UserProfile {
    pub language: String,
    pub timezone: String,
    pub birth_date: Option<NaiveDate>,
    pub sex: Option<String>,
    pub weight_kg: Option<f64>,
    pub max_hr: Option<i32>,
    pub resting_hr: Option<i32>,
    pub ftp_watts: Option<i32>,
    pub threshold_pace_s_per_km: Option<i32>,
}

/// Modifiche al profilo (PATCH): `None` = campo invariato.
/// Per i campi nullable `Some(None)` azzera il valore, `Some(Some(v))` lo imposta.
#[derive(Debug, Default)]
pub struct ProfileUpdate {
    pub language: Option<String>,
    pub timezone: Option<String>,
    pub birth_date: Option<Option<NaiveDate>>,
    pub sex: Option<Option<String>>,
    pub weight_kg: Option<Option<f64>>,
    pub max_hr: Option<Option<i32>>,
    pub resting_hr: Option<Option<i32>>,
    pub ftp_watts: Option<Option<i32>>,
    pub threshold_pace_s_per_km: Option<Option<i32>>,
}
