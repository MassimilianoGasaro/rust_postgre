-- migrations/0002_coach_links.sql
-- Relazione allenatore-atleta: l'atleta invia la richiesta, il coach la accetta o rifiuta.
-- Stati: pending → active | rejected | cancelled ; active → revoked.

CREATE TABLE coach_links (
    id           BIGSERIAL   PRIMARY KEY,
    athlete_id   BIGINT      NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    coach_id     BIGINT      NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    status       TEXT        NOT NULL CHECK (status IN ('pending', 'active', 'rejected', 'cancelled', 'revoked')),
    requested_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    responded_at TIMESTAMPTZ,
    ended_at     TIMESTAMPTZ
);

-- Un atleta ha al massimo UN link aperto (pending o active) alla volta.
-- È il database a garantirlo, così due richieste concorrenti non possono
-- creare due link: la seconda fallisce con violazione di unicità.
CREATE UNIQUE INDEX coach_links_one_open ON coach_links(athlete_id) WHERE status IN ('pending', 'active');

-- Lookup lato coach: elenco richieste/atleti e verifica dell'accesso.
CREATE INDEX coach_links_coach_idx ON coach_links(coach_id, status);
