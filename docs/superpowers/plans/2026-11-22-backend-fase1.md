# Backend Fase 1 — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Trasformare l'API esistente nella base della piattaforma di allenamento: ruoli atleta/allenatore, relazione allenatore-atleta, import di attività da file FIT/GPX/TCX/ZIP, elenco attività e statistiche per intervallo di date.

**Architecture:** Si mantiene l'architettura a strati esistente (`controllers` → `services` → `repositories` → `models`, `dto`, `errors`). `main.rs` viene ridotto e la logica del server passa in `src/lib.rs`, così i test di integrazione in `tests/` costruiscono il router in-process. Il parsing dei file vive in un modulo puro `src/parsing/` senza dipendenze dal DB; l'import gira in un task tokio in background e scrive lo stato su DB.

**Tech Stack:** Rust 2024, axum 0.7 (feature `multipart`), sqlx 0.8 (Postgres, macro `query!`/`query_as!`, `migrate`), tower-http (CORS), jsonwebtoken, bcrypt, `fitparser`, `quick-xml`, `zip`, `sha2`, `tempfile` (dev), `tower` `ServiceExt` (dev), `http-body-util` (dev).

**Spec:** `docs/superpowers/specs/2026-11-22-piattaforma-allenamento-fase1-design.md`

## Global Constraints

- Mantenere lo stile del codice esistente: identificatori in inglese, **commenti e doc comment in italiano**, un file per responsabilità.
- Tutte le route sono montate sotto il prefisso **`/api`** (il proxy del client inoltra `/api` → `https://localhost:3000`).
- Le macro `sqlx::query!` verificano le query sul DB a compile time: dopo ogni nuova migration eseguire `sqlx migrate run` prima di `cargo build`/`cargo test`. Prerequisiti una tantum: `docker compose up -d`, `cargo install sqlx-cli --no-default-features --features rustls,postgres`.
- Il DB di sviluppo può essere svuotato (`docker compose down -v`) — **deciso** dal proprietario.
- Formato errore JSON per **tutte** le risposte di errore: `{"error": "<messaggio>", "code": "<CODICE>", "status": <n>}`.
- Ruoli: `athlete`, `coach` (un solo ruolo per account). Fuso default `Europe/Rome`. Lingue `it`, `en`.
- Duplicato temporale: stesso utente, stesso sport normalizzato, inizio entro **±60 s**.
- Limiti configurabili da env con default: `MAX_UPLOAD_BYTES=1073741824` (1 GiB, corpo multipart totale), `MAX_FILE_BYTES=52428800` (50 MiB per file, anche dentro uno ZIP); archivio: max 5000 voci e 2 GiB non compressi totali.
- I file reali in `docs/files/` sono dati personali (contengono posizioni GPS): **non vanno mai committati**; `docs/files/` e `uploads/` vanno in `.gitignore`.
- Nessun segreto nel repo: `.env` non tracciato, `.env.example` sì.

## Review Focus

1. **Export completo di Garmin Connect**: ZIP che contiene altri ZIP (`.../DI-Connect-Uploaded-Files/UploadedFiles_0-_Part1.zip`) e molti file non di attività (json, csv). Atteso: si espande **un livello** di ZIP annidati, i file non supportati sono ignorati senza errore, le attività vengono importate. → test `import_nested_zip_one_level` (Task 6).
2. **Stessa attività da due formati** (es. `25143684168_ACTIVITY.fit` nello ZIP e `activity_25143684168.tcx`): una importata, l'altra `duplicate` con `activity_id` dell'esistente. → `same_activity_fit_and_tcx_is_duplicate` (Task 7, sintetico) + `real_garmin_zip_fit_matches_tcx` (Task 5) + prova manuale (Task 10).
3. **Upload grande** (TCX reale da 7,7 MB, export da centinaia di MB): il limite di default di axum (2 MB) non deve scattare; il corpo va scritto su disco in streaming. → `upload_larger_than_axum_default_limit` (Task 7).
4. **Attività vicino alla mezzanotte locale**: aggregata nel giorno del fuso dell'atleta, non in UTC. → `stats_bucket_uses_athlete_timezone` (Task 9).
5. **Allenatore con relazione revocata**: deve ricevere 403 su attività e statistiche. → `coach_loses_access_after_revoke` (Task 3).

---

## File Structure

```
src/
  main.rs                 — solo bootstrap: env, tracing, config, pool, migrate, fail_interrupted, TLS, serve
  lib.rs                  — dichiarazione moduli, AppState, build_router
  config.rs               — + CORS_ORIGIN, UPLOAD_DIR, MAX_UPLOAD_BYTES, MAX_FILE_BYTES
  errors.rs               — AppError con codici
  middleware.rs           — auth_middleware + estrattore AuthUser
  storage.rs              — salvataggio/cancellazione file originali
  models/       user.rs coach_link.rs activity.rs import.rs
  dto/          auth.rs user.rs coach_link.rs activity.rs import.rs stats.rs
  repositories/ user_repository.rs coach_link_repository.rs activity_repository.rs import_repository.rs stats_repository.rs
  services/     auth_service.rs user_service.rs coach_link_service.rs access_service.rs import_service.rs stats_service.rs
  controllers/  health_controller.rs auth_controller.rs user_controller.rs coach_link_controller.rs import_controller.rs activity_controller.rs stats_controller.rs
  parsing/      mod.rs metrics.rs gpx.rs tcx.rs fit.rs archive.rs
migrations/ 0001_users.sql 0002_coach_links.sql 0003_activities_imports.sql
tests/
  common/mod.rs           — TestApp
  fixtures/               — run.gpx ride.tcx ride_same_as_tcx.gpx malformed.fit
  health_tests.rs auth_tests.rs user_tests.rs coach_link_tests.rs import_tests.rs activity_tests.rs stats_tests.rs
  real_files.rs           — test #[ignore] sui file in docs/files
```

