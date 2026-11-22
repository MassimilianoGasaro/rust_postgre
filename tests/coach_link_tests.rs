mod common;

use axum::http::{Method, StatusCode};
use rust_postgre::errors::AppError;
use rust_postgre::middleware::AuthUser;
use rust_postgre::models::user::Role;
use rust_postgre::services::access_service;
use serde_json::{Value, json};
use sqlx::PgPool;

use common::TestApp;

/// L'atleta invia la richiesta al coach indicato (email o username).
async fn request_link(app: &TestApp, token: &str, coach: &str) -> (StatusCode, Value) {
    app.request(
        Method::POST,
        "/api/coach-links",
        Some(token),
        Some(json!({ "coach": coach })),
    )
    .await
}

/// Esegue un'azione (`accept`, `reject`, `cancel`, `revoke`) su un link.
async fn act(app: &TestApp, token: &str, link_id: i64, action: &str) -> (StatusCode, Value) {
    app.request(
        Method::POST,
        &format!("/api/coach-links/{link_id}/{action}"),
        Some(token),
        None,
    )
    .await
}

/// Crea un link e lo porta ad `active`; ritorna l'id del link.
async fn active_link(app: &TestApp, athlete_token: &str, coach_token: &str, coach: &str) -> i64 {
    let (status, body) = request_link(app, athlete_token, coach).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let id = body["id"].as_i64().unwrap();
    let (status, body) = act(app, coach_token, id, "accept").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    id
}

fn viewer(id: i64, role: Role) -> AuthUser {
    AuthUser { id, role }
}

fn assert_denied(result: Result<i64, AppError>) {
    match result {
        Err(AppError::Coded { status, code, .. }) => {
            assert_eq!(status, StatusCode::FORBIDDEN);
            assert_eq!(code, "ATHLETE_ACCESS_DENIED");
        }
        other => panic!("atteso ATHLETE_ACCESS_DENIED, ottenuto {other:?}"),
    }
}

// ── Route HTTP ───────────────────────────────────────────────────────────

#[sqlx::test]
async fn athlete_requests_coach_by_username(pool: PgPool) {
    let app = common::test_app(pool);
    let (athlete_id, athlete) = app.register("a@example.com", "atleta", "athlete").await;
    let (coach_id, _) = app.register("c@example.com", "allenatore", "coach").await;

    let (status, body) = request_link(&app, &athlete, "allenatore").await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert!(body["id"].is_i64());
    assert_eq!(body["status"], "pending");
    assert_eq!(
        body["athlete"],
        json!({"id": athlete_id, "username": "atleta"})
    );
    assert_eq!(
        body["coach"],
        json!({"id": coach_id, "username": "allenatore"})
    );
    assert!(body["requested_at"].is_string());
    assert!(body["responded_at"].is_null());
    assert!(body["ended_at"].is_null());
}

#[sqlx::test]
async fn athlete_requests_coach_by_email(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, athlete) = app.register("a@example.com", "atleta", "athlete").await;
    let (coach_id, _) = app.register("c@example.com", "allenatore", "coach").await;

    let (status, body) = request_link(&app, &athlete, "c@example.com").await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["coach"]["id"], coach_id);
    assert_eq!(body["status"], "pending");
}

#[sqlx::test]
async fn coach_cannot_create_request(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, coach) = app.register("c@example.com", "allenatore", "coach").await;
    app.register("c2@example.com", "altro", "coach").await;

    let (status, body) = request_link(&app, &coach, "altro").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["code"], "ROLE_FORBIDDEN");
}

#[sqlx::test]
async fn request_to_non_coach_is_400_not_a_coach(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, athlete) = app.register("a@example.com", "atleta", "athlete").await;
    app.register("b@example.com", "altro", "athlete").await;

    let (status, body) = request_link(&app, &athlete, "altro").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "NOT_A_COACH");
}

#[sqlx::test]
async fn request_to_unknown_is_404_coach_not_found(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, athlete) = app.register("a@example.com", "atleta", "athlete").await;

    let (status, body) = request_link(&app, &athlete, "nessuno").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "COACH_NOT_FOUND");
}

