# --- API error messages (German) ---

# Agent errors
api-error-agent-not-found = Agent nicht gefunden
api-error-agent-spawn-failed = Agent konnte nicht erstellt werden
api-error-agent-invalid-id = Ungueltige Agent-ID
api-error-agent-already-exists = Agent existiert bereits

# Message errors
api-error-message-too-large = Nachricht zu gross (max 64KB)
api-error-message-delivery-failed = Nachrichtzustellung fehlgeschlagen

# Template errors
api-error-template-invalid-name = Ungueltiger Vorlagenname
api-error-template-not-found = Vorlage '{ $name }' nicht gefunden
api-error-template-parse-failed = Vorlage konnte nicht analysiert werden: { $error }
api-error-template-required = 'manifest_toml' oder 'template' ist erforderlich

# Manifest errors
api-error-manifest-too-large = Manifest zu gross (max 1MB)
api-error-manifest-invalid-format = Ungueltiges Manifest-Format
api-error-manifest-signature-mismatch = Signierter Manifest-Inhalt stimmt nicht mit manifest_toml ueberein
api-error-manifest-signature-failed = Manifest-Signaturpruefung fehlgeschlagen

# Auth errors
api-error-auth-invalid-key = Ungueltiger API-Schluessel
api-error-auth-missing-header = Fehlender Authorization: Bearer <api_key> Header
api-error-auth-missing = API-Schluessel fuer diesen Anbieter nicht konfiguriert

# Session errors
api-error-session-load-failed = Sitzung konnte nicht geladen werden
api-error-session-not-found = Sitzung nicht gefunden

# Workflow errors
api-error-workflow-missing-steps = 'steps'-Array fehlt
api-error-workflow-step-needs-agent = Schritt '{ $step }' benoetigt 'agent_id' oder 'agent_name'
api-error-workflow-invalid-id = Ungueltige Workflow-ID
api-error-workflow-execution-failed = Workflow-Ausfuehrung fehlgeschlagen

# Trigger errors
api-error-trigger-missing-agent-id = 'agent_id' fehlt
api-error-trigger-invalid-agent-id = Ungueltige agent_id
api-error-trigger-invalid-pattern = Ungueltiges Trigger-Muster
api-error-trigger-missing-pattern = 'pattern' fehlt
api-error-trigger-registration-failed = Trigger-Registrierung fehlgeschlagen (Agent nicht gefunden?)
api-error-trigger-invalid-id = Ungueltige Trigger-ID
api-error-trigger-not-found = Trigger nicht gefunden

# Budget errors
api-error-budget-invalid-amount = Ungueltiger Budget-Betrag
api-error-budget-update-failed = Budget-Aktualisierung fehlgeschlagen

# Config errors
api-error-config-parse-failed = Konfiguration konnte nicht analysiert werden: { $error }
api-error-config-write-failed = Konfiguration konnte nicht geschrieben werden: { $error }

# Profile errors
api-error-profile-not-found = Profil '{ $name }' nicht gefunden

# Cron errors
api-error-cron-invalid-id = Ungueltige Cron-Job-ID
api-error-cron-not-found = Cron-Job nicht gefunden
api-error-cron-create-failed = Cron-Job konnte nicht erstellt werden: { $error }

# General errors
api-error-not-found = Ressource nicht gefunden
api-error-internal = Interner Serverfehler
api-error-bad-request = Ungueltige Anfrage: { $reason }
api-error-rate-limited = Anfragelimit ueberschritten. Bitte versuchen Sie es spaeter erneut.
