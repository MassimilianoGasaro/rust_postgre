/// models/import.rs — I Model degli import di file
///
/// Un import corrisponde a un upload (uno o più file, anche ZIP) elaborato in
/// background; ogni file prodotto dall'upload ha il suo `import_item`.
use chrono::{DateTime, Utc};

/// Stato dell'import. Nel DB è la stringa minuscola (vincolo CHECK).
///
/// `failed` è usato solo per gli import interrotti da un riavvio del server:
/// un'elaborazione normale termina sempre `completed`, anche se alcuni file
/// falliscono (l'esito è nei singoli item).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportStatus {
    Processing,
    Completed,
    Failed,
}

impl ImportStatus {
    /// Valore testuale usato nel DB e nelle risposte API.
    pub fn as_str(&self) -> &'static str {
        match self {
            ImportStatus::Processing => "processing",
            ImportStatus::Completed => "completed",
            ImportStatus::Failed => "failed",
        }
    }
}

/// Stato di un singolo file dell'import.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemStatus {
    /// In elaborazione
    Pending,
    /// Attività creata
    Imported,
    /// Attività già presente (stesso file o stessa attività da un altro file)
    Duplicate,
    /// Errore: il motivo è in `error_code`
    Failed,
}

impl ItemStatus {
    /// Valore testuale usato nel DB e nelle risposte API.
    pub fn as_str(&self) -> &'static str {
        match self {
            ItemStatus::Pending => "pending",
            ItemStatus::Imported => "imported",
            ItemStatus::Duplicate => "duplicate",
            ItemStatus::Failed => "failed",
        }
    }
}

/// Riga della tabella `imports`.
#[derive(Debug, Clone)]
pub struct Import {
    pub id: i64,
    pub user_id: i64,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// Riga della tabella `import_items`.
#[derive(Debug, Clone)]
pub struct ImportItem {
    pub id: i64,
    pub import_id: i64,
    pub filename: String,
    pub status: String,
    pub activity_id: Option<i64>,
    pub error_code: Option<String>,
}
