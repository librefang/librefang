# --- API error messages (French) ---

# Agent errors
api-error-agent-not-found = Agent non trouvé
api-error-agent-spawn-failed = Échec de la création de l'agent
api-error-agent-invalid-id = ID d'agent non valide
api-error-session-invalid-id = ID de session non valide
api-error-context-report-failed = Échec du rapport de contexte
api-error-agent-already-exists = L'agent existe déjà

# Message errors
api-error-message-too-large = Message trop volumineux (max. 64 Ko)
api-error-message-delivery-failed = Échec de la livraison du message : { $reason }

# Template errors
api-error-template-invalid-name = Nom de modèle non valide
api-error-template-not-found = Modèle '{ $name }' non trouvé
api-error-template-parse-failed = Échec de l'analyse du modèle : { $error }
api-error-template-required = 'manifest_toml' ou 'template' est requis
api-error-template-invalid-manifest = Manifeste de modèle non valide
api-error-template-read-failed = Échec de la lecture du modèle
api-error-agent-type-exists = Un type d'agent nommé '{ $name }' existe déjà
api-error-agent-type-name-taken = '{ $name }' est le nom d'un agent actif ; choisissez un autre nom pour le type d'agent
api-error-agent-type-not-editable = Le type d'agent '{ $name }' provient de l'espace de travail d'un agent actif et se gère via /api/agents

# Manifest errors
api-error-manifest-too-large = Manifeste trop volumineux (max. 1 Mo)
api-error-manifest-invalid-format = Format de manifeste non valide
api-error-manifest-signature-mismatch = Le contenu du manifeste signé ne correspond pas à manifest_toml
api-error-manifest-signature-failed = Échec de la vérification de la signature du manifeste

# Auth errors
api-error-auth-invalid-key = Clé API non valide
api-error-auth-missing-header = En-tête Authorization: Bearer <api_key> manquant
api-error-auth-missing = La clé API de ce fournisseur n'est pas configurée

# Session errors
api-error-session-load-failed = Échec du chargement de la session
api-error-session-not-found = Session non trouvée

# Workflow errors
api-error-workflow-missing-steps = Tableau 'steps' manquant
api-error-workflow-step-needs-agent = L'étape '{ $step }' nécessite 'agent_id' ou 'agent_name'
api-error-workflow-invalid-id = ID de workflow non valide
api-error-workflow-execution-failed = Échec de l'exécution du workflow

# Trigger errors
api-error-trigger-missing-agent-id = 'agent_id' manquant
api-error-trigger-invalid-agent-id = agent_id non valide
api-error-trigger-invalid-pattern = Modèle de déclencheur non valide
api-error-trigger-missing-pattern = 'pattern' manquant
api-error-trigger-registration-failed = Échec de l'enregistrement du déclencheur (agent non trouvé ?)
api-error-trigger-invalid-id = ID de déclencheur non valide
api-error-trigger-not-found = Déclencheur non trouvé

# Budget errors
api-error-budget-invalid-amount = Montant du budget non valide
api-error-budget-update-failed = Échec de la mise à jour du budget

# Config errors
api-error-config-parse-failed = Échec de l'analyse de la configuration : { $error }
api-error-config-write-failed = Échec de l'écriture de la configuration : { $error }

# Profile errors
api-error-profile-not-found = Profil '{ $name }' non trouvé

# Cron errors
api-error-cron-invalid-id = ID de tâche planifiée non valide
api-error-cron-not-found = Tâche planifiée non trouvée
api-error-cron-create-failed = Échec de la création de la tâche planifiée : { $error }

# General errors
api-error-not-found = Ressource non trouvée
api-error-internal = Erreur interne du serveur
api-error-bad-request = Requête invalide : { $reason }
api-error-rate-limited = Limite de requêtes dépassée. Veuillez réessayer plus tard.

# Generic catch-all — interpolates the underlying error string verbatim.
# Used by 41+ HTTP 500 handlers as a stopgap until each route is moved to a
# typed MemoryRouteError-style helper. Without this key, every `t_args("api-error-generic", …)`
# call returns the literal key as the response body and `$error` interpolation never runs.
api-error-generic = Erreur : { $error }