#[sqlx::test]
async fn second_request_while_pending_is_409(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, athlete) = app.register("a@example.com", "atleta", "athlete").await;
    app.register("c@example.com", "allenatore", "coach").await;
    app.register("c2@example.com", "secondo", "coach").await;

    let (status, _) = request_link(&app, &athlete, "allenatore").await;
    assert_eq!(status, StatusCode::CREATED);

    // Stesso coach e coach diverso: un solo link aperto per atleta.
    for coach in ["allenatore", "secondo"] {
        let (status, body) = request_link(&app, &athlete, coach).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["code"], "COACH_LINK_EXISTS");
    }
}

#[sqlx::test]
async fn second_request_while_active_is_409(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, athlete) = app.register("a@example.com", "atleta", "athlete").await;
    let (_, coach) = app.register("c@example.com", "allenatore", "coach").await;
    active_link(&app, &athlete, &coach, "allenatore").await;

    let (status, body) = request_link(&app, &athlete, "allenatore").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "COACH_LINK_EXISTS");
}

#[sqlx::test]
async fn concurrent_requests_yield_a_single_link(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, athlete) = app.register("a@example.com", "atleta", "athlete").await;
    app.register("c@example.com", "allenatore", "coach").await;

    let (r1, r2) = tokio::join!(
        request_link(&app, &athlete, "allenatore"),
        request_link(&app, &athlete, "allenatore")
    );
    let mut statuses = [r1.0, r2.0];
    statuses.sort();
    assert_eq!(statuses, [StatusCode::CREATED, StatusCode::CONFLICT]);
    let loser = if r1.0 == StatusCode::CONFLICT {
        &r1.1
    } else {
        &r2.1
    };
    assert_eq!(loser["code"], "COACH_LINK_EXISTS");
}

#[sqlx::test]
async fn coach_accepts_then_athlete_appears_in_athletes(pool: PgPool) {
    let app = common::test_app(pool);
    let (athlete_id, athlete) = app.register("a@example.com", "atleta", "athlete").await;
    let (_, coach) = app.register("c@example.com", "allenatore", "coach").await;

    let (_, created) = request_link(&app, &athlete, "allenatore").await;
    let link_id = created["id"].as_i64().unwrap();

    // Finché è pending l'atleta non compare tra gli atleti del coach.
    let (status, body) = app
        .request(Method::GET, "/api/athletes", Some(&coach), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!([]));

    let (status, body) = act(&app, &coach, link_id, "accept").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "active");
    assert!(body["responded_at"].is_string());
    assert!(body["ended_at"].is_null());

    let (status, body) = app
        .request(Method::GET, "/api/athletes", Some(&coach), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    let list = body.as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["id"], athlete_id);
    assert_eq!(list[0]["username"], "atleta");
    assert!(list[0]["linked_since"].is_string());
}

#[sqlx::test]
async fn athletes_endpoint_is_coach_only(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, athlete) = app.register("a@example.com", "atleta", "athlete").await;

    let (status, body) = app
        .request(Method::GET, "/api/athletes", Some(&athlete), None)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["code"], "ROLE_FORBIDDEN");

    let (status, _) = app.request(Method::GET, "/api/athletes", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn coach_rejects(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, athlete) = app.register("a@example.com", "atleta", "athlete").await;
    let (_, coach) = app.register("c@example.com", "allenatore", "coach").await;

    let (_, created) = request_link(&app, &athlete, "allenatore").await;
    let link_id = created["id"].as_i64().unwrap();

    let (status, body) = act(&app, &coach, link_id, "reject").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "rejected");
    assert!(body["responded_at"].is_string());

    // Dopo il rifiuto l'atleta può fare una nuova richiesta.
    let (status, _) = request_link(&app, &athlete, "allenatore").await;
    assert_eq!(status, StatusCode::CREATED);
}

