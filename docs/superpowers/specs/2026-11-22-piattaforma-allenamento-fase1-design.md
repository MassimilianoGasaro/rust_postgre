# Piattaforma di allenamento — Fase 1: fondamenta, import file, dashboard

Data: 2026-11-22
Repo coinvolti: `rust_postgre` (backend, esistente) e `rust_postgre_client` (client, nuovo, stesso livello di cartella, repo indipendente).

Legenda: **[deciso]** = scelto dal proprietario del progetto; **[scelta del designer]** = default proposto durante il design, modificabile; tutto il resto non marcato è lasciato al builder.

## 1. Intento

Un'applicazione per sportivi di endurance che riunisce in **un unico ambiente** i dati oggi sparsi tra Strava, Garmin e Coros. L'app raccoglie attività e (in futuro) dati salute, mostra una dashboard statistica su intervalli di date scelti dall'utente e, in fasi successive, **genera automaticamente un piano di allenamento** basato su un metodo scelto dall'utente (es. metodo norvegese) e sui dati salute giornalieri; l'allenatore può ritoccare il piano generato **[deciso]**. Gli **allenatori** possono supervisionare un atleta solo dopo una richiesta dell'atleta stesso **[deciso]**.

Perché: oggi l'utente deve saltare tra app diverse, nessuna combina storico, stato di forma del giorno e un metodo di allenamento strutturato, e il rapporto con l'allenatore vive fuori da questi strumenti.

## 2. Vincoli esterni sui provider (verificati il 2026-11-22)

- **Strava** (API Policy in vigore dal 2026-06-01): i dati di un utente si possono mostrare **solo a quell'utente**, anche con il suo consenso (niente vista allenatore); vietato usarli per qualsiasi applicazione AI, anche come input; cache massima 7 giorni; cancellazione entro 30 giorni dallo scollegamento; il livello base richiede un abbonamento Strava e copre max 10 atleti senza revisione. → **Strava escluso dalla Fase 1** **[deciso]**; eventualmente in futuro solo per la vista personale dell'atleta e dopo conferma scritta di Strava.
- **Garmin**: programma sviluppatori solo per aziende, alcune metriche salute con licenza a pagamento; **nuove richieste attualmente sospese**.
- **Coros**: API partner per piattaforme già avviate (società registrata, base utenti).

Conseguenza: in Fase 1 la fonte dati è l'**importazione di file** caricati dall'utente **[deciso]**. I dati sono dell'utente e possono essere usati liberamente per dashboard, allenatore e piani. Le integrazioni automatiche si aggiungeranno come ulteriori "fonti" dello stesso modello dati.

Azioni a carico del proprietario (non bloccanti): richiesta API a Coros, monitoraggio della riapertura Garmin, richiesta scritta a developers@strava.com sul caso allenatore.

## 3. Visione complessiva (fasi future, NON in questa fase)

- Integrazioni automatiche Garmin, Coros, eventualmente Strava (OAuth per-utente: ogni utente autorizza il proprio account).
- Dati salute giornalieri (sonno, HRV, FC a riposo, ecc.) e indicazione "come allenarsi oggi".
- Scelta del metodo di allenamento e generazione automatica del piano.
- L'allenatore aggiunge/modifica/cancella allenamenti del piano dell'atleta.
- PWA installabile, refresh token di sessione, stream dettagliati e analisi per singola attività.

Lo schema dati di questa fase deve rendere naturali queste estensioni (attività con campo "fonte", relazione allenatore-atleta con stati, file originali conservati per rielaborarli), ma non le implementa.

## 4. Obiettivi della Fase 1

1. **Pulizia backend** e base solida per crescere.
2. **Ruoli** atleta e allenatore.
3. **Relazione allenatore-atleta** su richiesta dell'atleta.
4. **Importazione attività da file** (FIT, GPX, TCX, singoli o in archivio ZIP).
5. **Statistiche** filtrabili per intervallo di date.
6. **Client Angular** responsive (desktop e mobile allo stesso modo) **[deciso]**, bilingue italiano/inglese **[deciso]**.
7. **Test automatici** su backend (DB di test reale) e client.

## 5. Non-obiettivi (Fase 1)

Integrazioni Strava/Garmin/Coros, piano di allenamento, metodi, dati salute, modifica allenamenti da parte dell'allenatore, mappe/tracce GPS, grafici per singola attività, notifiche, pagamenti, deploy in produzione, e2e browser test, unità imperiali.

## 6. Anti-obiettivi (fallimento anche se "funziona")