---

### Task 1: Fondamenta — lib, migration azzerate, errori con codice, config, CORS, harness di test

**Files:**
- Create: `src/lib.rs`, `src/controllers/health_controller.rs`, `migrations/0001_users.sql`, `.env.example`, `tests/common/mod.rs`, `tests/health_tests.rs`
- Delete: `migrations/001_create_users.sql`, `migrations/002_create_users_info.sql`, `migrations/003_create_users_config.sql`
- Modify: `src/main.rs`, `src/config.rs`, `src/errors.rs`, `Cargo.toml`, `.gitignore`, `src/models/user.rs`, `src/repositories/user_repository.rs`, `src/services/user_service.rs` (rimuovere `users_info`/`users_config`/`UserWithInfo`)

**Interfaces:**
- Produces:
  - `pub struct AppState { pub pool: PgPool, pub jwt_secret: String, pub jwt_expiration_hours: i64, pub upload_dir: PathBuf, pub max_upload_bytes: usize, pub max_file_bytes: usize, pub cors_origin: Option<String> }` (Clone)
  - `pub fn build_router(state: AppState) -> Router` — tutto sotto `/api`
  - `AppError` varianti: `Database(sqlx::Error)`, `Unauthorized(String)`, `Forbidden(String)`, `NotFound(String)`, `BadRequest(String)`, `Conflict(String)`, `Internal(String)`, `Jwt(..)`, `Bcrypt(..)`, `Coded { status: StatusCode, code: &'static str, message: String }`. Codici generici: `UNAUTHORIZED`, `FORBIDDEN`, `NOT_FOUND`, `BAD_REQUEST`, `CONFLICT`, `INTERNAL` (Database/Internal/Bcrypt → `INTERNAL`, Jwt → `UNAUTHORIZED`).
  - `pub fn AppError::coded(status: StatusCode, code: &'static str, message: impl Into<String>) -> AppError`
  - `tests/common/mod.rs`: `pub struct TestApp { pub router: Router, pub upload_dir: tempfile::TempDir, pub pool: PgPool }`; `pub fn test_app(pool: PgPool) -> TestApp`; `impl TestApp { pub async fn request(&self, method: Method, uri: &str, token: Option<&str>, body: Option<serde_json::Value>) -> (StatusCode, serde_json::Value) }` (corpo vuoto → `Value::Null`)
- Migration `0001_users.sql`: `users(id BIGSERIAL PK, email VARCHAR(255) UNIQUE NOT NULL, username VARCHAR(50) UNIQUE NOT NULL, password_hash TEXT NOT NULL, role TEXT NOT NULL CHECK (role IN ('athlete','coach')), is_active BOOLEAN NOT NULL DEFAULT TRUE, created_at/updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW())`; `user_profiles(user_id BIGINT PK REFERENCES users ON DELETE CASCADE, language TEXT NOT NULL DEFAULT 'it' CHECK (language IN ('it','en')), timezone TEXT NOT NULL DEFAULT 'Europe/Rome', birth_date DATE, sex TEXT CHECK (sex IN ('male','female','other')), weight_kg DOUBLE PRECISION, max_hr INTEGER, resting_hr INTEGER, ftp_watts INTEGER, threshold_pace_s_per_km INTEGER)`.

- [ ] **Step 1: Igiene repo.** `git rm --cached .env`; aggiungere a `.gitignore` le righe `docs/files/` e `uploads/`; creare `.env.example` con `DATABASE_URL=postgres://myuser:mypassword@localhost:5432/myapp`, `JWT_SECRET=change-me-min-32-chars`, `JWT_EXPIRATION_HOURS=24`, `SERVER_HOST=127.0.0.1`, `SERVER_PORT=3000`, `CORS_ORIGIN=http://localhost:4200`, `UPLOAD_DIR=./uploads`, `MAX_UPLOAD_BYTES=1073741824`, `MAX_FILE_BYTES=52428800`, `TLS_CERT_PATH=`, `TLS_KEY_PATH=`. Rigenerare `JWT_SECRET` nel `.env` locale (64 caratteri casuali) e aggiungervi le nuove variabili.
- [ ] **Step 2: Azzerare il DB.** `docker compose down -v; docker compose up -d`; sostituire le tre migration con `0001_users.sql`; `sqlx migrate run` → Expected: `Applied 1/migrate users`.
- [ ] **Step 3: Test che falliscono.** `tests/health_tests.rs`:

```rust
#[sqlx::test]
async fn health_returns_ok(pool: PgPool) {
    let app = common::test_app(pool);
    let (status, body) = app.request(Method::GET, "/api/health", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"status": "ok"}));
}
```
Unit test in `src/errors.rs`: `AppError::coded(StatusCode::CONFLICT, "EMAIL_TAKEN", "x")` → status 409, body `{"error":"x","code":"EMAIL_TAKEN","status":409}`; `AppError::NotFound("y".into())` → `"code":"NOT_FOUND"`.
- [ ] **Step 4: `cargo test`** → Expected: FAIL (non compila: `test_app`, `coded`, `/api/health` mancanti).
- [ ] **Step 5: Implementare.** `AppState`, `build_router` e dichiarazione moduli in `src/lib.rs`; `main.rs` usa `rust_postgre::{AppState, build_router}` ed esegue `sqlx::migrate!().run(&pool)` prima di servire. `build_router` nidifica tutto in `nest("/api", …)`, aggiunge `GET /api/health` e un `CorsLayer` che consente `state.cors_origin` se presente (metodi GET/POST/PATCH/DELETE, header `authorization`, `content-type`). `Cargo.toml`: `tower-http = { version = "0.6", features = ["cors"] }`, axum feature `multipart`, sqlx feature `migrate`; dev-deps `tower = { version = "0.5", features = ["util"] }`, `http-body-util = "0.1"`, `tempfile = "3"`. `test_app` usa un `jwt_secret` fisso di test e `upload_dir` = TempDir.
- [ ] **Step 6: `cargo test`** → Expected: PASS (2 test).
- [ ] **Step 7: Commit** — `git status` (verificare che `.env` e `docs/files/` NON siano nello stage), poi `git add -A; git commit -m "chore: lib split, /api prefix, reset migrations, coded errors, CORS, test harness"`.

