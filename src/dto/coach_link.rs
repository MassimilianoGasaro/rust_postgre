/// dto/coach_link.rs — DTO per le relazioni allenatore-atleta
///
/// Espongono solo id e username delle parti: mai email né altri dati personali.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::models::coach_link::{CoachLink, LinkedAthlete};

/// Body di `POST /coach-links`: email o username del coach.
#[derive(Debug, Deserialize)]
pub struct CreateCoachLinkRequest {
    pub coach: String,
}

/// Query string di `GET /coach-links`.
#[derive(Debug, Deserialize)]
pub struct ListCoachLinksQuery {
    pub status: Option<String>,
}

/// Riferimento sintetico a un utente.
#[derive(Debug, Serialize)]
pub struct UserRef {
    pub id: i64,
    pub username: String,
}

/// Relazione allenatore-atleta.
#[derive(Debug, Serialize)]
pub struct CoachLinkDto {
    pub id: i64,
    pub athlete: UserRef,
    pub coach: UserRef,
    pub status: String,
    pub requested_at: DateTime<Utc>,
    pub responded_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
}

impl From<CoachLink> for CoachLinkDto {
    fn from(l: CoachLink) -> Self {
        CoachLinkDto {
            id: l.id,
            athlete: UserRef {
                id: l.athlete_id,
                username: l.athlete_username,
            },
            coach: UserRef {
                id: l.coach_id,
                username: l.coach_username,
            },
            status: l.status,
            requested_at: l.requested_at,
            responded_at: l.responded_at,
            ended_at: l.ended_at,
        }
    }
}

/// Elemento di `GET /athletes`: un atleta collegato al coach.
#[derive(Debug, Serialize)]
pub struct LinkedAthleteDto {
    pub id: i64,
    pub username: String,
    pub linked_since: DateTime<Utc>,
}

impl From<LinkedAthlete> for LinkedAthleteDto {
    fn from(a: LinkedAthlete) -> Self {
        LinkedAthleteDto {
            id: a.id,
            username: a.username,
            linked_since: a.linked_since,
        }
    }
}
