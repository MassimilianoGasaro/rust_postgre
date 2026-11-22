/// models/coach_link.rs — Il Model della relazione allenatore-atleta
///
/// Rispecchia la tabella `coach_links`, arricchita con gli username delle
/// due parti (join con `users`) perché servono sempre insieme al link.

use chrono::{DateTime, Utc};

/// Stato del link. Nel DB è la stringa minuscola (vincolo CHECK).
///
/// Transizioni ammesse:
///   pending → active    (il coach accetta)
///   pending → rejected  (il coach rifiuta)
///   pending → cancelled (l'atleta annulla)
///   active  → revoked   (l'atleta revoca)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkStatus {
    Pending,
    Active,
    Rejected,
    Cancelled,
    Revoked,
}

impl LinkStatus {
    /// Valore testuale usato nel DB e nelle risposte API.
    pub fn as_str(&self) -> &'static str {
        match self {
            LinkStatus::Pending => "pending",
            LinkStatus::Active => "active",
            LinkStatus::Rejected => "rejected",
            LinkStatus::Cancelled => "cancelled",
            LinkStatus::Revoked => "revoked",
        }
    }

    /// Operazione inversa di `as_str`; `None` se il valore non è uno stato noto.
    pub fn parse(value: &str) -> Option<LinkStatus> {
        match value {
            "pending" => Some(LinkStatus::Pending),
            "active" => Some(LinkStatus::Active),
            "rejected" => Some(LinkStatus::Rejected),
            "cancelled" => Some(LinkStatus::Cancelled),
            "revoked" => Some(LinkStatus::Revoked),
            _ => None,
        }
    }
}

/// Riga di `coach_links` con gli username di atleta e coach.
#[derive(Debug, Clone)]
pub struct CoachLink {
    pub id: i64,
    pub athlete_id: i64,
    pub athlete_username: String,
    pub coach_id: i64,
    pub coach_username: String,
    pub status: String,
    pub requested_at: DateTime<Utc>,
    pub responded_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
}

/// Atleta collegato a un coach (link `active`).
#[derive(Debug, Clone)]
pub struct LinkedAthlete {
    pub id: i64,
    pub username: String,
    /// Momento dell'accettazione della richiesta.
    pub linked_since: DateTime<Utc>,
}