---

### Task 2: Registrazione con ruolo, login, `/users/me` con profilo

**Files:**
- Modify: `src/dto/auth.rs`, `src/services/auth_service.rs`, `src/controllers/auth_controller.rs`, `src/middleware.rs`, `src/models/user.rs`, `src/dto/user.rs`, `src/repositories/user_repository.rs`, `src/services/user_service.rs`, `src/controllers/user_controller.rs`, `src/lib.rs`, `tests/common/mod.rs`
- Test: `tests/auth_tests.rs`, `tests/user_tests.rs`

**Interfaces:**
- Consumes: `AppError::coded`, `TestApp::request` (Task 1)
- Produces:
  - `pub enum Role { Athlete, Coach }` (serde `lowercase`, `as_str()`), in `models/user.rs`
  - `Claims { sub: i64, email: String, role: String, exp: usize, iat: usize }`
  - `pub struct AuthUser { pub id: i64, pub role: Role }` — estrattore axum (`FromRequestParts`) da `Extension<Claims>`; `impl AuthUser { pub fn require_role(&self, role: Role) -> Result<(), AppError> }` → altrimenti `403 ROLE_FORBIDDEN`
  - `RegisterRequest { email, username, password, role: Role }`; `AuthResponse { token, user: UserInfo { id, email, username, role } }`
  - `MeResponse { id, email, username, role, created_at, profile: ProfileDto }`; `ProfileDto { language, timezone, birth_date, sex, weight_kg, max_hr, resting_hr, ftp_watts, threshold_pace_s_per_km }`
  - `user_repository::timezone_of(pool: &PgPool, user_id: i64) -> Result<String, AppError>` (usato da Task 8 e 9)
  - `TestApp::register(&self, email: &str, username: &str, role: &str) -> (i64, String)` (id, token)
- Route: `POST /api/auth/register` (201), `POST /api/auth/login` (200), `GET|PATCH|DELETE /api/users/me` (DELETE → 204). Rimuovere `GET /api/users` e `/api/users/:id`.
- Validazione registrazione → `400 VALIDATION_FAILED` (messaggio col nome del campo): email contiene `@` e ≤ 255; username 3–50 caratteri `[A-Za-z0-9_.-]`; password ≥ 8. JSON non deserializzabile (es. ruolo sconosciuto) → anch'esso `400 VALIDATION_FAILED` (estrattore JSON personalizzato che mappa il rejection di axum). Email già usata `409 EMAIL_TAKEN`, username `409 USERNAME_TAKEN`. Login errato `401 INVALID_CREDENTIALS`, disabilitato `401 ACCOUNT_DISABLED`. La registrazione crea la riga `user_profiles` con i default nella stessa transazione.
- PATCH `/users/me`: campo assente = invariato, `null` = azzera (solo i campi nullable del profilo). `timezone` validato con `SELECT 1 FROM pg_timezone_names WHERE name = $1` → altrimenti `400 INVALID_TIMEZONE`; `language` ∉ {it,en} → `400 VALIDATION_FAILED`; nuova password ≥ 8.
- DELETE `/users/me`: cancella utente (cascade) e `storage`-cartella `UPLOAD_DIR/{user_id}` se esiste (rimozione diretta con `tokio::fs::remove_dir_all`, ignorando NotFound).

- [ ] **Step 1: Test che falliscono.** `tests/auth_tests.rs`: `register_athlete_returns_token_and_role` (201, `user.role == "athlete"`); `register_coach`; `register_invalid_role_is_400` (`code == "VALIDATION_FAILED"`); `register_short_password_is_400`; `register_duplicate_email_is_409_email_taken`; `register_duplicate_username_is_409_username_taken`; `login_wrong_password_is_401_invalid_credentials`; `login_ok_returns_role`. `tests/user_tests.rs`: `me_requires_token` (401 `UNAUTHORIZED`); `me_returns_default_profile` (`language == "it"`, `timezone == "Europe/Rome"`); `patch_me_updates_profile_fields` (`weight_kg` 70.5, `max_hr` 190 → riletti); `patch_me_null_clears_field`; `patch_me_invalid_timezone_is_400` (`"Mars/Base"` → `INVALID_TIMEZONE`); `old_users_routes_are_gone` (`GET /api/users/1` con token → 404); `delete_me_then_login_fails` (204, poi login 401).
- [ ] **Step 2: `cargo test --test auth_tests --test user_tests`** → Expected: FAIL.
- [ ] **Step 3: Implementare** secondo le interfacce. `auth_middleware` resta com'è e inietta `Claims`; `AuthUser` li converte.
- [ ] **Step 4: `cargo test`** → Expected: PASS.
- [ ] **Step 5: Commit** — `feat: roles, profile and /users/me`.

---

### Task 3: Relazione allenatore-atleta e servizio di accesso

**Files:**
- Create: `migrations/0002_coach_links.sql`, `src/models/coach_link.rs`, `src/dto/coach_link.rs`, `src/repositories/coach_link_repository.rs`, `src/services/coach_link_service.rs`, `src/services/access_service.rs`, `src/controllers/coach_link_controller.rs`
- Modify: `src/lib.rs`, i `mod.rs`
- Test: `tests/coach_link_tests.rs`

