/// controllers/import_controller.rs — Handler HTTP per gli import di file
///
/// Endpoint protetti da JWT:
///   POST /imports      → upload multipart (campo ripetibile `files`), solo atleta (202;
///                        409 IMPORT_IN_PROGRESS se l'utente ha già un import in corso)
///   GET  /imports      → ultimi 20 import dell'utente, senza item
///   GET  /imports/:id  → dettaglio con gli item (solo il proprietario, altrimenti 404)
///
/// Il corpo dell'upload non viene mai tenuto in memoria: ogni campo `files`
/// è scritto chunk per chunk in un file temporaneo sotto `UPLOAD_DIR/tmp/`.
/// La dimensione massima del corpo (`MAX_UPLOAD_BYTES`) è imposta dal
/// `DefaultBodyLimit` della route.
use std::path::Path;

use axum::{
    Json,
    extract::{
        Multipart, State,
        multipart::{MultipartError, MultipartRejection},
    },
    http::StatusCode,
};
use tokio::io::AsyncWriteExt;

use crate::AppState;
use crate::dto::import::ImportDto;
use crate::errors::AppError;
use crate::extractors::AppPath;
use crate::middleware::AuthUser;
use crate::models::user::Role;
use crate::parsing::archive::ArchiveLimits;
use crate::services::import_service::{self, UploadedFile};

/// Nome del campo multipart che contiene i file.
const FILES_FIELD: &str = "files";

/// POST /imports
pub async fn upload(
    State(state): State<AppState>,
    auth: AuthUser,
    multipart: Result<Multipart, MultipartRejection>,
) -> Result<(StatusCode, Json<ImportDto>), AppError> {
    // Ruolo e import in corso si verificano prima di leggere il corpo
    auth.require_role(Role::Athlete)?;
    import_service::ensure_no_import_in_progress(&state.pool, auth.id).await?;
    let mut multipart = multipart.map_err(|rejection| {
        AppError::coded(
            rejection.status(),
            "VALIDATION_FAILED",
            rejection.body_text(),
        )
    })?;

    let files = receive_files(&state.upload_dir, &mut multipart).await?;
    if files.is_empty() {
        return Err(AppError::coded(
            StatusCode::BAD_REQUEST,
            "NO_FILES",
            "Nessun file nel campo 'files'",
        ));
    }

    let limits = ArchiveLimits {
        max_file_bytes: state.max_file_bytes as u64,
        ..ArchiveLimits::default()
    };
    let import = import_service::start_import(
        &state.pool,
        &state.upload_dir,
        limits,
        state.import_permits.clone(),
        auth.id,
        files,
    )
    .await?;
    Ok((StatusCode::ACCEPTED, Json(import)))
}

/// GET /imports
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<ImportDto>>, AppError> {
    Ok(Json(
        import_service::list_imports(&state.pool, &auth).await?,
    ))
}

/// GET /imports/:id
pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    AppPath(id): AppPath<i64>,
) -> Result<Json<ImportDto>, AppError> {
    Ok(Json(
        import_service::get_import(&state.pool, &auth, id).await?,
    ))
}

/// Scrive ogni campo `files` in un file temporaneo sotto `UPLOAD_DIR/tmp/`.
///
/// Gli altri campi sono ignorati. In caso di errore i file temporanei già
/// creati vengono eliminati (drop di `TempPath`).
async fn receive_files(
    upload_dir: &Path,
    multipart: &mut Multipart,
) -> Result<Vec<UploadedFile>, AppError> {
    let tmp_dir = upload_dir.join("tmp");
    tokio::fs::create_dir_all(&tmp_dir)
        .await
        .map_err(io_error)?;

    let mut files = Vec::new();
    while let Some(mut field) = multipart.next_field().await.map_err(multipart_error)? {
        if field.name() != Some(FILES_FIELD) {
            continue;
        }
        let filename = field.file_name().unwrap_or_default().to_string();

        let (file, path) = tempfile::Builder::new()
            .prefix("upload-")
            .tempfile_in(&tmp_dir)
            .map_err(io_error)?
            .into_parts();
        let mut out = tokio::fs::File::from_std(file);
        while let Some(chunk) = field.chunk().await.map_err(multipart_error)? {
            out.write_all(&chunk).await.map_err(io_error)?;
        }
        // Attende il completamento dell'ultima scrittura prima di chiudere
        out.flush().await.map_err(io_error)?;
        drop(out);

        files.push(UploadedFile { filename, path });
    }
    Ok(files)
}

/// Errore di lettura del corpo multipart: `413 UPLOAD_TOO_LARGE` oltre il
/// limite, altrimenti `400 VALIDATION_FAILED`.
fn multipart_error(e: MultipartError) -> AppError {
    if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
        AppError::coded(
            StatusCode::PAYLOAD_TOO_LARGE,
            "UPLOAD_TOO_LARGE",
            "Upload oltre la dimensione massima consentita",
        )
    } else {
        AppError::coded(StatusCode::BAD_REQUEST, "VALIDATION_FAILED", e.body_text())
    }
}

/// Errore di I/O sui file temporanei: il dettaglio (con i percorsi) va solo nel log.
fn io_error(e: std::io::Error) -> AppError {
    tracing::error!("scrittura file temporaneo fallita: {e}");
    AppError::Internal("Errore interno del server".to_string())
}
