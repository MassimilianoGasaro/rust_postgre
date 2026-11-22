/// services/coach_link_service.rs — Logica di business dei link allenatore-atleta
///
/// Qui vivono le regole: chi può fare cosa e quali transizioni di stato
/// sono ammesse. Non conosce HTTP.
///
/// Un link che non riguarda l'utente — oppure su cui l'utente non è l'attore
/// designato dell'azione — risponde sempre `404 NOT_FOUND`: così non si può
/// scoprire l'esistenza di link altrui.
use axum::http::StatusCode;
use sqlx::PgPool;

use crate::dto::coach_link::{CoachLinkDto, LinkedAthleteDto};
use crate::errors::AppError;
use crate::middleware::AuthUser;
use crate::models::coach_link::{CoachLink, LinkStatus};
use crate::models::user::Role;
use crate::repositories::{coach_link_repository, user_repository};

/// Azione richiesta su un link esistente.
#[derive(Debug, Clone, Copy)]
pub enum LinkAction {
    /// Il coach accetta: pending → active
    Accept,
    /// Il coach rifiuta: pending → rejected
    Reject,
    /// L'atleta annulla la richiesta: pending → cancelled
    Cancel,
    /// L'atleta revoca il coach: active → revoked
    Revoke,
}

impl LinkAction {
    /// Ruolo dell'unico attore che può eseguire l'azione.
    fn actor(&self) -> Role {
        match self {
            LinkAction::Accept | LinkAction::Reject => Role::Coach,
            LinkAction::Cancel | LinkAction::Revoke => Role::Athlete,
        }
    }

    /// (stato di partenza, stato di arrivo, `true` se valorizza `responded_at`).
    fn transition(&self) -> (LinkStatus, LinkStatus, bool) {
        match self {
            LinkAction::Accept => (LinkStatus::Pending, LinkStatus::Active, true),
            LinkAction::Reject => (LinkStatus::Pending, LinkStatus::Rejected, true),
            LinkAction::Cancel => (LinkStatus::Pending, LinkStatus::Cancelled, false),
            LinkAction::Revoke => (LinkStatus::Active, LinkStatus::Revoked, false),
        }
    }
}

/// L'atleta richiede un coach, indicato per email o username.
pub async fn request_coach(
    pool: &PgPool,
    auth: &AuthUser,
    coach: &str,
) -> Result<CoachLinkDto, AppError> {
    auth.require_role(Role::Athlete)?;

    let coach_user = user_repository::find_by_email_or_username(pool, coach.trim())
        .await?
        .ok_or_else(|| {
            AppError::coded(
                StatusCode::NOT_FOUND,
                "COACH_NOT_FOUND",
                "Coach non trovato",
            )
        })?;

    if Role::parse(&coach_user.role) != Some(Role::Coach) {
        return Err(AppError::coded(
            StatusCode::BAD_REQUEST,
            "NOT_A_COACH",
            "L'utente indicato non è un coach",
        ));
    }

    let link_id = coach_link_repository::create(pool, auth.id, coach_user.id).await?;
    let link = coach_link_repository::find_by_id(pool, link_id)
        .await?
        .ok_or_else(|| AppError::Internal("link appena creato non trovato".to_string()))?;

    Ok(link.into())
}

/// Link dell'utente: quelli inviati (atleta) o ricevuti (coach), filtrabili per stato.
/// Uno stato sconosciuto → `400 VALIDATION_FAILED`.
pub async fn list_links(
    pool: &PgPool,
    auth: &AuthUser,
    status: Option<&str>,
) -> Result<Vec<CoachLinkDto>, AppError> {
    let status = match status {
        None => None,
        Some(value) => Some(LinkStatus::parse(value).ok_or_else(|| {
            AppError::coded(
                StatusCode::BAD_REQUEST,
                "VALIDATION_FAILED",
                format!("Stato non valido: '{value}'"),
            )
        })?),
    };

    let links =
        coach_link_repository::list_for_user(pool, auth.id, auth.role == Role::Coach, status)
            .await?;

    Ok(links.into_iter().map(Into::into).collect())
}

/// Esegue un'azione su un link: solo l'attore designato (e parte del link)
/// la vede; la transizione è atomica e fallita → `409 LINK_INVALID_STATE`.
pub async fn act_on_link(
    pool: &PgPool,
    auth: &AuthUser,
    link_id: i64,
    action: LinkAction,
) -> Result<CoachLinkDto, AppError> {
    let link = coach_link_repository::find_by_id(pool, link_id)
        .await?
        .filter(|l| is_designated_actor(l, auth, action))
        .ok_or_else(|| AppError::NotFound("Link non trovato".to_string()))?;

    let (from, to, is_response) = action.transition();
    if !coach_link_repository::transition(pool, link.id, from, to, is_response).await? {
        return Err(AppError::coded(
            StatusCode::CONFLICT,
            "LINK_INVALID_STATE",
            "Transizione non ammessa dallo stato attuale del link",
        ));
    }

    let updated = coach_link_repository::find_by_id(pool, link.id)
        .await?
        .ok_or_else(|| AppError::NotFound("Link non trovato".to_string()))?;

    Ok(updated.into())
}

/// L'utente è la parte del link prevista per l'azione (coach destinatario
/// per accept/reject, atleta richiedente per cancel/revoke)?
fn is_designated_actor(link: &CoachLink, auth: &AuthUser, action: LinkAction) -> bool {
    match action.actor() {
        Role::Coach => auth.role == Role::Coach && link.coach_id == auth.id,
        Role::Athlete => auth.role == Role::Athlete && link.athlete_id == auth.id,
    }
}

/// Atleti collegati (link `active`) al coach autenticato.
pub async fn list_athletes(
    pool: &PgPool,
    auth: &AuthUser,
) -> Result<Vec<LinkedAthleteDto>, AppError> {
    auth.require_role(Role::Coach)?;

    let athletes = coach_link_repository::list_active_athletes(pool, auth.id).await?;
    Ok(athletes.into_iter().map(Into::into).collect())
}
