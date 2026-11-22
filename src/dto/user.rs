/// dto/user.rs — DTO per il profilo dell'utente autenticato (`/users/me`)
///
/// Separare i DTO utente da quelli auth ci permette di:
/// - Avere risposte diverse per contesti diversi
/// - Controllare esattamente quali campi esporre
/// - Aggiungere validazioni specifiche per ogni operazione

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Deserializer, Serialize};

use crate::models::user::{Role, UserProfile};

/// Profilo fisiologico e preferenze dell'utente.
#[derive(Debug, Serialize)]
pub struct ProfileDto {
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

impl From<UserProfile> for ProfileDto {
    fn from(p: UserProfile) -> Self {
        ProfileDto {
            language: p.language,
            timezone: p.timezone,
            birth_date: p.birth_date,
            sex: p.sex,
            weight_kg: p.weight_kg,
            max_hr: p.max_hr,
            resting_hr: p.resting_hr,
            ftp_watts: p.ftp_watts,
            threshold_pace_s_per_km: p.threshold_pace_s_per_km,
        }
    }
}

/// Risposta di GET/PATCH `/users/me`: dati account + profilo.
/// Notare: nessun campo sensibile (password_hash, ecc.)
#[derive(Debug, Serialize)]
pub struct MeResponse {
    pub id: i64,
    pub email: String,
    pub username: String,
    pub role: Role,
    pub created_at: DateTime<Utc>,
    pub profile: ProfileDto,
}

/// Deserializza un campo "nullable" distinguendo assente da `null`:
/// - assente → `None` (grazie a `#[serde(default)]`): campo invariato
/// - `null`  → `Some(None)`: azzera il valore
/// - valore  → `Some(Some(v))`: imposta il valore
fn double_option<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// Dati per PATCH `/users/me` — tutti i campi sono opzionali:
/// un campo assente resta invariato.
///
/// I campi del profilo che nel DB sono nullable (`Option<Option<T>>`)
/// possono essere azzerati inviando `null`. Gli altri (email, username,
/// password, language, timezone) non sono azzerabili: `null` equivale ad assente.
#[derive(Debug, Deserialize)]
pub struct UpdateMeRequest {
    pub email: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub language: Option<String>,
    pub timezone: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    pub birth_date: Option<Option<NaiveDate>>,
    #[serde(default, deserialize_with = "double_option")]
    pub sex: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub weight_kg: Option<Option<f64>>,
    #[serde(default, deserialize_with = "double_option")]
    pub max_hr: Option<Option<i32>>,
    #[serde(default, deserialize_with = "double_option")]
    pub resting_hr: Option<Option<i32>>,
    #[serde(default, deserialize_with = "double_option")]
    pub ftp_watts: Option<Option<i32>>,
    #[serde(default, deserialize_with = "double_option")]
    pub threshold_pace_s_per_km: Option<Option<i32>>,
}
