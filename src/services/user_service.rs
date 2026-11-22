/// services/user_service.rs — Logica di business per l'utente autenticato
///
/// Questo service orchestra le operazioni su `/users/me`:
/// - Lettura di account + profilo
/// - Aggiornamento parziale (con validazioni e re-hash della password)
/// - Eliminazione dell'account e dei file dell'utente
///
/// Principio: il service è l'unico posto dove si prende la "decisione".
/// Il repository esegue solo SQL, il controller gestisce solo HTTP.
use std::path::Path;

use axum::http::StatusCode;
use sqlx::PgPool;

use crate::dto::user::{MeResponse, ProfileDto, UpdateMeRequest};
use crate::errors::AppError;
use crate::models::user::{ProfileUpdate, Role, User};
use crate::repositories::user_repository;
use crate::services::auth_service::{
    hash_password, validate_email, validate_password, validate_username, validation_error,
};

/// Lingue supportate.
const LANGUAGES: [&str; 2] = ["it", "en"];
/// Valori ammessi per `sex` (vincolo CHECK nel DB).
const SEXES: [&str; 3] = ["male", "female", "other"];

/// Carica l'utente del token. Se non esiste più (account eliminato) il token
/// è inutilizzabile → 401.
async fn load_user(pool: &PgPool, user_id: i64) -> Result<User, AppError> {
    user_repository::find_by_id(pool, user_id)
        .await?
        .ok_or_else(|| AppError::Unauthorized("Utente non trovato".to_string()))
}

/// Compone la risposta `MeResponse` rileggendo utente e profilo dal DB.
async fn build_me(pool: &PgPool, user_id: i64) -> Result<MeResponse, AppError> {
    let user = load_user(pool, user_id).await?;
    let profile = user_repository::find_profile(pool, user_id).await?;
    let role = Role::parse(&user.role)
        .ok_or_else(|| AppError::Internal(format!("Ruolo non valido nel DB: {}", user.role)))?;

    Ok(MeResponse {
        id: user.id,
        email: user.email,
        username: user.username,
        role,
        created_at: user.created_at,
        profile: ProfileDto::from(profile),
    })
}

/// Restituisce account e profilo dell'utente autenticato.
pub async fn get_me(pool: &PgPool, user_id: i64) -> Result<MeResponse, AppError> {
    build_me(pool, user_id).await
}

/// Un valore numerico del profilo, se presente, deve essere positivo.
fn check_positive<T: PartialOrd + Default>(
    field: &str,
    value: &Option<Option<T>>,
) -> Result<(), AppError> {
    if let Some(Some(v)) = value {
        if *v <= T::default() {
            return Err(validation_error(format!("{field} deve essere positivo")));
        }
    }
    Ok(())
}

/// Aggiorna account e profilo dell'utente autenticato (PATCH).
///
/// Flusso:
/// 1. Valida i campi inviati (senza toccare il DB, tranne il fuso orario)
/// 2. Se la password cambia, la re-hasha
/// 3. In una transazione aggiorna account e profilo
///    (email/username duplicati → 409 dal vincolo UNIQUE)
/// 4. Restituisce lo stato aggiornato
pub async fn update_me(
    pool: &PgPool,
    user_id: i64,
    req: UpdateMeRequest,
) -> Result<MeResponse, AppError> {
    load_user(pool, user_id).await?;

    // 1. Validazione
    if let Some(email) = &req.email {
        validate_email(email)?;
    }
    if let Some(username) = &req.username {
        validate_username(username)?;
    }
    if let Some(password) = &req.password {
        validate_password(password)?;
    }
    if let Some(language) = &req.language {
        if !LANGUAGES.contains(&language.as_str()) {
            return Err(validation_error(
                "language non valida: valori ammessi 'it', 'en'",
            ));
        }
    }
    if let Some(Some(sex)) = &req.sex {
        if !SEXES.contains(&sex.as_str()) {
            return Err(validation_error(
                "sex non valido: valori ammessi 'male', 'female', 'other'",
            ));
        }
    }
    check_positive("weight_kg", &req.weight_kg)?;
    check_positive("max_hr", &req.max_hr)?;
    check_positive("resting_hr", &req.resting_hr)?;
    check_positive("ftp_watts", &req.ftp_watts)?;
    check_positive("threshold_pace_s_per_km", &req.threshold_pace_s_per_km)?;

    if let Some(timezone) = &req.timezone {
        if !user_repository::timezone_exists(pool, timezone).await? {
            return Err(AppError::coded(
                StatusCode::BAD_REQUEST,
                "INVALID_TIMEZONE",
                format!("Fuso orario '{timezone}' sconosciuto"),
            ));
        }
    }

    // 2. Re-hash della password, se cambia
    let new_password_hash = match req.password {
        Some(password) => Some(hash_password(password).await?),
        None => None,
    };

    // 3. Account e profilo cambiano insieme o per niente
    let account_changes =
        req.email.is_some() || req.username.is_some() || new_password_hash.is_some();
    let profile_update = ProfileUpdate {
        language: req.language,
        timezone: req.timezone,
        birth_date: req.birth_date,
        sex: req.sex,
        weight_kg: req.weight_kg,
        max_hr: req.max_hr,
        resting_hr: req.resting_hr,
        ftp_watts: req.ftp_watts,
        threshold_pace_s_per_km: req.threshold_pace_s_per_km,
    };

    let mut tx = pool.begin().await?;
    if account_changes {
        user_repository::update_account(
            &mut tx,
            user_id,
            req.email.as_deref(),
            req.username.as_deref(),
            new_password_hash.as_deref(),
        )
        .await?;
    }
    user_repository::update_profile(&mut tx, user_id, &profile_update).await?;
    tx.commit().await?;

    // 4. Stato aggiornato
    build_me(pool, user_id).await
}

/// Elimina l'account dell'utente autenticato.
///
/// Prima la riga nel DB (profilo e dati collegati vanno in cascata), poi la
/// cartella `upload_dir/{user_id}` con i suoi file: se non esiste va bene.
pub async fn delete_me(pool: &PgPool, upload_dir: &Path, user_id: i64) -> Result<(), AppError> {
    let rows_affected = user_repository::delete(pool, user_id).await?;
    if rows_affected == 0 {
        return Err(AppError::Unauthorized("Utente non trovato".to_string()));
    }

    match tokio::fs::remove_dir_all(upload_dir.join(user_id.to_string())).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(AppError::Internal(format!(
            "Impossibile eliminare i file dell'utente {user_id}: {e}"
        ))),
    }
}