#[sqlx::test]
async fn athlete_cancels_pending(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, athlete) = app.register("a@example.com", "atleta", "athlete").await;
    let (_, coach) = app.register("c@example.com", "allenatore", "coach").await;

    let (_, created) = request_link(&app, &athlete, "allenatore").await;
    let link_id = created["id"].as_i64().unwrap();

    let (status, body) = act(&app, &athlete, link_id, "cancel").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "cancelled");
    assert!(body["ended_at"].is_string());

    // Il coach non può più accettare.
    let (status, body) = act(&app, &coach, link_id, "accept").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "LINK_INVALID_STATE");
}

#[sqlx::test]
async fn athlete_revokes_active_then_can_request_again(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, athlete) = app.register("a@example.com", "atleta", "athlete").await;
    let (_, coach) = app.register("c@example.com", "allenatore", "coach").await;
    let link_id = active_link(&app, &athlete, &coach, "allenatore").await;

    let (status, body) = act(&app, &athlete, link_id, "revoke").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "revoked");
    assert!(body["ended_at"].is_string());

    let (_, list) = app
        .request(Method::GET, "/api/athletes", Some(&coach), None)
        .await;
    assert_eq!(list, json!([]));

    let (status, body) = request_link(&app, &athlete, "allenatore").await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

#[sqlx::test]
async fn accept_twice_is_409_link_invalid_state(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, athlete) = app.register("a@example.com", "atleta", "athlete").await;
    let (_, coach) = app.register("c@example.com", "allenatore", "coach").await;
    let link_id = active_link(&app, &athlete, &coach, "allenatore").await;

    let (status, body) = act(&app, &coach, link_id, "accept").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "LINK_INVALID_STATE");
}

#[sqlx::test]
async fn invalid_transitions_are_409(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, athlete) = app.register("a@example.com", "atleta", "athlete").await;
    let (_, coach) = app.register("c@example.com", "allenatore", "coach").await;

    // pending: non si può revocare.
    let (_, created) = request_link(&app, &athlete, "allenatore").await;
    let link_id = created["id"].as_i64().unwrap();
    let (status, body) = act(&app, &athlete, link_id, "revoke").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "LINK_INVALID_STATE");

    // active: non si può annullare né rifiutare.
    act(&app, &coach, link_id, "accept").await;
    let (status, body) = act(&app, &athlete, link_id, "cancel").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "LINK_INVALID_STATE");
    let (status, body) = act(&app, &coach, link_id, "reject").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "LINK_INVALID_STATE");
}

#[sqlx::test]
async fn other_coach_cannot_accept_is_404(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, athlete) = app.register("a@example.com", "atleta", "athlete").await;
    app.register("c@example.com", "allenatore", "coach").await;
    let (_, other_coach) = app.register("c2@example.com", "altro", "coach").await;

    let (_, created) = request_link(&app, &athlete, "allenatore").await;
    let link_id = created["id"].as_i64().unwrap();

    for action in ["accept", "reject"] {
        let (status, body) = act(&app, &other_coach, link_id, action).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["code"], "NOT_FOUND");
    }
}

#[sqlx::test]
async fn non_designated_actor_gets_404(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, athlete) = app.register("a@example.com", "atleta", "athlete").await;
    let (_, other_athlete) = app.register("b@example.com", "altro", "athlete").await;
    let (_, coach) = app.register("c@example.com", "allenatore", "coach").await;

    let (_, created) = request_link(&app, &athlete, "allenatore").await;
    let link_id = created["id"].as_i64().unwrap();

    // Parte del link ma non l'attore designato dell'azione.
    for (token, action) in [
        (&athlete, "accept"),
        (&athlete, "reject"),
        (&coach, "cancel"),
    ] {
        let (status, body) = act(&app, token, link_id, action).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{action}");
        assert_eq!(body["code"], "NOT_FOUND");
    }

    // Estraneo.
    let (status, body) = act(&app, &other_athlete, link_id, "cancel").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "NOT_FOUND");

    // Il link non è stato toccato.
    let (_, list) = app
        .request(Method::GET, "/api/coach-links", Some(&athlete), None)
        .await;
    assert_eq!(list[0]["status"], "pending");
}