**Interfaces:**
- Consumes: `AuthUser`, `Role`, `TestApp::register`
- Produces:
  - `coach_links(id BIGSERIAL PK, athlete_id BIGINT NOT NULL REFERENCES users ON DELETE CASCADE, coach_id BIGINT NOT NULL REFERENCES users ON DELETE CASCADE, status TEXT NOT NULL CHECK (status IN ('pending','active','rejected','cancelled','revoked')), requested_at TIMESTAMPTZ NOT NULL DEFAULT NOW(), responded_at TIMESTAMPTZ, ended_at TIMESTAMPTZ)` + `CREATE UNIQUE INDEX coach_links_one_open ON coach_links(athlete_id) WHERE status IN ('pending','active')`
  - `CoachLinkDto { id, athlete: UserRef, coach: UserRef, status, requested_at, responded_at, ended_at }`, `UserRef { id, username }`
  - `pub async fn access_service::resolve_target(pool: &PgPool, viewer: &AuthUser, athlete_id: Option<i64>) -> Result<i64, AppError>` — `None` o id del viewer → id del viewer; altrimenti il viewer deve essere `coach` con link `active` verso quell'atleta, altrimenti `403 ATHLETE_ACCESS_DENIED`. Usato da Task 8 e 9.
- Route:
  - `POST /api/coach-links` body `{"coach": "<email o username>"}` — solo atleta (altrimenti `403 ROLE_FORBIDDEN`) → 201 `CoachLinkDto` `pending`. Utente inesistente → `404 COACH_NOT_FOUND`; esiste ma non coach → `400 NOT_A_COACH`; atleta con link `pending`/`active` → `409 COACH_LINK_EXISTS`.
  - `GET /api/coach-links?status=<opz>` — atleta: i propri; coach: quelli verso di sé; `requested_at DESC`.
  - `POST /api/coach-links/:id/accept` | `/reject` — solo il coach destinatario, solo da `pending` (imposta `responded_at`).
  - `POST /api/coach-links/:id/cancel` — solo l'atleta, da `pending` → `cancelled` (`ended_at`).
  - `POST /api/coach-links/:id/revoke` — solo l'atleta, da `active` → `revoked` (`ended_at`).
  - Transizione non ammessa → `409 LINK_INVALID_STATE`; link non proprio → `404 NOT_FOUND`.
  - `GET /api/athletes` — solo coach → `[{ id, username, linked_since }]` con link `active`.

- [ ] **Step 1: Test che falliscono** in `tests/coach_link_tests.rs` — via HTTP: `athlete_requests_coach_by_username`; `athlete_requests_coach_by_email`; `coach_cannot_create_request` (403 `ROLE_FORBIDDEN`); `request_to_non_coach_is_400_not_a_coach`; `request_to_unknown_is_404_coach_not_found`; `second_request_while_pending_is_409`; `coach_accepts_then_athlete_appears_in_athletes`; `coach_rejects`; `athlete_cancels_pending`; `athlete_revokes_active_then_can_request_again`; `accept_twice_is_409_link_invalid_state`; `other_coach_cannot_accept_is_404`. Chiamando direttamente `access_service::resolve_target` col pool: `resolve_own_id_ok`, `resolve_coach_active_ok`, `resolve_coach_pending_denied`, `resolve_athlete_on_other_athlete_denied`, `coach_loses_access_after_revoke` (Review Focus 5).
- [ ] **Step 2: `sqlx migrate run; cargo test --test coach_link_tests`** → Expected: FAIL.
- [ ] **Step 3: Implementare.**
- [ ] **Step 4: `cargo test`** → Expected: PASS.
- [ ] **Step 5: Commit** — `feat: coach-athlete links and access control`.

---

### Task 4: Parsing GPX e TCX + metriche

**Files:**
- Create: `src/parsing/mod.rs`, `src/parsing/metrics.rs`, `src/parsing/gpx.rs`, `src/parsing/tcx.rs`, `tests/fixtures/run.gpx`, `tests/fixtures/ride.tcx`, `tests/fixtures/ride_same_as_tcx.gpx`
- Modify: `Cargo.toml` (`quick-xml = "0.37"`), `src/lib.rs`

**Interfaces:**
- Produces (`src/parsing/mod.rs`):
  - `pub enum Sport { Running, Cycling, Swimming, Walking, Hiking, Strength, Other }` + `pub fn as_str(&self) -> &'static str` (`"running"`, `"cycling"`, …)
  - `pub enum FileFormat { Fit, Gpx, Tcx }`; `pub fn from_filename(name: &str) -> Option<FileFormat>` (estensione case-insensitive); `as_str()` → `"fit"|"gpx"|"tcx"`
  - `pub struct ParsedActivity { pub sport: Sport, pub sport_raw: String, pub name: Option<String>, pub start_time: DateTime<Utc>, pub utc_offset_seconds: Option<i32>, pub elapsed_s: i32, pub moving_s: Option<i32>, pub distance_m: Option<f64>, pub elevation_gain_m: Option<f64>, pub avg_hr: Option<i32>, pub max_hr: Option<i32>, pub avg_speed_mps: Option<f64>, pub avg_power_w: Option<i32>, pub avg_cadence: Option<i32>, pub calories: Option<i32> }`
  - `pub enum ParseError { Malformed(String), Empty }` + `pub fn code(&self) -> &'static str` → `"PARSE_ERROR"` / `"EMPTY_ACTIVITY"`
  - `pub fn parse(format: FileFormat, bytes: &[u8]) -> Result<ParsedActivity, ParseError>` (ramo `Fit` nel Task 5; fino ad allora `Malformed("fit not yet supported")`)
  - `metrics.rs`:
    - `pub fn elevation_gain(altitudes: &[f64]) -> f64` — isteresi 2,0 m:
      ```
      ref = a[0]; gain = 0
      per ogni a: se a > ref + 2.0 { gain += a - ref; ref = a } altrimenti se a < ref { ref = a }
      ```
    - `pub fn haversine_m(a: (f64, f64), b: (f64, f64)) -> f64` (lat, lon in gradi; raggio 6_371_000 m)
    - `pub fn moving_seconds(samples: &[(DateTime<Utc>, f64)]) -> i32` — somma degli intervalli tra campioni consecutivi con velocità > 0,5 m/s (`f64` = distanza cumulata in m)
