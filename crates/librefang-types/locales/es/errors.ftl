# --- API error messages (Spanish) ---

# Agent errors
api-error-agent-not-found = Agente no encontrado
api-error-agent-spawn-failed = Error al crear el agente
api-error-agent-invalid-id = ID de agente no valido
api-error-agent-already-exists = El agente ya existe

# Message errors
api-error-message-too-large = Mensaje demasiado grande (max 64KB)
api-error-message-delivery-failed = Error al enviar el mensaje: { $reason }

# Template errors
api-error-template-invalid-name = Nombre de plantilla no valido
api-error-template-not-found = Plantilla '{ $name }' no encontrada
api-error-template-parse-failed = Error al analizar la plantilla: { $error }
api-error-template-required = Se requiere 'manifest_toml' o 'template'

# Manifest errors
api-error-manifest-too-large = Manifiesto demasiado grande (max 1MB)
api-error-manifest-invalid-format = Formato de manifiesto no valido
api-error-manifest-signature-mismatch = El contenido del manifiesto firmado no coincide con manifest_toml
api-error-manifest-signature-failed = Verificacion de firma del manifiesto fallida

# Auth errors
api-error-auth-invalid-key = Clave API no valida
api-error-auth-missing-header = Falta el encabezado Authorization: Bearer <api_key>
api-error-auth-missing = La clave API de este proveedor no esta configurada

# Session errors
api-error-session-load-failed = Error al cargar la sesion
api-error-session-not-found = Sesion no encontrada

# Workflow errors
api-error-workflow-missing-steps = Falta el arreglo 'steps'
api-error-workflow-step-needs-agent = El paso '{ $step }' necesita 'agent_id' o 'agent_name'
api-error-workflow-invalid-id = ID de flujo de trabajo no valido
api-error-workflow-execution-failed = Error en la ejecucion del flujo de trabajo

# Trigger errors
api-error-trigger-missing-agent-id = Falta 'agent_id'
api-error-trigger-invalid-agent-id = agent_id no valido
api-error-trigger-invalid-pattern = Patron de activador no valido
api-error-trigger-missing-pattern = Falta 'pattern'
api-error-trigger-registration-failed = Error al registrar el activador (agente no encontrado?)
api-error-trigger-invalid-id = ID de activador no valido
api-error-trigger-not-found = Activador no encontrado

# Budget errors
api-error-budget-invalid-amount = Monto de presupuesto no valido
api-error-budget-update-failed = Error al actualizar el presupuesto

# Config errors
api-error-config-parse-failed = Error al analizar la configuracion: { $error }
api-error-config-write-failed = Error al escribir la configuracion: { $error }

# Profile errors
api-error-profile-not-found = Perfil '{ $name }' no encontrado

# Cron errors
api-error-cron-invalid-id = ID de tarea programada no valido
api-error-cron-not-found = Tarea programada no encontrada
api-error-cron-create-failed = Error al crear la tarea programada: { $error }

# General errors
api-error-not-found = Recurso no encontrado
api-error-internal = Error interno del servidor
api-error-bad-request = Solicitud incorrecta: { $reason }
api-error-rate-limited = Limite de solicitudes excedido. Intente de nuevo mas tarde.
