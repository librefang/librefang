# --- API error messages (French) ---

# Agent errors
api-error-agent-not-found = Agent non trouve
api-error-agent-spawn-failed = Echec de la creation de l'agent
api-error-agent-invalid-id = ID d'agent non valide
api-error-agent-already-exists = L'agent existe deja

# Message errors
api-error-message-too-large = Message trop volumineux (max 64 Ko)
api-error-message-delivery-failed = Echec de la livraison du message : { $reason }

# Template errors
api-error-template-invalid-name = Nom de modele non valide
api-error-template-not-found = Modele '{ $name }' non trouve
api-error-template-parse-failed = Echec de l'analyse du modele : { $error }
api-error-template-required = 'manifest_toml' ou 'template' est requis

# Manifest errors
api-error-manifest-too-large = Manifeste trop volumineux (max 1 Mo)
api-error-manifest-invalid-format = Format de manifeste non valide
api-error-manifest-signature-mismatch = Le contenu du manifeste signe ne correspond pas a manifest_toml
api-error-manifest-signature-failed = Echec de la verification de la signature du manifeste

# Auth errors
api-error-auth-invalid-key = Cle API non valide
api-error-auth-missing-header = En-tete Authorization: Bearer <api_key> manquant
api-error-auth-missing = La cle API de ce fournisseur n'est pas configuree

# Session errors
api-error-session-load-failed = Echec du chargement de la session
api-error-session-not-found = Session non trouvee

# Workflow errors
api-error-workflow-missing-steps = Tableau 'steps' manquant
api-error-workflow-step-needs-agent = L'etape '{ $step }' necessite 'agent_id' ou 'agent_name'
api-error-workflow-invalid-id = ID de workflow non valide
api-error-workflow-execution-failed = Echec de l'execution du workflow

# Trigger errors
api-error-trigger-missing-agent-id = 'agent_id' manquant
api-error-trigger-invalid-agent-id = agent_id non valide
api-error-trigger-invalid-pattern = Modele de declencheur non valide
api-error-trigger-missing-pattern = 'pattern' manquant
api-error-trigger-registration-failed = Echec de l'enregistrement du declencheur (agent non trouve ?)
api-error-trigger-invalid-id = ID de declencheur non valide
api-error-trigger-not-found = Declencheur non trouve

# Budget errors
api-error-budget-invalid-amount = Montant du budget non valide
api-error-budget-update-failed = Echec de la mise a jour du budget

# Config errors
api-error-config-parse-failed = Echec de l'analyse de la configuration : { $error }
api-error-config-write-failed = Echec de l'ecriture de la configuration : { $error }

# Profile errors
api-error-profile-not-found = Profil '{ $name }' non trouve

# Cron errors
api-error-cron-invalid-id = ID de tache planifiee non valide
api-error-cron-not-found = Tache planifiee non trouvee
api-error-cron-create-failed = Echec de la creation de la tache planifiee : { $error }

# General errors
api-error-not-found = Ressource non trouvee
api-error-internal = Erreur interne du serveur
api-error-bad-request = Requete invalide : { $reason }
api-error-rate-limited = Limite de requetes depassee. Veuillez reessayer plus tard.