- Mappatura sport — TCX: `Running`→running, `Biking`→cycling, altro→other. GPX `<type>` (minuscolo): contiene `run`→running; `bik`|`cycl`|`ride`→cycling; `swim`→swimming; `walk`→walking; `hik`→hiking; `strength`|`weight`→strength; altrimenti other. `sport_raw` = valore originale.
- TCX: prima `<Activity>`; start = `<Id>`; `moving_s` = somma `TotalTimeSeconds` dei lap; `elapsed_s` = ultimo trackpoint − start; distanza e calorie = somme dei lap; `avg_hr` = media dei lap pesata su `TotalTimeSeconds`; `max_hr` = max dei lap; dislivello da `AltitudeMeters`; potenza = media dei `Watts` dei trackpoint (match sul local name, qualsiasi prefisso); cadenza = media pesata di `Cadence` dei lap, altrimenti `RunCadence`; `avg_speed_mps` = distanza / moving_s; `utc_offset_seconds = None`.
- GPX: start = primo `<time>`; distanza = somma haversine; `elapsed_s` = ultimo − primo; `moving_s` = `moving_seconds`; dislivello da `<ele>`; FC/cadenza/potenza dalle estensioni (local name `hr`, `cad`, `power`) se presenti; `name` = `<trk><name>`. Nessun punto con tempo → `Empty`.

- [ ] **Step 1: Fixture sintetiche** (coordinate fittizie vicino a lat 0, lon 0): `run.gpx` — `<type>running</type>`, `<name>Morning Run</name>`, 11 punti ogni 10 s spaziati 0,001° di latitudine, `<ele>` `100,101,103,102,106,106,104,110,111,111,112`, estensione `hr` da 140 a 150; `ride.tcx` — `Sport="Biking"`, `<Id>2026-01-10T08:00:00Z</Id>`, lap 1 (600 s, 5000 m, 100 cal, avgHR 130, maxHR 150), lap 2 (300 s, 2500 m, 60 cal, avgHR 160, maxHR 175), trackpoint ogni 60 s da `08:00:00` a `08:15:00` con `AltitudeMeters` e `ns3:TPX/ns3:Watts` = 200; `ride_same_as_tcx.gpx` — `<type>cycling</type>`, primo punto `2026-01-10T08:00:30Z`, 5 punti.
- [ ] **Step 2: Test che falliscono** (unit, nei rispettivi file):
  - `metrics`: `elevation_gain(&[100.,101.,103.,102.,106.,106.,104.,110.,111.,111.,112.]) == 13.0`; `elevation_gain(&[100.,101.,100.,101.]) == 0.0`; `haversine_m((0.0,0.0),(0.001,0.0))` ∈ `111.0..111.4`; `moving_seconds` su 4 campioni a 10 s con distanze cumulate `[0, 50, 50, 100]` == 20.
  - `tcx::parse_ride_tcx`: sport `Cycling`, sport_raw `"Biking"`, start `2026-01-10T08:00:00Z`, `elapsed_s` 900, `moving_s` 900, distance 7500.0, calories 160, `avg_hr` 140, `max_hr` 175, `avg_power_w` 200, `avg_speed_mps` ∈ `8.33..8.34`.
  - `gpx::parse_run_gpx`: sport `Running`, name `"Morning Run"`, `elapsed_s` 100, distance ∈ `1110.0..1114.0`, `elevation_gain_m == 13.0`, `avg_hr` presente.
  - `parse_garbage_is_malformed` (`b"not xml"` con Gpx e Tcx); `gpx_without_points_is_empty`.
- [ ] **Step 3: `cargo test parsing`** → Expected: FAIL.
- [ ] **Step 4: Implementare** con `quick_xml::Reader` in streaming, confrontando i local name.
- [ ] **Step 5: `cargo test parsing`** → Expected: PASS.
- [ ] **Step 6: Commit** — `feat: GPX/TCX parsing and metrics`.

---

### Task 5: Parsing FIT + test sui file reali

**Files:**
- Create: `src/parsing/fit.rs`, `tests/fixtures/malformed.fit`, `tests/real_files.rs`
- Modify: `src/parsing/mod.rs`, `Cargo.toml` (`fitparser = "0.9"`; dev-dep `zip` già presente dal Task 6 — se si esegue prima, aggiungerla qui)

**Interfaces:**
- Consumes: `ParsedActivity`, `Sport`, `ParseError` (Task 4)
- Produces: ramo `FileFormat::Fit` di `parse`.
- Regole FIT: messaggi `session`. Più sessioni → sport della sessione con `total_elapsed_time` maggiore; tempi, distanza, ascesa, calorie sommati; `max_hr` = massimo; `avg_hr`/`avg_power`/`avg_cadence` = medie pesate su `total_timer_time`. Campi: `start_time`; `total_elapsed_time`→`elapsed_s`; `total_timer_time`→`moving_s`; `total_distance`; `total_ascent`; `avg_heart_rate`; `max_heart_rate`; `enhanced_avg_speed` (fallback `avg_speed`); `avg_power`; `avg_cadence`; `total_calories`. `utc_offset_seconds` = `activity.local_timestamp − activity.timestamp` se presente. Sport: `running`→Running, `cycling`→Cycling, `swimming`→Swimming, `walking`→Walking, `hiking`→Hiking, `training` + sub_sport `strength_training`→Strength, altro→Other; `sport_raw` = `"<sport>/<sub_sport>"`. Nessuna `session` → `Empty`; errore del decoder → `Malformed`.

