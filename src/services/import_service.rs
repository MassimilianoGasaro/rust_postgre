/// services/import_service.rs — Logica di business degli import di file
///
/// Un upload viene registrato come import `processing` e restituito subito;
/// l'elaborazione avviene in background (`tokio::spawn`), un file caricato
/// alla volta:
///
/// 1. `expand_upload` (sincrono, in `spawn_blocking`) espande il file o lo ZIP
///    e consegna ogni elemento su un canale limitato;
/// 2. per ogni elemento si crea un `import_item` e si decide l'esito:
///    rifiutato → `failed`; stesso SHA-256 già importato → `duplicate`;
///    parsing fallito → `failed`; stessa attività (sport + inizio ±60 s) →
///    `duplicate`; altrimenti file salvato e attività inserita → `imported`;
/// 3. alla fine l'import diventa `completed` e i file temporanei sono rimossi.
///
/// Un errore inatteso su un elemento lo marca `failed`/`INTERNAL` e si prosegue.

use std::path::{Path, PathBuf};

use sqlx::PgPool;
use tempfile::TempPath;
use tokio::sync::mpsc;

use crate::dto::import::ImportDto;
use crate::errors::AppError;
use crate::middleware::AuthUser;
use crate::models::import::ItemStatus;
use crate::parsing::archive::{expand_upload, ArchiveLimits, Expanded, InputFile};
use crate::parsing::parse;
use crate::repositories::{activity_repository, import_repository};
use crate::storage;

/// Tolleranza sull'orario di inizio per considerare due attività la stessa.
const DUPLICATE_TOLERANCE_S: i64 = 60;

/// Lunghezza massima (in caratteri) del nome file salvato negli item.
const MAX_FILENAME_CHARS: usize = 255;

/// Numero di import restituiti da `list`.
const LIST_LIMIT: i64 = 20;

/// Elementi espansi in attesa di elaborazione: limita la memoria usata
/// quando l'espansione è più veloce delle scritture a DB.
const CHANNEL_CAPACITY: usize = 4;

/// File caricato dal client e già scritto su disco in un file temporaneo.
pub struct UploadedFile {
    /// Nome dichiarato dal client (non fidato, mai usato come percorso)
    pub filename: String,
    /// File temporaneo: viene eliminato al drop
    pub path: TempPath,
}

/// Dati comuni all'elaborazione di tutti gli elementi di un import.
struct ImportContext {
    pool: PgPool,
    upload_dir: PathBuf,
    limits: ArchiveLimits,
    user_id: i64,
    import_id: i64,
}

