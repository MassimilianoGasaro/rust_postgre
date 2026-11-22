/// services/import_service.rs — Logica di business degli import di file
///
/// Un upload viene registrato come import `processing` e restituito subito;
/// l'elaborazione avviene in background (`tokio::spawn`), un file caricato
/// alla volta. Ogni utente ha al massimo un import `processing` (un altro
/// upload è `409 IMPORT_IN_PROGRESS`) e l'elaborazione parte solo con uno dei
/// permessi globali (`MAX_CONCURRENT_IMPORTS`): in attesa l'import resta
/// `processing`.
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
/// Un supervisore chiude l'import in ogni caso (anche se l'elaborazione va in
/// panic), con alcuni tentativi: a elaborazione avviata un import non resta
/// mai `processing` finché il server è attivo.
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use sqlx::PgPool;
use tempfile::TempPath;
use tokio::sync::{Semaphore, mpsc};

use crate::dto::import::ImportDto;
use crate::errors::AppError;
use crate::middleware::AuthUser;
use crate::models::import::ItemStatus;
use crate::parsing::archive::{ArchiveLimits, Expanded, InputFile, expand_upload};
use crate::parsing::parse;
use crate::repositories::{activity_repository, import_repository};
use crate::storage;

/// Tolleranza sull'orario di inizio per considerare due attività la stessa.
const DUPLICATE_TOLERANCE_S: i64 = 60;

/// Lunghezza massima (in caratteri) del nome file salvato negli item.
const MAX_FILENAME_CHARS: usize = 255;

/// Numero di import restituiti da `list`.
const LIST_LIMIT: i64 = 20;

/// Tentativi per le scritture a DB che registrano l'esito (item, chiusura import).
const WRITE_ATTEMPTS: u32 = 3;

/// Attesa prima del secondo tentativo; raddoppia a ogni tentativo successivo.
const RETRY_BASE_DELAY: Duration = Duration::from_millis(100);

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

/// `409 IMPORT_IN_PROGRESS` se l'utente ha già un import `processing`.
///
/// Controllo preventivo (evita di ricevere un upload che verrebbe rifiutato);
/// il vincolo definitivo è applicato da `import_repository::create`.
pub async fn ensure_no_import_in_progress(pool: &PgPool, user_id: i64) -> Result<(), AppError> {
    if import_repository::has_processing(pool, user_id).await? {
        return Err(AppError::coded(
            StatusCode::CONFLICT,
            import_repository::IMPORT_IN_PROGRESS,
            "Un import è già in corso: attendi che termini",
        ));
    }
    Ok(())
}