- Un allenatore vede dati di un atleta senza una relazione **accettata e non revocata**.
- Un utente autenticato può leggere/modificare/cancellare dati di un altro utente (oggi possibile su `/users/:id`: va chiuso).
- Segreti versionati su git.
- Modello dati delle attività legato a un formato o a un provider, tale da dover essere rifatto per le integrazioni future.
- Lo stesso allenamento caricato due volte (o da due fonti) conta doppio nelle statistiche.
- Un file malformato o enorme blocca o fa cadere il server.
- UI che su telefono è una versione ridotta o scomoda della versione desktop.

## 7. Stato attuale del backend (rilevato)

- Rust edition 2024, axum 0.7, sqlx 0.8 (Postgres), JWT (`jsonwebtoken`), bcrypt, tracing, HTTPS con certificato self-signed generato a runtime se mancano `TLS_CERT_PATH`/`TLS_KEY_PATH`.
- Architettura a strati: `controllers` → `services` → `repositories` → `models`, con `dto` ed `errors` centralizzati. **Da mantenere.**
- Endpoint: `POST /auth/register`, `POST /auth/login`, `GET /users`, `GET|PATCH|DELETE /users/:id` (protetti da JWT ma senza controllo di proprietà).
- Migration: `001_create_users`, `002_create_users_info` (`bio`), `003_create_users_config` (`can_edit`). Non eseguite all'avvio.
- `.env` è in `.gitignore` ma **è tracciato su git** (repo GitHub `MassimilianoGasaro/rust_postgre`).
- `docker-compose.yml` con Postgres 16 (`myuser`/`mypassword`/`myapp`, porta 5432).
- Nessun test presente.

## 8. Backend — requisiti