#[sqlx::test]
async fn unknown_link_is_404(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, coach) = app.register("c@example.com", "allenatore", "coach").await;

    let (status, body) = act(&app, &coach, 9999, "accept").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "NOT_FOUND");
}

#[sqlx::test]
async fn list_links_by_role_and_status(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, athlete) = app.register("a@example.com", "atleta", "athlete").await;
    let (_, athlete2) = app.register("b@example.com", "atleta2", "athlete").await;
    let (_, coach) = app.register("c@example.com", "allenatore", "coach").await;
    let (_, coach2) = app.register("c2@example.com", "altro", "coach").await;

    // atleta: link 1 rifiutato, poi link 2 pending; atleta2 → coach2.
    let (_, first) = request_link(&app, &athlete, "allenatore").await;
    let first_id = first["id"].as_i64().unwrap();
    act(&app, &coach, first_id, "reject").await;
    let (_, second) = request_link(&app, &athlete, "allenatore").await;
    let second_id = second["id"].as_i64().unwrap();
    request_link(&app, &athlete2, "altro").await;

    // Atleta: solo i propri, requested_at DESC.
    let (status, body) = app
        .request(Method::GET, "/api/coach-links", Some(&athlete), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    let ids: Vec<i64> = body
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["id"].as_i64().unwrap())
        .collect();
    assert_eq!(ids, vec![second_id, first_id]);

    // Coach: solo quelli verso di sé.
    let (_, body) = app
        .request(Method::GET, "/api/coach-links", Some(&coach), None)
        .await;
    assert_eq!(body.as_array().unwrap().len(), 2);
    let (_, body) = app
        .request(Method::GET, "/api/coach-links", Some(&coach2), None)
        .await;
    assert_eq!(body.as_array().unwrap().len(), 1);
    assert_eq!(body[0]["athlete"]["username"], "atleta2");

    // Filtro per stato.
    let (status, body) = app
        .request(
            Method::GET,
            "/api/coach-links?status=pending",
            Some(&athlete),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().unwrap().len(), 1);
    assert_eq!(body[0]["id"], second_id);

    // Stato sconosciuto → 400.
    let (status, body) = app
        .request(
            Method::GET,
            "/api/coach-links?status=bogus",
            Some(&athlete),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "VALIDATION_FAILED");
}