/// Registra un import `processing` e ne avvia l'elaborazione in background.
///
/// I file temporanei passano al task di background, che li elimina.
/// L'elaborazione attende uno dei `permits` globali.
pub async fn start_import(
    pool: &PgPool,
    upload_dir: &Path,
    limits: ArchiveLimits,
    permits: Arc<Semaphore>,
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
    tokio::spawn(supervise(
        pool.clone(),
        import.id,
        permits,
        run_import(ctx, files),
    ));

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

/// Attende un permesso, esegue `work` in un task separato e, comunque termini
/// (anche in panic), chiude l'import come `completed`.
///
/// Il permesso è tenuto per tutta l'elaborazione e rilasciato al suo termine.
async fn supervise<F>(pool: PgPool, import_id: i64, permits: Arc<Semaphore>, work: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    match permits.acquire_owned().await {
        Ok(permit) => {
            let task = tokio::spawn(async move {
                let _permit = permit;
                work.await;
            });
            if let Err(e) = task.await {
                tracing::error!(import_id, "elaborazione dell'import interrotta: {e}");
            }
        }
        // Il semaforo non viene mai chiuso: caso solo difensivo
        Err(e) => tracing::error!(import_id, "permesso di elaborazione non ottenuto: {e}"),
    }

    let completed = with_retry(|| import_repository::complete(&pool, import_id)).await;
    if let Err(e) = completed {
        tracing::error!(import_id, "chiusura import fallita: {e}");
    }
}

/// Elabora i file caricati uno alla volta (la chiusura spetta a `supervise`).
async fn run_import(ctx: ImportContext, files: Vec<UploadedFile>) {
    for file in files {
        process_upload(&ctx, file).await;
    }
}

/// Ripete un'operazione a DB fino a `WRITE_ATTEMPTS` volte, con attesa crescente.
async fn with_retry<T, F, Fut>(mut op: F) -> Result<T, AppError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, AppError>>,
{
    let mut delay = RETRY_BASE_DELAY;
    let mut attempt = 1;
    loop {
        match op().await {
            Ok(value) => return Ok(value),
            Err(e) if attempt >= WRITE_ATTEMPTS => return Err(e),
            Err(e) => {
                tracing::warn!(attempt, "scrittura a DB fallita, nuovo tentativo: {e}");
                tokio::time::sleep(delay).await;
                delay *= 2;
                attempt += 1;
            }
        }
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
        tracing::error!(
            import_id = ctx.import_id,
            "espansione dell'upload fallita: {e}"
        );
        record_failure(ctx, &filename, "INTERNAL").await;
    }

    if let Err(e) = path.close() {
        tracing::warn!(
            import_id = ctx.import_id,
            "rimozione file temporaneo fallita: {e}"
        );
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

    let filename = truncate_filename(&file.filename);
    let item_id = match with_retry(|| {
        import_repository::insert_item(
            &ctx.pool,
            ctx.import_id,
            &filename,
            ItemStatus::Pending,
            None,
        )
    })
    .await
    {
        Ok(id) => id,
        Err(e) => {
            tracing::error!(import_id = ctx.import_id, "creazione item fallita: {e}");
            return;
        }
    };

    let outcome = import_file(ctx, file).await.unwrap_or_else(|e| {
        tracing::error!(
            import_id = ctx.import_id,
            item_id,
            "import del file fallito: {e}"
        );
        Outcome::Failed("INTERNAL")
    });

    let (status, activity_id, error_code) = match outcome {
        Outcome::Imported(id) => (ItemStatus::Imported, Some(id), None),
        Outcome::Duplicate(id) => (ItemStatus::Duplicate, Some(id), None),
        Outcome::Failed(code) => (ItemStatus::Failed, None, Some(code)),
    };
    // Se fallisce anche l'ultimo tentativo l'item resta `pending`:
    // la chiusura dell'import lo trasforma in `failed`/`INTERNAL`
    let finished = with_retry(|| {
        import_repository::finish_item(&ctx.pool, item_id, status, activity_id, error_code)
    })
    .await;
    if let Err(e) = finished {
        tracing::error!(
            import_id = ctx.import_id,
            item_id,
            "aggiornamento item fallito: {e}"
        );
    }
}

/// Registra direttamente un item `failed` con il codice indicato.
async fn record_failure(ctx: &ImportContext, filename: &str, code: &'static str) {
    let filename = truncate_filename(filename);
    let inserted = with_retry(|| {
        import_repository::insert_item(
            &ctx.pool,
            ctx.import_id,
            &filename,
            ItemStatus::Failed,
            Some(code),
        )
    })
    .await;
    if let Err(e) = inserted {
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

    // Controllo rapido senza lock: evita il parsing dei file già importati
    // (ripetuto sotto lock prima dell'inserimento)
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

    // Controlli dei duplicati e inserimento in un'unica transazione, sotto un
    // lock per utente: due import concorrenti della stessa attività (anche da
    // file diversi) non possono inserirla entrambi.
    let mut tx = ctx.pool.begin().await?;
    activity_repository::lock_user_for_import(&mut *tx, ctx.user_id).await?;

    if let Some(id) = activity_repository::find_by_hash(&mut *tx, ctx.user_id, &sha).await? {
        return Ok(Outcome::Duplicate(id));
    }
    if let Some(id) = activity_repository::find_overlapping(
        &mut *tx,
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

    // Con il lock la violazione di unicità non dovrebbe verificarsi; se accade
    // comunque (es. inserimento fuori da questo flusso) è un duplicato.
    let outcome = match activity_repository::insert(
        &mut *tx,
        ctx.user_id,
        &activity,
        format,
        &file_path,
        &sha,
    )
    .await
    {
        Ok(id) => Outcome::Imported(id),
        Err(e) if is_duplicate_file(&e) => {
            // La transazione è annullata dall'errore: si rilegge fuori
            drop(tx);
            return activity_repository::find_by_hash(&ctx.pool, ctx.user_id, &sha)
                .await?
                .map(Outcome::Duplicate)
                .ok_or_else(|| AppError::Internal("attività duplicata non trovata".to_string()));
        }
        Err(e) => return Err(e),
    };
    tx.commit().await?;
    Ok(outcome)
}

/// `true` se l'errore è la violazione di unicità (utente, SHA-256) di `insert`.
fn is_duplicate_file(e: &AppError) -> bool {
    matches!(e, AppError::Coded { code, .. } if *code == activity_repository::DUPLICATE_FILE)
}

/// Nome file limitato a `MAX_FILENAME_CHARS` caratteri.
fn truncate_filename(name: &str) -> String {
    name.chars().take(MAX_FILENAME_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    use super::*;

    /// Crea un utente e un import `processing`; ritorna `(user_id, import_id)`.
    async fn processing_import(pool: &PgPool) -> (i64, i64) {
        let user_id: i64 = sqlx::query_scalar(
            "INSERT INTO users (email, username, password_hash, role)              VALUES ('a@example.com', 'a', 'x', 'athlete') RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        let import = import_repository::create(pool, user_id).await.unwrap();
        (user_id, import.id)
    }

    /// L'elaborazione parte solo con un permesso; nel frattempo l'import resta `processing`.
    #[sqlx::test]
    async fn supervise_waits_for_a_permit(pool: PgPool) {
        let (user_id, import_id) = processing_import(&pool).await;
        let permits = Arc::new(Semaphore::new(0));
        let started = Arc::new(AtomicBool::new(false));

        let flag = started.clone();
        let handle = tokio::spawn(supervise(
            pool.clone(),
            import_id,
            permits.clone(),
            async move {
                flag.store(true, Ordering::SeqCst);
            },
        ));

        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!started.load(Ordering::SeqCst), "avviato senza permesso");
        let import = import_repository::find_for_user(&pool, import_id, user_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(import.status, "processing");

        permits.add_permits(1);
        handle.await.unwrap();
        assert!(started.load(Ordering::SeqCst));
        let import = import_repository::find_for_user(&pool, import_id, user_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(import.status, "completed");
        // Il permesso è restituito a fine elaborazione
        assert_eq!(permits.available_permits(), 1);
    }

    /// Anche se l'elaborazione va in panic l'import viene chiuso.
    #[sqlx::test]
    async fn supervise_completes_import_when_work_panics(pool: PgPool) {
        let (user_id, import_id) = processing_import(&pool).await;
        import_repository::insert_item(&pool, import_id, "a.gpx", ItemStatus::Pending, None)
            .await
            .unwrap();

        let permits = Arc::new(Semaphore::new(1));
        supervise(pool.clone(), import_id, permits.clone(), async {
            panic!("elaborazione interrotta (test)");
        })
        .await;
        assert_eq!(permits.available_permits(), 1);

        let import = import_repository::find_for_user(&pool, import_id, user_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(import.status, "completed");
        assert!(import.finished_at.is_some());
        let items = import_repository::list_items(&pool, import.id)
            .await
            .unwrap();
        assert_eq!(items[0].status, "failed");
        assert_eq!(items[0].error_code.as_deref(), Some("INTERNAL"));
    }

    #[tokio::test]
    async fn with_retry_retries_until_success() {
        let calls = AtomicU32::new(0);
        let result = with_retry(|| async {
            if calls.fetch_add(1, Ordering::SeqCst) < WRITE_ATTEMPTS - 1 {
                Err(AppError::Internal("transitorio".into()))
            } else {
                Ok(7)
            }
        })
        .await;
        assert_eq!(result.unwrap(), 7);
        assert_eq!(calls.load(Ordering::SeqCst), WRITE_ATTEMPTS);
    }

    #[tokio::test]
    async fn with_retry_gives_up_after_max_attempts() {
        let calls = AtomicU32::new(0);
        let result: Result<(), AppError> = with_retry(|| async {
            calls.fetch_add(1, Ordering::SeqCst);
            Err(AppError::Internal("permanente".into()))
        })
        .await;
        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), WRITE_ATTEMPTS);
    }

    #[test]
    fn truncate_filename_counts_chars_not_bytes() {
        let long = "è".repeat(300);
        let truncated = truncate_filename(&long);
        assert_eq!(truncated.chars().count(), MAX_FILENAME_CHARS);
        assert_eq!(truncate_filename("run.gpx"), "run.gpx");
    }
}