### 8.1 Pulizia e infrastruttura
- `git rm --cached .env`, aggiungere `.env.example` con tutte le variabili (valori fittizi). Rigenerare `JWT_SECRET` perché il valore attuale è nella history.
- Il DB di sviluppo **può essere svuotato** **[deciso]**: non serve migrare dati esistenti. Le migration 001–003 possono essere riscritte/consolidate liberamente; da qui in poi le migration si aggiungono e non si modificano.
- Migration eseguite all'avvio con `sqlx::migrate!()` **[scelta del designer]**; `sqlx-cli` disponibile per uso manuale.
- Rimuovere `users_config.can_edit` (i permessi dell'allenatore derivano dalla relazione) e sostituire `users_info.bio` con il profilo (sez. 8.2). Allineare i commenti dei model allo schema reale.
- CORS configurabile da env (origine del client).
- Errori API con codice stabile e leggibile dalla macchina (es. `{"error": "...", "code": "COACH_LINK_EXISTS", "status": 409}`) così il client traduce i messaggi in IT/EN **[scelta del designer]**.

### 8.2 Utenti, ruoli, profilo
- `users.role` ∈ {`athlete`, `coach`}, scelto alla registrazione, un solo ruolo per account **[scelta del designer]**.
- Il ruolo entra nei claims JWT; i permessi sono applicati per ruolo.
- Endpoint utente ristretti a sé stessi: `GET|PATCH|DELETE /users/me`. Rimuovere la lista di tutti gli utenti e l'accesso per id altrui.
- Profilo: lingua preferita (`it`/`en`), fuso orario (default `Europe/Rome`), dati fisiologici opzionali utili alle fasi future (data di nascita, sesso, peso, FC max, FC riposo, FTP, passo di soglia). Insieme esatto dei campi lasciato al builder.

### 8.3 Relazione allenatore-atleta
- Stati: `pending` → `active` | `rejected`; `active` → `revoked`; l'atleta può annullare una `pending`. Timestamp di richiesta e risposta.
- L'**atleta** invia la richiesta indicando email o username dell'allenatore **[scelta del designer]**; l'allenatore vede le richieste in arrivo e accetta/rifiuta; l'atleta revoca quando vuole.
- Un atleta ha al massimo **un** allenatore attivo o pendente; un allenatore segue **più** atleti **[scelta del designer]**.
- In Fase 1 l'allenatore con relazione attiva **legge** attività e statistiche dell'atleta (sola lettura) **[scelta del designer]**.

### 8.4 Importazione attività da file
- Formati: **FIT** (principale, prodotto da Garmin, Coros e quasi tutti i dispositivi), **GPX**, **TCX**; anche **ZIP** contenente questi file (es. export completo da Garmin Connect) **[scelta del designer]**.
- Caricamento via upload multipart; l'elaborazione avviene in background: l'upload restituisce un id di importazione, e il client ne interroga lo stato e l'esito **per file** (importato / duplicato / errore con motivo).
- Limiti di dimensione configurabili (per file e per archivio) e protezione contro archivi malevoli (zip bomb, percorsi assoluti/`..`). Un file non valido fallisce da solo senza interrompere gli altri.
- Dal file si calcolano le metriche di riepilogo: sport (normalizzato + valore originale), inizio (UTC + offset locale se presente, altrimenti fuso del profilo), durata totale e in movimento, distanza, dislivello positivo, FC media/max, velocità media, potenza media, cadenza media, calorie quando presenti.
- Il **file originale viene conservato** (su disco in una cartella configurabile da env, riferito dal DB) per poterlo rielaborare in fasi future (stream, zone, piani) **[scelta del designer]**.
- **Duplicati** **[scelta del designer]**: un file identico (stesso hash) già caricato dallo stesso utente viene scartato; un'attività dello stesso utente con stesso sport e inizio entro ±60 secondi è considerata la stessa attività e segnalata come duplicato. La stessa regola servirà quando arriveranno più fonti.
- L'atleta può cancellare una propria attività (con il file originale).
- Le attività hanno un campo **fonte** (`file_upload` ora; in futuro `garmin`, `coros`, `strava`).

### 8.5 Statistiche
- Riepilogo con parametri `from`, `to` (date, inclusive) e filtro opzionale per sport: totali (numero attività, distanza, tempo, dislivello), ripartizione per sport, serie temporale aggregata (giorno/settimana/mese, scelta dal client o automatica).
- Elenco attività paginato con gli stessi filtri.
- Aggregazione nel fuso orario dell'utente.
- Stessi endpoint utilizzabili dall'allenatore per un proprio atleta attivo.

### 8.6 Test backend
- Test di integrazione HTTP con `#[sqlx::test]` (database temporaneo isolato per test, migration applicate) e chiamate al router in-process.
- File di esempio FIT/GPX/TCX/ZIP piccoli versionati come fixture di test (generati o pubblicamente utilizzabili), più casi malformati.
- Copertura minima: registrazione/login per entrambi i ruoli; accesso negato ai dati altrui; ciclo di vita completo della relazione allenatore-atleta e accesso dell'allenatore solo con relazione attiva; import per ogni formato; duplicati; file malformato e archivio malevolo; statistiche su intervalli e confini di fuso orario.

## 9. Client — requisiti

- Repo `rust_postgre_client` in `<cartella repos>\`, su GitHub con l'utente `MassimilianoGasaro` **[deciso]**.
- **Angular** (ultima versione stabile), standalone components, TypeScript strict **[scelta del designer]**.
- **Responsive** mobile e desktop a parità di funzioni **[deciso]**.
- **Bilingue IT/EN** con cambio lingua a runtime (libreria tipo Transloco), lingua iniziale dal profilo o dal browser **[deciso: bilingue; libreria: scelta del designer]**.
- UI: Angular Material; grafici: ECharts (ngx-echarts) **[scelta del designer]**.
- In sviluppo il client chiama il backend tramite il proxy del dev server Angular (`/api` → `https://localhost:3000`, certificato self-signed accettato dal proxy) **[scelta del designer]**.
- Pagine:
  - Login, registrazione (con scelta ruolo).
  - **Atleta**: dashboard (selettore intervallo con scorciatoie 7/30/90 giorni/anno + intervallo libero; totali; grafico andamento; ripartizione per sport; elenco attività con cancellazione); importazione (trascina-e-rilascia o selezione di più file/ZIP, avanzamento ed esito per file, istruzioni brevi su come esportare da Garmin Connect e Coros); profilo; allenatore (invia richiesta, stato, annulla/revoca).
  - **Allenatore**: richieste in arrivo (accetta/rifiuta), elenco atleti, dashboard in sola lettura di un atleta.
- JWT con guard per rotte e ruolo; logout su 401.
- Test unitari con il test runner di default di Angular su servizi, guard e componenti principali.

## 10. Ambiente locale

- `docker compose up -d` nel repo backend avvia Postgres 16 su `localhost:5432`.
- DB di sviluppo `myapp`; i test usano database temporanei creati da `#[sqlx::test]` sullo stesso server (l'utente `myuser` del container ha i permessi necessari).
- Strumenti presenti: Docker 29, cargo 1.98, Node 22.
- Il repo client viene creato su GitHub (stesso account del backend); visibilità pubblica/privata da confermare al momento della creazione.

## 11. Lasciato al builder

Struttura esatta delle tabelle e nomi delle colonne, scelta delle crate (parsing FIT/GPX/TCX, ZIP, multipart), forma esatta di DTO ed endpoint nel rispetto dei requisiti, meccanismo del job di import in background, valori dei limiti di dimensione, struttura delle cartelle del client, design visivo.
