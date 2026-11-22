-- migrations/0004_imports_one_processing.sql
-- Al massimo un import `processing` per utente: un nuovo upload mentre il
-- precedente è in elaborazione viene rifiutato (409 IMPORT_IN_PROGRESS).
-- L'indice parziale rende il controllo atomico anche tra richieste concorrenti.

CREATE UNIQUE INDEX imports_one_processing_per_user
    ON imports(user_id)
    WHERE status = 'processing';