/// Esito finale di un elemento.
enum Outcome {
    Imported(i64),
    Duplicate(i64),
    Failed(&'static str),
}

/// Registra un import `processing` e ne avvia l'elaborazione in background.
///
/// I file temporanei passano al task di background, che li elimina.
pub async fn start_import(
    pool: &PgPool,
    upload_dir: &Path,
    limits: ArchiveLimits,
    user_id: i64,
    files: Vec<UploadedFile>,
) -> Result<ImportDto, AppError> {
    let import = import_repository::create(pool, user_id).await?;

    let ctx = ImportContext {
        pool: pool.clone(),
        upload_dir: upload_dir.to_path_buf(),
        limits,
        user_id,
        import_id: import.id,
    };
    tokio::spawn(run_import(ctx, files));

    Ok(ImportDto::with_items(import, Vec::new()))
}

/// Dettaglio di un import con i suoi item; `404` se non è dell'utente.
pub async fn get_import(pool: &PgPool, auth: &AuthUser, id: i64) -> Result<ImportDto, AppError> {
    let import = import_repository::find_for_user(pool, id, auth.id)
        .await?
        .ok_or_else(|| AppError::NotFound("Import non trovato".to_string()))?;
    let items = import_repository::list_items(pool, import.id).await?;
    Ok(ImportDto::with_items(import, items))
}

/// Ultimi import dell'utente, senza item.
pub async fn list_imports(pool: &PgPool, auth: &AuthUser) -> Result<Vec<ImportDto>, AppError> {
    let imports = import_repository::list_recent(pool, auth.id, LIST_LIMIT).await?;
    Ok(imports.into_iter().map(ImportDto::from).collect())
}

/// Elabora i file caricati uno alla volta, poi chiude l'import.
async fn run_import(ctx: ImportContext, files: Vec<UploadedFile>) {
    for file in files {
        process_upload(&ctx, file).await;
    }

    if let Err(e) = import_repository::complete(&ctx.pool, ctx.import_id).await {
        tracing::error!(import_id = ctx.import_id, "chiusura import fallita: {e}");
    }
}

/// Espande un file caricato ed elabora ogni elemento prodotto.
async fn process_upload(ctx: &ImportContext, file: UploadedFile) {
    let UploadedFile { filename, path } = file;

    let (tx, mut rx) = mpsc::channel::<Expanded>(CHANNEL_CAPACITY);
    let source = path.to_path_buf();
    let limits = ctx.limits.clone();
    let upload_name = filename.clone();
    let expansion = tokio::task::spawn_blocking(move || {
        expand_upload(&upload_name, &source, &limits, &mut |item| {
            // Il ricevitore vive finché il canale non è chiuso: l'invio non fallisce
            let _ = tx.blocking_send(item);
        });
    });

    while let Some(item) = rx.recv().await {
        process_item(ctx, item).await;
    }

    if let Err(e) = expansion.await {
        tracing::error!(import_id = ctx.import_id, "espansione dell'upload fallita: {e}");
        record_failure(ctx, &filename, "INTERNAL").await;
    }

    if let Err(e) = path.close() {
        tracing::warn!(import_id = ctx.import_id, "rimozione file temporaneo fallita: {e}");
    }
}

/// Crea l'item di un elemento espanso e ne registra l'esito.
async fn process_item(ctx: &ImportContext, item: Expanded) {
    let file = match item {
        Expanded::Rejected { filename, code } => {
            record_failure(ctx, &filename, code).await;
            return;
        }
        Expanded::File(file) => file,
    };

    let item_id = match import_repository::insert_item(
        &ctx.pool,
        ctx.import_id,
        &truncate_filename(&file.filename),
        ItemStatus::Pending,
        None,
    )
    .await
    {
        Ok(id) => id,
        Err(e) => {
            tracing::error!(import_id = ctx.import_id, "creazione item fallita: {e}");
            return;
        }
    };

    let outcome = import_file(ctx, file).await.unwrap_or_else(|e| {
        tracing::error!(import_id = ctx.import_id, item_id, "import del file fallito: {e}");
        Outcome::Failed("INTERNAL")
    });

    let (status, activity_id, error_code) = match outcome {
        Outcome::Imported(id) => (ItemStatus::Imported, Some(id), None),
        Outcome::Duplicate(id) => (ItemStatus::Duplicate, Some(id), None),
        Outcome::Failed(code) => (ItemStatus::Failed, None, Some(code)),
    };
    if let Err(e) =
        import_repository::finish_item(&ctx.pool, item_id, status, activity_id, error_code).await
    {
        tracing::error!(import_id = ctx.import_id, item_id, "aggiornamento item fallito: {e}");
    }
}

/// Registra direttamente un item `failed` con il codice indicato.
async fn record_failure(ctx: &ImportContext, filename: &str, code: &'static str) {
    if let Err(e) = import_repository::insert_item(
        &ctx.pool,
        ctx.import_id,
        &truncate_filename(filename),
        ItemStatus::Failed,
        Some(code),
    )
    .await
    {
        tracing::error!(import_id = ctx.import_id, "creazione item fallita: {e}");
    }
}

/// Deduplica, parsing, salvataggio e inserimento di un singolo file.
async fn import_file(ctx: &ImportContext, file: InputFile) -> Result<Outcome, AppError> {
    let InputFile { format, bytes, .. } = file;

    // Hash e parsing sono CPU-bound: fuori dai thread del runtime async
    let (bytes, sha) = tokio::task::spawn_blocking(move || {
        let sha = storage::sha256_hex(&bytes);
        (bytes, sha)
    })
    .await
    .map_err(|e| AppError::Internal(format!("calcolo hash: {e}")))?;

    if let Some(id) = activity_repository::find_by_hash(&ctx.pool, ctx.user_id, &sha).await? {
        return Ok(Outcome::Duplicate(id));
    }

    let (bytes, parsed) = tokio::task::spawn_blocking(move || {
        let parsed = parse(format, &bytes);
        (bytes, parsed)
    })
    .await
    .map_err(|e| AppError::Internal(format!("parsing: {e}")))?;
    let activity = match parsed {
        Ok(activity) => activity,
        Err(e) => return Ok(Outcome::Failed(e.code())),
    };

    if let Some(id) = activity_repository::find_overlapping(
        &ctx.pool,
        ctx.user_id,
        activity.sport.as_str(),
        activity.start_time,
        DUPLICATE_TOLERANCE_S,
    )
    .await?
    {
        return Ok(Outcome::Duplicate(id));
    }

    let file_path = storage::save(&ctx.upload_dir, ctx.user_id, &sha, format, &bytes)
        .await
        .map_err(|e| AppError::Internal(format!("salvataggio file: {e}")))?;

    match activity_repository::insert(&ctx.pool, ctx.user_id, &activity, format, &file_path, &sha)
        .await
    {
        Ok(id) => Ok(Outcome::Imported(id)),
        // Stesso file importato nel frattempo da un altro upload: il file su
        // disco ha lo stesso percorso di quello dell'attività esistente.
        Err(AppError::Coded { code, .. }) if code == activity_repository::DUPLICATE_FILE => {
            activity_repository::find_by_hash(&ctx.pool, ctx.user_id, &sha)
                .await?
                .map(Outcome::Duplicate)
                .ok_or_else(|| AppError::Internal("attività duplicata non trovata".to_string()))
        }
        Err(e) => Err(e),
    }
}

/// Nome file limitato a `MAX_FILENAME_CHARS` caratteri.
fn truncate_filename(name: &str) -> String {
    name.chars().take(MAX_FILENAME_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_filename_counts_chars_not_bytes() {
        let long = "è".repeat(300);
        let truncated = truncate_filename(&long);
        assert_eq!(truncated.chars().count(), MAX_FILENAME_CHARS);
        assert_eq!(truncate_filename("run.gpx"), "run.gpx");
    }
}