- [ ] **Step 1: Test che falliscono.** Unit `fit_garbage_is_malformed` su `tests/fixtures/malformed.fit` (64 byte: header FIT di 14 byte con firma `.FIT` seguito da byte casuali). `tests/real_files.rs`, tutti `#[ignore]`, leggono da `docs/files/` e se il file manca fanno `panic!` con messaggio esplicito:
  - `real_coros_fit_parses` — `481934560924303460_f9d4fbc5e2.fit`: `elapsed_s > 0`, `distance_m > 0`, `start_time` successivo al `2015-01-01`.
  - `real_garmin_tcx_parses` — `activity_25087995114.tcx`: sport `Cycling`, start `2026-11-15T11:08:59Z`.
  - `real_garmin_zip_fit_matches_tcx` — FIT estratto da `25143684168.zip` (crate `zip` nel test) vs `activity_25143684168.tcx`: entrambi `Cycling`, `|start_fit − start_tcx| ≤ 60 s`, distanze entro il 2 %.
- [ ] **Step 2: `cargo test fit` e `cargo test --test real_files -- --ignored`** → Expected: FAIL.
- [ ] **Step 3: Implementare** con `fitparser::from_bytes`.
- [ ] **Step 4: Rieseguire i due comandi** → Expected: PASS. Riportare nel messaggio di commit sport e distanza letti dal file Coros, per un controllo umano.
- [ ] **Step 5: Commit** — `feat: FIT parsing` (verificare che nulla di `docs/files/` sia nello stage).

---

### Task 6: Espansione archivi e storage dei file originali

**Files:**
- Create: `src/parsing/archive.rs`, `src/storage.rs`
- Modify: `Cargo.toml` (`zip = { version = "2", default-features = false, features = ["deflate"] }`, `sha2 = "0.10"`, `hex = "0.4"`), `src/parsing/mod.rs`, `src/lib.rs`

**Interfaces:**
- Produces:
  - `pub struct ArchiveLimits { pub max_file_bytes: u64, pub max_total_bytes: u64, pub max_entries: usize }` + `Default` (50 MiB / 2 GiB / 5000)
  - `pub struct InputFile { pub filename: String, pub format: FileFormat, pub bytes: Vec<u8> }`
  - `pub enum Expanded { File(InputFile), Rejected { filename: String, code: &'static str } }`
  - `pub fn expand_upload(filename: &str, path: &Path, limits: &ArchiveLimits) -> Vec<Expanded>` — `.fit/.gpx/.tcx` → un `File` (o `Rejected FILE_TOO_LARGE`); `.zip` → voci espanse; ZIP annidati espansi **un solo livello**; voci non supportate, directory e ZIP di livello più profondo ignorate in silenzio; `filename` = solo basename della voce (mai usato come percorso); superati `max_entries` o `max_total_bytes` → un unico `Rejected { filename: <nome zip>, code: "ARCHIVE_TOO_LARGE" }`; voce oltre `max_file_bytes` → `Rejected FILE_TOO_LARGE` (leggere con `take(max + 1)`, non fidarsi della dimensione dichiarata); ZIP illeggibile → `Rejected PARSE_ERROR`; altra estensione al primo livello → `Rejected UNSUPPORTED_FORMAT`.
  - `storage.rs`: `pub fn sha256_hex(bytes: &[u8]) -> String`; `pub async fn save(upload_dir: &Path, user_id: i64, sha256: &str, format: FileFormat, bytes: &[u8]) -> std::io::Result<String>` → percorso relativo `"{user_id}/{sha256}.{ext}"`; `pub async fn remove(upload_dir: &Path, relative: &str) -> std::io::Result<()>` (NotFound ignorato).

- [ ] **Step 1: Test che falliscono** (ZIP costruiti nel test con `zip::ZipWriter` su `tempfile`): `plain_gpx_is_single_file`; `unknown_extension_rejected_unsupported`; `zip_with_activity_and_json_keeps_only_activity`; `import_nested_zip_one_level` (zip esterno con `DI_CONNECT/DI-Connect-Uploaded-Files/UploadedFiles_0-_Part1.zip` contenente 2 `.tcx` e un ulteriore `.zip` → esattamente 2 `File`) — Review Focus 1; `zip_entry_over_limit_rejected_file_too_large` (limite 1024 byte); `zip_too_many_entries_rejected` (`max_entries` 3, 4 voci); `zip_entry_with_dotdot_path_returns_basename` (`../../evil.gpx` → `evil.gpx`); `corrupt_zip_rejected_parse_error`; `save_then_remove_roundtrip`; `sha256_known_vector` (`sha256_hex(b"abc") == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"`).
- [ ] **Step 2: `cargo test archive` e `cargo test storage`** → Expected: FAIL.
- [ ] **Step 3: Implementare.**
- [ ] **Step 4: Rieseguire** → Expected: PASS.
- [ ] **Step 5: Commit** — `feat: archive expansion and original file storage`.

---

### Task 7: Pipeline di import (upload, background, duplicati, stato)

**Files:**
- Create: `migrations/0003_activities_imports.sql`, `src/models/activity.rs`, `src/models/import.rs`, `src/dto/import.rs`, `src/repositories/activity_repository.rs`, `src/repositories/import_repository.rs`, `src/services/import_service.rs`, `src/controllers/import_controller.rs`
- Modify: `src/lib.rs`, `src/main.rs`, `tests/common/mod.rs`
- Test: `tests/import_tests.rs`