#[sqlx::test]
async fn coach_links_require_token(pool: PgPool) {
    let app = common::test_app(pool);
    let (status, _) = app
        .request(Method::GET, "/api/coach-links", None, None)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = app
        .request(
            Method::POST,
            "/api/coach-links",
            None,
            Some(json!({"coach": "x"})),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn malformed_body_is_400_validation_failed(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, athlete) = app.register("a@example.com", "atleta", "athlete").await;

    let (status, body) = app
        .request(
            Method::POST,
            "/api/coach-links",
            Some(&athlete),
            Some(json!({"wrong": 1})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "VALIDATION_FAILED");
}

// ── access_service::resolve_target ───────────────────────────────────────

#[sqlx::test]
async fn resolve_own_id_ok(pool: PgPool) {
    let app = common::test_app(pool.clone());
    let (athlete_id, _) = app.register("a@example.com", "atleta", "athlete").await;
    let me = viewer(athlete_id, Role::Athlete);

    assert_eq!(
        access_service::resolve_target(&pool, &me, None)
            .await
            .unwrap(),
        athlete_id
    );
    assert_eq!(
        access_service::resolve_target(&pool, &me, Some(athlete_id))
            .await
            .unwrap(),
        athlete_id
    );
}

#[sqlx::test]
async fn resolve_coach_active_ok(pool: PgPool) {
    let app = common::test_app(pool.clone());
    let (athlete_id, athlete) = app.register("a@example.com", "atleta", "athlete").await;
    let (coach_id, coach) = app.register("c@example.com", "allenatore", "coach").await;
    active_link(&app, &athlete, &coach, "allenatore").await;

    let me = viewer(coach_id, Role::Coach);
    assert_eq!(
        access_service::resolve_target(&pool, &me, Some(athlete_id))
            .await
            .unwrap(),
        athlete_id
    );
    // Senza athlete_id il coach vede i propri dati.
    assert_eq!(
        access_service::resolve_target(&pool, &me, None)
            .await
            .unwrap(),
        coach_id
    );
}

#[sqlx::test]
async fn resolve_coach_pending_denied(pool: PgPool) {
    let app = common::test_app(pool.clone());
    let (athlete_id, athlete) = app.register("a@example.com", "atleta", "athlete").await;
    let (coach_id, _) = app.register("c@example.com", "allenatore", "coach").await;
    request_link(&app, &athlete, "allenatore").await;

    let me = viewer(coach_id, Role::Coach);
    assert_denied(access_service::resolve_target(&pool, &me, Some(athlete_id)).await);
}

#[sqlx::test]
async fn resolve_coach_without_link_denied(pool: PgPool) {
    let app = common::test_app(pool.clone());
    let (athlete_id, athlete) = app.register("a@example.com", "atleta", "athlete").await;
    let (_, linked_coach) = app.register("c@example.com", "allenatore", "coach").await;
    let (other_coach_id, _) = app.register("c2@example.com", "altro", "coach").await;
    active_link(&app, &athlete, &linked_coach, "allenatore").await;

    // Il coach collegato è "allenatore"; "altro" non ha alcun link → negato.
    let me = viewer(other_coach_id, Role::Coach);
    assert_denied(access_service::resolve_target(&pool, &me, Some(athlete_id)).await);
    // Atleta inesistente → negato (nessuna differenza osservabile).
    assert_denied(access_service::resolve_target(&pool, &me, Some(999_999)).await);
}

#[sqlx::test]
async fn resolve_athlete_on_other_athlete_denied(pool: PgPool) {
    let app = common::test_app(pool.clone());
    let (a_id, _) = app.register("a@example.com", "atleta", "athlete").await;
    let (b_id, _) = app.register("b@example.com", "altro", "athlete").await;

    let me = viewer(a_id, Role::Athlete);
    assert_denied(access_service::resolve_target(&pool, &me, Some(b_id)).await);
}

#[sqlx::test]
async fn coach_loses_access_after_revoke(pool: PgPool) {
    let app = common::test_app(pool.clone());
    let (athlete_id, athlete) = app.register("a@example.com", "atleta", "athlete").await;
    let (coach_id, coach) = app.register("c@example.com", "allenatore", "coach").await;
    let link_id = active_link(&app, &athlete, &coach, "allenatore").await;
    let me = viewer(coach_id, Role::Coach);

    assert_eq!(
        access_service::resolve_target(&pool, &me, Some(athlete_id))
            .await
            .unwrap(),
        athlete_id
    );

    let (status, _) = act(&app, &athlete, link_id, "revoke").await;
    assert_eq!(status, StatusCode::OK);

    assert_denied(access_service::resolve_target(&pool, &me, Some(athlete_id)).await);
}

// ── Parametri di path/query malformati ───────────────────────────────────

#[sqlx::test]
async fn malformed_path_id_is_400_validation_failed(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, coach) = app.register("c@example.com", "allenatore", "coach").await;

    for id in ["abc", "99999999999999999999"] {
        let (status, body) = app
            .request(
                Method::POST,
                &format!("/api/coach-links/{id}/accept"),
                Some(&coach),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "id={id}");
        assert_eq!(body["code"], "VALIDATION_FAILED");
        assert_eq!(body["status"], 400);
    }
}

#[sqlx::test]
async fn malformed_query_is_400_validation_failed(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, athlete) = app.register("a@example.com", "atleta", "athlete").await;

    // Parametro ripetuto: non deserializzabile in `Option<String>`.
    let (status, body) = app
        .request(
            Method::GET,
            "/api/coach-links?status=pending&status=active",
            Some(&athlete),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "VALIDATION_FAILED");
    assert_eq!(body["status"], 400);
}
