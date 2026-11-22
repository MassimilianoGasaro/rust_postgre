/// middleware.rs — Middleware di autenticazione JWT
///
/// In Axum i middleware sono funzioni che wrappano gli handler.
/// Questo middleware:
/// 1. Legge l'header `Authorization: Bearer <token>`
/// 2. Valida il JWT tramite auth_service::validate_jwt
/// 3. Verifica a DB che l'utente esista ancora e sia attivo
/// 4. Se valido: inietta i Claims nell'handler via Extension
/// 5. Altrimenti: blocca la request con 401 Unauthorized
///
/// Gli handler protetti ricevono l'utente come `AuthUser`, ricavato dai claims.
use axum::{
    async_trait,
    extract::{FromRequestParts, Request, State},
    http::{StatusCode, header::AUTHORIZATION, request::Parts},
    middleware::Next,
    response::Response,
};

use crate::AppState;
use crate::dto::auth::Claims;
use crate::errors::AppError;
use crate::models::user::Role;
use crate::services::auth_service;

/// Middleware che verifica il JWT su ogni request alle route protette.
///
/// Axum lo registra con `.route_layer(axum::middleware::from_fn_with_state(...))`
/// applicandolo solo alle route che vogliamo proteggere.
pub async fn auth_middleware(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, AppError> {
    // Legge l'header Authorization
    let auth_header = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized("Header Authorization mancante".to_string()))?;

    // Il formato atteso è "Bearer <token>"
    let token = auth_header.strip_prefix("Bearer ").ok_or_else(|| {
        AppError::Unauthorized("Formato token non valido. Usa: Bearer <token>".to_string())
    })?;

    // Valida il JWT — se scaduto o manomesso → AppError::Unauthorized
    let claims = auth_service::validate_jwt(token, &state.jwt_secret)?;

    // Un token resta valido fino alla scadenza: l'account potrebbe nel
    // frattempo essere stato eliminato o disattivato
    auth_service::ensure_active_user(&state.pool, claims.sub).await?;

    // Inserisce i claims nella request come Extension
    // Gli handler possono estrarli con: Extension(claims): Extension<Claims>
    req.extensions_mut().insert(claims);

    // Passa la request all'handler successivo
    Ok(next.run(req).await)
}

/// Utente autenticato, estratto dai `Claims` iniettati da `auth_middleware`.
///
/// Gli handler lo dichiarano come parametro (`auth: AuthUser`) invece di
/// leggere i claims a mano. Funziona solo su route protette dal middleware:
/// senza claims risponde 401.
#[derive(Debug, Clone, Copy)]
pub struct AuthUser {
    pub id: i64,
    pub role: Role,
}

impl AuthUser {
    /// Verifica che l'utente abbia il ruolo richiesto, altrimenti `403 ROLE_FORBIDDEN`.
    pub fn require_role(&self, role: Role) -> Result<(), AppError> {
        if self.role == role {
            Ok(())
        } else {
            Err(AppError::coded(
                StatusCode::FORBIDDEN,
                "ROLE_FORBIDDEN",
                format!("Operazione riservata al ruolo '{}'", role.as_str()),
            ))
        }
    }
}

#[async_trait]
impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let claims = parts
            .extensions
            .get::<Claims>()
            .ok_or_else(|| AppError::Unauthorized("Autenticazione richiesta".to_string()))?;

        let role = Role::parse(&claims.role)
            .ok_or_else(|| AppError::Unauthorized("Ruolo nel token non valido".to_string()))?;

        Ok(AuthUser {
            id: claims.sub,
            role,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn require_role_allows_matching_role_only() {
        let coach = AuthUser {
            id: 1,
            role: Role::Coach,
        };
        assert!(coach.require_role(Role::Coach).is_ok());

        match coach.require_role(Role::Athlete) {
            Err(AppError::Coded { status, code, .. }) => {
                assert_eq!(status, StatusCode::FORBIDDEN);
                assert_eq!(code, "ROLE_FORBIDDEN");
            }
            other => panic!("atteso ROLE_FORBIDDEN, ottenuto {other:?}"),
        }
    }
}