**Interfaces:**
- Consumes: `parse`, `expand_upload`, `ArchiveLimits`, `storage::*` (Task 4–6), `AuthUser`, `Role`
- Produces:
  - `activities(id BIGSERIAL PK, user_id BIGINT NOT NULL REFERENCES users ON DELETE CASCADE, source TEXT NOT NULL DEFAULT 'file_upload' CHECK (source IN ('file_upload','garmin','coros','strava')), sport TEXT NOT NULL, sport_raw TEXT NOT NULL, name TEXT, start_time TIMESTAMPTZ NOT NULL, utc_offset_seconds INTEGER, elapsed_s INTEGER NOT NULL, moving_s INTEGER, distance_m DOUBLE PRECISION, elevation_gain_m DOUBLE PRECISION, avg_hr INTEGER, max_hr INTEGER, avg_speed_mps DOUBLE PRECISION, avg_power_w INTEGER, avg_cadence INTEGER, calories INTEGER, file_format TEXT NOT NULL, file_path TEXT NOT NULL, file_sha256 TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(), UNIQUE (user_id, file_sha256))` + indice `(user_id, start_time)`; `imports(id BIGSERIAL PK, user_id BIGINT NOT NULL REFERENCES users ON DELETE CASCADE, status TEXT NOT NULL CHECK (status IN ('processing','completed','failed')), created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(), finished_at TIMESTAMPTZ)`; `import_items(id BIGSERIAL PK, import_id BIGINT NOT NULL REFERENCES imports ON DELETE CASCADE, filename TEXT NOT NULL, status TEXT NOT NULL CHECK (status IN ('pending','imported','duplicate','failed')), activity_id BIGINT REFERENCES activities ON DELETE SET NULL, error_code TEXT)`
  - `activity_repository::find_by_hash(pool, user_id: i64, sha: &str) -> Result<Option<i64>, AppError>`; `find_overlapping(pool, user_id: i64, sport: &str, start: DateTime<Utc>, tolerance_s: i64) -> Result<Option<i64>, AppError>`; `insert(pool, user_id: i64, a: &ParsedActivity, format: FileFormat, file_path: &str, sha: &str) -> Result<i64, AppError>`
  - `import_repository::fail_interrupted(pool) -> Result<u64, AppError>` — import `processing` → `failed` (`finished_at = NOW()`), item `pending` → `failed` con `error_code = 'IMPORT_INTERRUPTED'`; chiamata in `main.rs` all'avvio dopo le migration
  - `ImportDto { id, status, created_at, finished_at, items: Vec<ImportItemDto> }`, `ImportItemDto { filename, status, activity_id, error_code }`
  - `TestApp::upload(&self, token: &str, files: &[(&str, Vec<u8>)]) -> (StatusCode, Value)`; `TestApp::wait_import(&self, token: &str, id: i64) -> Value` (poll ogni 50 ms, max 10 s, panica se non `completed`)
- Route: `POST /api/imports` (multipart, campo ripetibile `files`) — solo atleta → `202 ImportDto` (`processing`); nessun file → `400 NO_FILES`; corpo oltre `MAX_UPLOAD_BYTES` → `413 UPLOAD_TOO_LARGE`. `GET /api/imports/:id` — solo proprietario (altrimenti `404 NOT_FOUND`). `GET /api/imports` — ultimi 20 dell'utente, senza item.
- Flusso: la route ha `DefaultBodyLimit::max(state.max_upload_bytes)`; il controller scrive ogni campo `files` **in streaming** (chunk per chunk) in un file temporaneo sotto `UPLOAD_DIR/tmp/`; crea l'import; risponde 202; un `tokio::spawn` elabora ciascun upload con `expand_upload` e, per ogni elemento, crea il suo `import_item` e: `Rejected` → `failed` col codice; `File` → `sha256_hex` → `find_by_hash` (→ `duplicate` con quell'id) → `parse` (errore → `failed` con `code()`) → `find_overlapping(±60 s)` (→ `duplicate`) → `storage::save` → `insert` (violazione UNIQUE → `duplicate`) → `imported`. Un errore inatteso su un elemento lo marca `failed`/`INTERNAL` e si prosegue. Alla fine: import `completed`, `finished_at`, file temporanei rimossi.

- [ ] **Step 1: Test che falliscono** in `tests/import_tests.rs`: `upload_gpx_imports_activity` (item `imported`, `activity_id` presente, file esistente sotto `upload_dir/{user_id}/`); `upload_same_file_twice_second_is_duplicate`; `same_activity_fit_and_tcx_is_duplicate` (`ride.tcx` poi `ride_same_as_tcx.gpx` → il secondo `duplicate` con `activity_id` del primo) — Review Focus 2; `malformed_file_fails_others_succeed` (`malformed.fit` + `run.gpx` insieme → uno `failed`/`PARSE_ERROR`, uno `imported`, import `completed`); `unsupported_extension_fails_unsupported_format` (`notes.txt`); `zip_upload_expands` (zip con `run.gpx` e `ride.tcx` → 2 `imported`); `coach_cannot_upload` (403 `ROLE_FORBIDDEN`); `no_files_is_400_no_files`; `other_user_cannot_read_import` (404); `upload_larger_than_axum_default_limit` (`run.gpx` con ~3 MB di commento XML inserito → `imported`) — Review Focus 3; `fail_interrupted_marks_pending` (inserire a mano un import `processing` con un item `pending`, chiamare `fail_interrupted`, verificare stati e `IMPORT_INTERRUPTED`).
- [ ] **Step 2: `sqlx migrate run; cargo test --test import_tests`** → Expected: FAIL.
- [ ] **Step 3: Implementare.**
- [ ] **Step 4: `cargo test`** → Expected: PASS.
- [ ] **Step 5: Commit** — `feat: background file import with duplicate detection`.

---

### Task 8: Elenco e cancellazione attività

**Files:**
- Create: `src/dto/activity.rs`, `src/controllers/activity_controller.rs`
- Modify: `src/repositories/activity_repository.rs`, `src/lib.rs`
- Test: `tests/activity_tests.rs`

**Interfaces:**
- Consumes: `access_service::resolve_target` (Task 3), `user_repository::timezone_of` (Task 2), `storage::remove` (Task 6), `TestApp::upload`/`wait_import` (Task 7)
- Produces:
  - `ActivityDto { id, source, sport, sport_raw, name, start_time, utc_offset_seconds, elapsed_s, moving_s, distance_m, elevation_gain_m, avg_hr, max_hr, avg_speed_mps, avg_power_w, avg_cadence, calories, file_format }`
  - `GET /api/activities?from=YYYY-MM-DD&to=YYYY-MM-DD&sport=<s>&page=1&per_page=20&athlete_id=<opz>` → `{ items: [ActivityDto], total, page, per_page }`, `start_time DESC`. `from`/`to` opzionali e inclusivi, nel fuso **dell'atleta target**: `(start_time AT TIME ZONE $tz)::date BETWEEN $from AND $to`. `per_page` 1–100 (fuori → `400 VALIDATION_FAILED`); `from > to` → `400 VALIDATION_FAILED`.
  - `DELETE /api/activities/:id` — solo il proprietario (chiunque altro, coach incluso → `404 NOT_FOUND`) → 204, rimuove anche il file originale.
- [ ] **Step 1: Test che falliscono**: `list_own_activities_desc`; `list_filters_by_date_and_sport`; `list_paginates_with_total`; `coach_with_active_link_can_list_athlete`; `coach_without_link_gets_403_athlete_access_denied`; `athlete_cannot_list_other_athlete` (403); `delete_own_activity_removes_file`; `cannot_delete_others_activity_404`; `per_page_over_100_is_400`.
- [ ] **Step 2: `cargo test --test activity_tests`** → Expected: FAIL.
- [ ] **Step 3: Implementare.**
- [ ] **Step 4: `cargo test`** → Expected: PASS.
- [ ] **Step 5: Commit** — `feat: activity list and delete`.

---

### Task 9: Statistiche per intervallo

**Files:**
- Create: `src/dto/stats.rs`, `src/repositories/stats_repository.rs`, `src/services/stats_service.rs`, `src/controllers/stats_controller.rs`
- Modify: `src/lib.rs`
- Test: `tests/stats_tests.rs`

**Interfaces:**
- Consumes: `resolve_target`, `timezone_of`, `activity_repository::insert`
- Produces: `GET /api/stats/summary?from=YYYY-MM-DD&to=YYYY-MM-DD&sport=<opz>&granularity=day|week|month&athlete_id=<opz>` →
  ```json
  { "from": "2026-11-01", "to": "2026-11-30", "granularity": "day", "timezone": "Europe/Rome",
    "totals": { "count": 0, "distance_m": 0.0, "moving_s": 0, "elapsed_s": 0, "elevation_gain_m": 0.0 },
    "by_sport": [ { "sport": "running", "count": 0, "distance_m": 0.0, "moving_s": 0 } ],
    "series": [ { "period_start": "2026-11-01", "count": 0, "distance_m": 0.0, "moving_s": 0, "elevation_gain_m": 0.0 } ] }
  ```
  - `from`, `to` obbligatori; mancanti, `from > to` o intervallo > 3660 giorni → `400 VALIDATION_FAILED`.
  - `granularity` assente → ≤ 31 giorni `day`, ≤ 180 `week`, altrimenti `month`; settimane ISO (lunedì).
  - Aggregazione nel fuso dell'atleta target: `date_trunc(g, start_time AT TIME ZONE tz)`; `moving_s` = `COALESCE(moving_s, elapsed_s)`; campi null = 0.
  - `series` include i periodi vuoti a zero (`generate_series` da `date_trunc(g, from)` a `to`); `by_sport` ordinato per `moving_s DESC`.
- [ ] **Step 1: Test che falliscono** (attività inserite con `activity_repository::insert` e un `ParsedActivity` costruito nel test): `summary_totals_and_by_sport`; `series_zero_fills_empty_days` (10 giorni, 2 attività → 10 elementi, 8 a zero); `auto_granularity_day_week_month` (10, 90, 365 giorni); `stats_bucket_uses_athlete_timezone` — attività `2026-03-28T23:30:00Z`, fuso `Europe/Rome` → conteggiata nel periodo `2026-03-29` (Review Focus 4); `sport_filter`; `missing_from_is_400`; `from_after_to_is_400`; `coach_active_link_ok_else_403`.
- [ ] **Step 2: `cargo test --test stats_tests`** → Expected: FAIL.
- [ ] **Step 3: Implementare.**
- [ ] **Step 4: `cargo test`** → Expected: PASS.
- [ ] **Step 5: Commit** — `feat: stats summary by date range`.

---

### Task 10: Chiusura — README, qualità, prova end-to-end

**Files:**
- Modify: `README.md`

- [ ] **Step 1: README** (in italiano): prerequisiti (Docker, Rust, sqlx-cli); `copy .env.example .env`; `docker compose up -d`; `sqlx migrate run`; `cargo run`; `cargo test`; `cargo test --test real_files -- --ignored`; tabella degli endpoint (metodo, percorso, ruolo, una riga di descrizione); nota che `docs/files/` è locale e ignorato da git.
- [ ] **Step 2: `cargo clippy --all-targets -- -D warnings`** → Expected: nessun warning (correggere).
- [ ] **Step 3: `cargo test` e `cargo test --test real_files -- --ignored`** → Expected: tutto PASS.
- [ ] **Step 4: Prova manuale**: `cargo run`; con `curl -k` registrare un atleta, caricare i quattro file di `docs/files/` in un unico upload, attendere `completed` → atteso 3 `imported` + 1 `duplicate` (FIT nello ZIP vs `activity_25143684168.tcx`); `GET /api/stats/summary?from=2017-01-01&to=2026-12-31` → `totals.count == 3`.
- [ ] **Step 5: Commit** — `docs: README for local setup and API`.
