# --- Daemon lifecycle ---
daemon-starting = Iniciando el daemon...
daemon-stopped = Daemon de LibreFang detenido.
kernel-booted = Kernel iniciado ({ $provider }/{ $model })
models-available = { $count } modelos disponibles
agents-loaded = { $count } agente(s) cargado(s)
daemon-started-bg = Daemon iniciado en segundo plano
daemon-still-starting = Daemon lanzado en segundo plano y todavía iniciando
daemon-stopped-ok = Daemon detenido
daemon-stopped-forced = Daemon detenido (forzado)
daemon-stop-waiting = Esperando a que el daemon (PID { $pid }) termine de apagarse...
daemon-stop-kill-failed = El daemon (PID { $pid }) sigue en ejecución tras un kill forzado
daemon-stop-kill-failed-fix = Detén el proceso { $pid } a mano y vuelve a ejecutar el comando
daemon-stop-timeout = El daemon no se detuvo en { $secs }s
daemon-error = Error del daemon: { $error }
daemon-already-running = El daemon ya se ejecuta en { $url }
daemon-already-running-fix = Usa `librefang status` para comprobarlo, o deténlo primero
daemon-not-running-start = El daemon no se ejecuta. Inícialo con: librefang start
daemon-no-running-found = No se encontró ningún daemon en ejecución
daemon-no-running-found-fix = ¿Está en ejecución? Compruébalo con: librefang status
daemon-restarting = Reiniciando el daemon...
daemon-no-running-starting = No se encontró ningún daemon en ejecución; iniciando uno nuevo
daemon-bg-exited = El daemon en segundo plano terminó antes de estar listo ({ $status })
daemon-bg-exited-fix = Revisa los registros de arranque: { $path }
daemon-bg-wait-fail = Error al esperar al daemon en segundo plano
daemon-bg-wait-fail-fix = { $error }. Revisa los registros de arranque: { $path }
daemon-launch-fail = Error al lanzar el daemon en segundo plano
daemon-no-running-auto = No hay ningún daemon en ejecución; iniciando uno ahora...
daemon-started = Daemon iniciado
daemon-start-fail = No se pudo iniciar el daemon: { $error }
daemon-start-fail-fix = Inícialo manualmente: librefang start
shutdown-request-fail = La solicitud de apagado falló ({ $status })
could-not-reach-daemon = No se pudo contactar con el daemon: { $error }
# Issue #4693 — after `curl install.sh | sh` upgrades the binary without
# restarting the running daemon, `librefang restart` (new CLI) hits the old
# daemon's `/api/shutdown` and is rejected with 401 because the new CLI's
# Authorization header does not match the old daemon's expected key (typical
# trigger: locked vault, rotated `[api] api_key`, or freshly enabled
# dashboard credentials). Surface the cause + auto-fall-back to PID-based
# shutdown so users can move forward without hand-editing config.
shutdown-401-detected = El daemon en ejecución rechazó la solicitud de apagado (401 Unauthorized).
shutdown-401-explainer = La nueva CLI no puede autenticarse contra el daemon que se está ejecutando. Suele ocurrir después de que `curl install.sh | sh` actualice el binario sin reiniciar el daemon: el daemon en ejecución se inició con otro api_key, o no se pudo desbloquear la bóveda que lo guarda.
shutdown-401-fallback-attempt = Recurriendo a una parada por PID (PID { $pid })...
shutdown-401-fallback-success = Daemon detenido mediante el PID { $pid }
shutdown-401-fallback-fail = La parada por PID tampoco funcionó.
shutdown-401-fallback-fix = Detén el daemon a mano y arráncalo de nuevo:
    kill { $pid }    # o: kill -9 { $pid } si no termina
    librefang start
shutdown-401-no-pid-fix = No se pudo leer el PID del daemon desde { $path }. Ejecuta `ps -ef | grep librefang` para encontrarlo, luego `kill <pid>` y `librefang start`.

# --- Labels ---
label-api = API
label-dashboard = Panel
label-provider = Proveedor
label-model = Modelo
label-pid = PID
label-log = Registro
label-status = Estado
label-agents = Agentes
label-data-dir = Directorio de datos
label-uptime = Tiempo activo
label-version = Versión
label-daemon = Daemon
label-id = ID
label-active-agents = Agentes activos
label-pairing-code = Código de emparejamiento
label-expires = Caduca
label-yes = sí
label-no = no
label-not-loaded = no cargado
label-current = Actual
label-channel = Canal
label-binary = Binario
label-latest = Última
label-target = Destino
label-installed = Instalado

# --- Hints ---
hint-open-dashboard = Abre el panel en el navegador, o ejecuta `librefang chat`
hint-stop-daemon = Usa `librefang stop` para detener el daemon
hint-tail-stop = Ctrl+C deja de seguir el registro; el daemon sigue en ejecución
hint-check-status = Ejecuta `librefang status` para comprobar que está listo
hint-start-daemon = Inícialo con: librefang start
hint-start-daemon-cmd = Inicia el daemon: librefang start
hint-or-chat = O prueba `librefang chat`, que funciona sin daemon
hint-non-interactive = Terminal no interactiva detectada; ejecutando en modo rápido
hint-non-interactive-wizard = Para el asistente interactivo, ejecuta: librefang init (en una terminal)
hint-starting-chat = Iniciando sesión de chat...
hint-no-api-keys = No se encontraron claves API de proveedores LLM
hint-groq-free = Groq ofrece un nivel gratuito: https://console.groq.com
hint-ollama-local = O instala Ollama para modelos locales: https://ollama.com
hint-gemini-free = Gemini ofrece un nivel gratuito: https://aistudio.google.com
hint-deepseek-free = DeepSeek ofrece 5M de tokens gratis: https://platform.deepseek.com
guide-title = Configuración rápida
guide-free-providers-title = Elige un proveedor gratuito para empezar (2 min de configuración):
guide-get-free-key = Consigue tu clave API gratuita
guide-paste-key-placeholder = pega aquí tu clave API
guide-setting-up = Configurando
guide-testing-key = Probando la clave...
guide-key-verified = ✓ ¡Clave verificada!
guide-test-key-unverified = ⚠ No se pudo verificar (puede funcionar igualmente)
guide-help-select = ↑↓ navegar  Enter seleccionar  s/Esc omitir
guide-help-paste = Pega la clave + Enter  Esc volver
guide-help-wait = Espera un momento...
guide-paste-key-hint = Copia la clave API del navegador y pégala abajo.
hint-could-not-open-browser-visit = No se pudo abrir el navegador. Visita: { $url }
hint-chat-with-agent = Chatea con: librefang chat { $name }
hint-agent-lost-on-exit = Nota: el agente se perderá cuando termine este proceso
hint-persistent-agents = Para agentes persistentes, ejecuta `librefang start` primero
hint-url-copied = URL copiada al portapapeles
hint-doctor-repair = Ejecuta `librefang doctor --repair` para intentar la reparación automática
hint-run-start = Ejecuta `librefang start` para lanzar el daemon
hint-config-edit = Corrige con: librefang config edit
hint-set-key = O ejecuta: librefang config set-key groq

# --- Init ---
init-quick-success = LibreFang inicializado (modo rápido)
init-interactive-success = ¡LibreFang inicializado!
init-cancelled = Configuración cancelada.
init-next-start = Inicia el daemon:  librefang start
init-next-chat = Chatear:           librefang chat

# --- Error messages ---
error-home-dir = No se pudo determinar el directorio personal
error-create-dir = Error al crear { $path }
error-create-dir-fix = Comprueba los permisos de { $path }
error-write-config = Error al escribir la configuración
error-config-created = Creado: { $path }
error-config-exists = La configuración ya existe: { $path }

# --- Daemon communication errors ---
error-daemon-returned = El daemon devolvió un error ({ $status })
error-daemon-returned-fix = Revisa los registros del daemon con: librefang logs --follow
error-request-timeout = La solicitud agotó el tiempo de espera
error-request-timeout-fix = Puede que el agente esté procesando una solicitud compleja. Inténtalo de nuevo o comprueba `librefang status`
error-connect-refused = No se puede conectar con el daemon
error-connect-refused-fix = ¿Está el daemon en ejecución? Inícialo con: librefang start
error-daemon-comm = Error de comunicación con el daemon: { $error }
error-daemon-comm-fix = Comprueba `librefang status` o reinicia: librefang start

# --- Boot errors ---
error-boot-config = Error al analizar la configuración
error-boot-config-fix = Revisa la sintaxis de tu config.toml: librefang config show
error-boot-db = Error de base de datos (puede que el archivo esté bloqueado)
error-boot-db-fix = Comprueba si hay otro proceso de LibreFang en ejecución: librefang status
error-boot-auth = Falló la autenticación del proveedor LLM
error-boot-auth-fix = Ejecuta `librefang doctor` para comprobar la configuración de tu clave API
error-boot-generic = Error al iniciar el kernel: { $error }
error-boot-generic-fix = Ejecuta `librefang doctor` para diagnosticar el problema

# --- Require daemon ---
error-require-daemon = `librefang { $command }` requiere un daemon en ejecución
error-require-daemon-fix = Inicia el daemon: librefang start

# --- Provider detection ---
detected-provider = Detectado { $display } ({ $env_var })
detected-ollama = Ollama detectado en local (no hace falta clave API)

# --- Desktop app ---

# --- Dashboard ---
dashboard-opening = Abriendo el panel en { $url }

# --- Agent commands ---
agent-spawned = Agente '{ $name }' creado
agent-spawned-inprocess = Agente '{ $name }' creado (en proceso)
agent-spawn-failed = Error al crear: { $error }
agent-spawn-agent-failed = Error al crear el agente: { $error }
agent-killed = Agente { $id } terminado.
agent-kill-failed = Error al terminar el agente: { $error }
agent-invalid-id = ID de agente no válido: { $id }
agent-no-agents = No hay agentes en ejecución.
agent-spawn-success = ¡Agente creado correctamente!
agent-spawn-inprocess-mode = Agente creado (modo en proceso).
agent-note-lost = Nota: el agente se perderá cuando termine este proceso.
agent-note-persistent = Para agentes persistentes, ejecuta `librefang start` primero.
section-agent-templates = Plantillas de agente disponibles

# --- Manifest errors ---
manifest-not-found = No se encontró el archivo de manifiesto: { $path }
manifest-not-found-fix = Usa `librefang agent new` para crear uno a partir de una plantilla
error-reading-manifest = Error al leer el manifiesto: { $error }

# --- Status ---
section-daemon-status = Estado del daemon de LibreFang
section-status-inprocess = Estado de LibreFang (en proceso)
section-active-agents = Agentes activos
section-persisted-agents = Agentes persistidos
label-daemon-not-running = NO EN EJECUCIÓN
label-home = Inicio
label-platform = Plataforma
label-sessions = Sesiones
label-memory = Memoria
label-running = en ejecución
label-response = Respuesta
label-checks = Comprobaciones
section-status-locked = Restringido (requiere clave API)
hint-status-locked = Define `api_key` en ~/.librefang/config.toml para ver agentes, sesiones y memoria.
warn-public-bind = expuesto públicamente
warn-key-missing = sin definir
section-recent-errors = Errores recientes (daemon.log)
section-verbose = Detalles
label-auth = Autenticación
label-mcp = Servidores MCP
label-peers = Pares OFP
label-channels = Canales
label-skills = Habilidades
label-hands = Hands
label-config-warnings = Avisos de configuración
auth-none = ninguna (anónima)
auth-api-key = Clave API
auth-dashboard-login = acceso al panel

# --- Doctor ---
doctor-title = LibreFang Doctor
doctor-all-passed = ¡Todas las comprobaciones pasaron! LibreFang está listo.
doctor-repairs-applied = Reparaciones aplicadas. Vuelve a ejecutar `librefang doctor` para verificar.
doctor-some-failed = Algunas comprobaciones fallaron.
doctor-no-api-keys = ¡No se encontraron claves API de proveedores LLM!
section-getting-api-key = Obtener una clave API (niveles gratuitos)

# --- Security ---
section-security-status = Estado de seguridad
label-audit-trail = Registro de auditoría
label-taint-tracking = Seguimiento de contaminación
label-wasm-sandbox = Sandbox WASM
label-wire-protocol = Protocolo de red
label-api-keys = Claves API
label-manifests = Manifiestos
value-audit-trail = Cadena de hashes Merkle (SHA-256)
value-taint-tracking = Etiquetas de flujo de información
value-wasm-sandbox = Medición doble (fuel + epoch)
value-wire-protocol = Autenticación mutua OFP HMAC-SHA256
value-api-keys = Zeroizing<String> (borrado automático al soltar)
value-manifests = Firmados con Ed25519
audit-verified = Integridad del registro de auditoría verificada (cadena Merkle válida).
audit-failed = La comprobación de integridad del registro de auditoría FALLÓ.

# --- Health ---
health-ok = El daemon está en buen estado
health-degraded = El daemon responde, pero informa de un subsistema degradado
health-not-running = El daemon no está en ejecución.

# --- Channel setup ---
channel-none-configured = No hay canales configurados.
channel-use-setup-hint = Usa `librefang channel setup` para añadir uno.
channel-reloaded = Canales recargados ({ $started } sidecar(s) iniciados).
channel-registry-empty = El registro de canales del daemon está vacío.
channel-install-sdk-hint = Instala el SDK de sidecars para que los adaptadores aparezcan en el catálogo:
channel-install-sdk-cmd =   pip install librefang-sdk
channel-rerun-setup-hint = Después, vuelve a ejecutar `librefang channel setup`.
channel-all-configured = Todos los canales disponibles ya están configurados.
channel-see-list-hint = Usa `librefang channel list` para verlos, o
channel-remove-entry-hint = `librefang channel rm <name>` para quitar una entrada primero.
channel-pick-setup = Elige un canal para configurar:
channel-choice-prompt = Opción [1]: 
channel-unknown-error = Canal desconocido: { $name }
channel-unknown-error-fix = Ejecuta `librefang channel list` para ver los adaptadores disponibles.
channel-no-configurable-fields = `{ $name }` no expone campos configurables; no hay nada que preguntar.
channel-hot-reload-manual-hint = (Puedes recargar en caliente con `librefang channel reload` si ya has editado config.toml a mano.)
channel-prompt-secret-keep =   { $label } ({ $key }) [definido — déjalo vacío para conservarlo]: 
channel-prompt-secret-retype =   { $label } ({ $key }) [definido, obligatorio — vuelve a escribirlo para guardarlo]:
channel-prompt-default =   { $label } ({ $key }) [{ $current }]: 
channel-prompt-required =   { $label } ({ $key }) *: 
channel-prompt-optional =   { $label } ({ $key }): 
channel-save-rejected = Se rechazó el guardado de `{ $name }`: { $error }
channel-save-rejected-fix = Vuelve a ejecutarlo con los valores corregidos o revisa el registro del daemon para más detalles.
channel-saved-restart-required = ✓ `{ $name }` guardado; reinicia el daemon para aplicar los cambios.
channel-saved-hot-reload = ✓ `{ $name }` guardado; recarga en caliente aplicada.
channel-env-shadowing-warn = Aviso: las variables de entorno del shell tapan estos tokens; quítalas y reinicia para que se apliquen los nuevos valores: { $keys }
channel-config-read-fail = No se puede leer { $path }: { $error }
channel-config-read-fail-fix = Ejecuta `librefang init` para crear el archivo de configuración.
channel-config-parse-fail = No se puede analizar { $path }: { $error }
channel-config-parse-fail-fix = Corrige la sintaxis TOML y vuelve a intentarlo.
channel-no-entries-to-remove = No hay entradas [[sidecar_channels]] en config.toml; no hay nada que quitar.
channel-no-entry-with-name = No hay ninguna entrada [[sidecar_channels]] con name="{ $name }".
channel-config-write-fail = Error al escribir { $path }: { $error }
channel-config-write-fail-fix = Comprueba los permisos del sistema de archivos.
channel-removed-entries = ✓ Se quitaron { $count } entrada(s) [[sidecar_channels]] llamadas `{ $name }`.
channel-hot-reloaded-daemon =   Daemon recargado en caliente.
channel-reload-status-warn =   La recarga devolvió { $status }: el cambio se aplicará en el próximo reinicio del daemon.
channel-reload-contact-fail-warn =   No se pudo contactar con el daemon para recargar ({ $error }); el cambio se aplicará en el próximo arranque.
channel-reload-daemon-offline =   El daemon no está en ejecución; el cambio se aplicará en el próximo arranque.
# --- Vault ---
vault-initialized = Bóveda de credenciales inicializada.
vault-not-initialized = Bóveda no inicializada.
vault-not-init-run = Bóveda no inicializada. Ejecuta: librefang vault init
vault-unlock-failed = No se pudo desbloquear la bóveda: { $error }
vault-empty-value = Valor vacío; no se guardó.
vault-stored = '{ $key }' guardado en la bóveda.
vault-store-failed = Error al guardar: { $error }
vault-removed = '{ $key }' eliminado de la bóveda.
vault-key-not-found = No se encontró la clave '{ $key }' en la bóveda.
vault-remove-failed = Error al eliminar: { $error }
vault-rotate-no-vault = No se encontró ningún archivo de bóveda. Ejecuta `librefang vault init` primero.
vault-rotate-old-key-missing = LIBREFANG_VAULT_KEY_OLD sin definir. Proporciona la clave maestra actual (base64 de 32 bytes) antes de rotar.
vault-rotate-new-key-missing = LIBREFANG_VAULT_KEY_NEW sin definir. Proporciona la nueva clave maestra (base64 de 32 bytes) o pasa --from-stdin para leerla de la entrada estándar.
vault-rotate-stdin-read-failed = Error al leer la nueva clave de la entrada estándar: { $error }
vault-rotate-stdin-empty = La nueva clave leída de la entrada estándar estaba vacía.
vault-rotate-same-key = LIBREFANG_VAULT_KEY_OLD y la nueva clave son idénticas; se rechaza rotar a la misma clave.
vault-rotate-old-key-invalid = LIBREFANG_VAULT_KEY_OLD no es una clave base64 válida de 32 bytes: { $error }
vault-rotate-new-key-invalid = La nueva clave no es una clave base64 válida de 32 bytes: { $error }
vault-rotate-unlock-failed = Error al desbloquear la bóveda con la clave ANTIGUA: { $error }. Comprueba que LIBREFANG_VAULT_KEY_OLD coincide con la clave con la que se cifró originalmente la bóveda.
vault-rotate-sentinel-failed = Falló la verificación del centinela de la bóveda con la clave ANTIGUA: { $error }
vault-rotate-rewrap-failed = Error al recifrar la bóveda con la nueva clave: { $error }. El archivo de bóveda original no ha cambiado.
vault-rotate-success = Bóveda recifrada con la nueva clave maestra ({ $count } entradas de usuario conservadas).
vault-rotate-next-step = Siguiente paso: asigna el nuevo valor a LIBREFANG_VAULT_KEY antes de reiniciar el daemon.

# --- Cron ---
cron-created = Tarea cron creada: { $id }
cron-create-failed = Error al crear la tarea cron: { $error }
cron-deleted = Tarea cron { $id } eliminada.
cron-delete-failed = Error al eliminar la tarea cron: { $error }
cron-toggled = Tarea cron { $id } actualizada ({ $action }).
cron-toggle-failed = Error al cambiar la tarea cron ({ $action }): { $error }

# --- Automation ---
automation-workflow-none = No hay flujos de trabajo registrados.
automation-workflow-file-not-found = No se encontró el archivo de flujo de trabajo: { $path }
automation-workflow-read-error = Error al leer el archivo de flujo de trabajo: { $error }
automation-workflow-invalid-json = JSON no válido: { $error }
automation-workflow-created = ¡Flujo de trabajo creado correctamente!
automation-workflow-created-id =   ID: { $id }
automation-workflow-create-failed = Error al crear el flujo de trabajo: { $error }
automation-workflow-completed = ¡Flujo de trabajo completado!
automation-workflow-run-id =   ID de ejecución: { $id }
automation-workflow-still-running = Flujo de trabajo iniciado; sigue en ejecución.
automation-workflow-failed = El flujo de trabajo falló: { $error }
automation-trigger-none = No hay activadores registrados.
automation-trigger-invalid-pattern = JSON de patrón no válido: { $error }
automation-trigger-created = ¡Activador creado correctamente!
automation-trigger-created-id =   ID del activador: { $id }
automation-trigger-created-agent =   ID del agente:   { $agent_id }
automation-trigger-created-target =   Destino:     { $target }
automation-trigger-create-failed = Error al crear el activador: { $error }
automation-trigger-deleted = Activador { $id } eliminado.
automation-trigger-delete-failed = Error al eliminar el activador: { $error }
automation-trigger-get-failed = Error al obtener el activador: { $error }
automation-trigger-update-failed = Error al actualizar el activador: { $error }
automation-trigger-updated = Activador { $id } actualizado.
automation-trigger-toggle-failed = Error al cambiar el activador ({ $action }): { $error }
automation-trigger-toggled = Activador { $id } actualizado ({ $action }).
automation-trigger-info-id = ID del activador:    { $id }
automation-trigger-info-agent = ID del agente:      { $id }
automation-trigger-info-pattern = Patrón:       { $pattern }
automation-trigger-info-prompt = Prompt:        { $prompt }
automation-trigger-info-enabled = Habilitado:       { $enabled }
automation-trigger-info-fires = Disparos:    { $count }
automation-trigger-info-max-fires = Disparos máx.: { $count }
automation-trigger-info-target = Agente destino:  { $agent }
automation-trigger-info-cooldown = Enfriamiento:      { $secs }s
automation-trigger-info-session = Modo de sesión:  { $mode }
automation-unlimited = ilimitado
automation-cron-none = No hay tareas programadas.

label-header-steps = PASOS
label-header-trigger-id = ID DEL ACTIVADOR
label-header-agent-id = ID DEL AGENTE
label-header-fires = DISPAROS
label-header-pattern = PATRÓN
label-header-schedule = PROGRAMACIÓN
label-header-prompt = PROMPT

# --- Approvals ---
approval-responded = Aprobación { $id } registrada ({ $action }).
approval-failed = Error al registrar la aprobación ({ $action }): { $error }

# --- Memory ---
memory-set = { $key } definido para el agente '{ $agent }'.
memory-set-failed = Error al definir la memoria: { $error }
memory-deleted = Clave '{ $key }' eliminada para el agente '{ $agent }'.
memory-delete-failed = Error al eliminar la memoria: { $error }

# --- Devices ---
section-device-pairing = Emparejamiento de dispositivos
device-scan-qr = Escanea este código QR con la app móvil de LibreFang:
device-removed = Dispositivo { $id } eliminado.
device-remove-failed = Error al eliminar el dispositivo: { $error }

# --- Webhooks ---
webhook-created = Webhook creado: { $id }
webhook-create-failed = Error al crear el webhook: { $error }
webhook-deleted = Webhook { $id } eliminado.
webhook-delete-failed = Error al eliminar el webhook: { $error }
webhook-test-ok = Carga de prueba del webhook { $id } enviada correctamente.
webhook-test-failed = Error al probar el webhook: { $error }

# --- Models ---
model-set-success = Modelo predeterminado definido como: { $model }
model-set-failed = Error al definir el modelo: { $error }
model-no-catalog = No hay modelos en el catálogo.
section-select-model = Selecciona un modelo
model-out-of-range = Número fuera de rango (1-{ $max })
model-none-found = No se encontraron modelos.
model-prompt-selection =   Introduce el número o el ID del modelo: 


# --- Config ---
config-no-file = No se encontró ningún archivo de configuración
config-no-file-fix = Ejecuta `librefang init` primero
config-read-failed = Error al leer la configuración: { $error }
config-parse-error = Error de análisis de la configuración: { $error }
config-parse-fix = Corrige la sintaxis de tu config.toml o ejecuta `librefang config edit`
config-parse-fix-alt = Corrige primero la sintaxis de tu config.toml
config-key-not-found = Clave no encontrada: { $key }
config-key-path-not-found = Ruta de clave no encontrada: { $key }
config-empty-key = Clave vacía
config-section-not-scalar = '{ $key }' es una sección, no un escalar
config-section-not-scalar-fix = Usa notación con puntos: { $key }.field_name
config-parent-not-table = El padre de '{ $key }' no es una tabla
config-serialize-failed = Error al serializar la configuración: { $error }
config-write-failed = Error al escribir la configuración: { $error }
config-set-kv = { $key } = { $value } definido
config-removed-key = Clave eliminada: { $key }
config-no-key = No se proporcionó ninguna clave. Cancelado.
config-saved-key = { $env_var } guardada en ~/.librefang/.env
config-save-key-failed = Error al guardar la clave: { $error }
config-removed-env = { $env_var } eliminada de ~/.librefang/.env
config-remove-key-failed = Error al eliminar la clave: { $error }
config-env-not-set = { $env_var } sin definir
config-set-key-hint = Defínela: librefang config set-key { $provider }
config-update-key-hint = Actualiza la clave: librefang config set-key { $provider }
config-no-file-found = No se encontró configuración en: { $path }
config-run-init-hint = Ejecuta `librefang init` para crear una.
config-read-error = Error al leer la configuración: { $error }
config-editor-exit = El editor terminó con: { $status }
config-editor-open-fail = Error al abrir el editor '{ $editor }': { $error }
config-editor-env-hint = Define $EDITOR con tu editor preferido.
config-val-exceeds-i64 = el valor { $value } supera i64::MAX ({ $max }); TOML no puede almacenar enteros sin signo por encima de este límite
config-invalid-integer = '{ $raw }' no es un entero válido
config-paste-api-key-prompt =   Pega tu clave API de { $provider }: 
config-testing-key =   Probando la clave... 
config-testing-provider-key =   Probando { $provider } ({ $env_var })... 
config-test-ok = OK
config-test-failed = FALLÓ (401/403)
config-test-unverified = no se pudo verificar (puede funcionar igualmente)


# --- Hand commands ---
hand-install-deps-success = Dependencias instaladas para el Hand '{ $id }'.
hand-install-deps-dep-failed = { $key } ({ $status }): { $message }
hand-install-deps-dep-still-missing = { $key }: sigue faltando la dependencia obligatoria
hand-paused = Instancia de Hand '{ $label } (instancia: { $instance_id })' pausada.
hand-resumed = Instancia de Hand '{ $label } (instancia: { $instance_id })' reanudada.

# --- Daemon notify ---

# --- System info ---
section-system-info = Información del sistema LibreFang

# --- Uninstall ---
uninstall-warning = Esto desinstalará LibreFang por completo del sistema.
uninstall-remove-data-kept =   • Eliminar los datos de { $path } (conservando los archivos de configuración)
uninstall-remove-all =   • Eliminar { $path }
uninstall-remove-binary =   • Eliminar el binario: { $path }
uninstall-remove-cargo-binary =   • Eliminar el binario de cargo: { $path }
uninstall-remove-autostart =   • Eliminar las entradas de inicio automático (si las hay)
uninstall-clean-path =   • Limpiar PATH en las configuraciones del shell (si las hay)
uninstall-confirm-prompt =   Escribe 'uninstall' para confirmar: 
uninstall-goodbye = LibreFang se ha desinstalado. ¡Hasta luego!
uninstall-cancelled = Cancelado.
uninstall-stopping-daemon = Deteniendo el daemon en ejecución...
uninstall-removed = { $path } eliminado
uninstall-remove-failed = Error al eliminar { $path }: { $error }
uninstall-removed-data-kept = Datos eliminados (archivos de configuración conservados)
uninstall-removed-autostart-win = Entrada de inicio automático de Windows eliminada
uninstall-removed-launch-agent = Agente de inicio de macOS eliminado
uninstall-remove-launch-fail = Error al eliminar el agente de inicio: { $error }
uninstall-removed-autostart-linux = Entrada de inicio automático de Linux eliminada
uninstall-remove-autostart-fail = Error al eliminar la entrada de inicio automático: { $error }
uninstall-removed-systemd = Servicio de usuario de systemd eliminado
uninstall-remove-systemd-fail = Error al eliminar el servicio de systemd: { $error }
uninstall-cleaned-path = PATH limpiado en { $path }
uninstall-cleaned-path-win = PATH limpiado del entorno de usuario de Windows

# --- Reset ---
reset-success = { $path } eliminado
reset-fail = Error al eliminar { $path }: { $error }

# --- Logs ---
log-following = --- Siguiendo { $path } (Ctrl+C para detener) ---

# --- Extracted from Rust sources ---
init-error-create-data-dir = Error al crear el directorio de datos: { $error }
init-upgrade-existing = Instalación existente detectada; ejecutando la actualización para conservar tus ajustes.
init-upgrade-fresh-hint = Para empezar de cero, elimina ~/.librefang/config.toml y vuelve a ejecutar `librefang init`.
init-upgrade-no-config = Nada que actualizar: no se encontró config.toml. Ejecuta `librefang init` primero.
init-upgrade-registry-synced = Registro sincronizado
init-upgrade-registry-failed = Falló la sincronización del registro (¿problema de red?); continuando con el contenido en caché
init-upgrade-config-up-to-date = La configuración ya está al día; no se añadieron campos nuevos
init-upgrade-sections-added = Se añadieron { $count } sección(es) nueva(s) a la configuración:
init-upgrade-legacy-openclaw = Instalación heredada de ~/.openclaw detectada.
init-upgrade-legacy-openclaw-hint = Ejecuta `librefang migrate --from openclaw` para migrar tus datos.
init-upgrade-approval-warning = Tu lista require_approval solo contiene "shell_exec". Las operaciones de archivos (file_write, file_delete) ahora requieren aprobación de forma predeterminada.
init-upgrade-approval-hint = Para habilitarlo: añade "file_write" y "file_delete" a require_approval en config.toml
init-upgrade-success-summary = ¡Actualización completada!
init-upgrade-title = Actualizando la instalación de LibreFang
init-upgrade-progress-label = Actualizando
init-upgrade-backing-up = Haciendo copia de seguridad de la configuración
init-upgrade-backup-success = Configuración copiada a backups/{ $name }
init-upgrade-syncing-registry = Sincronizando el registro
init-upgrade-initializing-vault-git = Inicializando bóveda/git
init-upgrade-merging-config = Combinando campos de configuración
init-upgrade-failed-read = Actualización abortada: no se pudo leer config.toml: { $error }
init-upgrade-failed-parse = Actualización abortada: no se pudo analizar config.toml: { $error }
init-upgrade-backup-saved-hint = Tu configuración original se guardó en backups/{ $name }
init-upgrade-failed-parse-template = Actualización abortada: no se pudo analizar la plantilla de configuración predeterminada: { $error }
init-upgrade-failed-write = Actualización abortada: no se pudo escribir la configuración: { $error }
init-upgrade-steps-complete = Pasos de la actualización completados
label-backup = Copia de seguridad
label-new-fields = Campos nuevos

auth-chatgpt-device-requested = Autenticación de dispositivo solicitada.
auth-chatgpt-device-open-url = Abre esta URL en cualquier navegador:\n  { $url }\n
auth-chatgpt-device-one-time-code = Introduce este código de un solo uso:\n  { $code }\n
auth-chatgpt-device-do-not-share = No compartas este código.
auth-chatgpt-device-waiting = Esperando la autorización...
auth-chatgpt-switching-browser = \nCambiando al flujo de acceso estándar del navegador...\n
auth-chatgpt-opening-browser = Abriendo el navegador para la autenticación de OpenAI...
auth-chatgpt-open-manually-hint = Si el navegador no se abre, visita:\n  { $url }\n
auth-chatgpt-open-browser-failed = No se pudo abrir el navegador automáticamente: { $error }
auth-chatgpt-open-manually = Ábrelo manualmente: { $url }
auth-chatgpt-tokens-saved = \nTokens de ChatGPT guardados en { $path }
auth-chatgpt-detecting-model = Detectando el mejor modelo disponible...
auth-chatgpt-selected-model = Modelo seleccionado: { $model }
auth-chatgpt-config-updated = config.toml actualizado: provider = "chatgpt", model = "{ $model }"
auth-chatgpt-starting-flow = Iniciando el flujo de autenticación de ChatGPT...\n
auth-chatgpt-complete = Autenticación de ChatGPT completada.
auth-chatgpt-failed = Falló la autenticación de ChatGPT: { $error }

auth-pool-config-not-array = `credential_pools` existe en config.toml pero no es un array de tablas
auth-pool-daemon-error-fallback = El daemon devolvió HTTP { $status }; recurriendo a la vista de config.toml
auth-pool-daemon-connect-fallback = Error al consultar el daemon en { $url }: { $error }; recurriendo a la vista de config.toml
auth-pool-no-config-offline = No hay configuración en { $path } y el daemon no está en ejecución.
auth-pool-config-load-failed = Error al cargar la configuración: { $error }
auth-pool-none-configured = No hay grupos de credenciales configurados.
auth-pool-invalid-env-name = `{ $env_var }` no es un nombre de variable de entorno válido. Se esperan letras mayúsculas, dígitos y guiones bajos (p. ej., OPENAI_API_KEY_2).
auth-pool-env-empty = la variable de entorno `{ $env_var }` está definida pero vacía.
auth-pool-env-empty-fix = Asígnale tu clave API antes de añadir la entrada al grupo, p. ej.\n  export { $env_var }=sk-…\nDespués vuelve a intentarlo.
auth-pool-env-not-set = la variable de entorno `{ $env_var }` no está definida en el shell actual.
auth-pool-env-not-set-fix = Expórtala antes de añadir la entrada al grupo, p. ej.\n  export { $env_var }=sk-…\nDespués vuelve a intentarlo. (El daemon la leerá de su propio entorno al arrancar; asegúrate de exportarla también allí.)
auth-pool-keys-not-array = El grupo de `{ $provider }` tiene un campo `keys` que no es un array de tablas.
auth-pool-key-duplicate = Ya existe una clave con env_var `{ $env_var }` en el grupo del proveedor `{ $provider }`.
auth-pool-key-added = Clave `{ $label }` (env={ $env_var }, priority={ $priority }) añadida al grupo de `{ $provider }`. Reinicia el daemon o recarga la configuración en caliente para aplicarlo.
auth-pool-not-configured = No hay ningún grupo de credenciales configurado para el proveedor `{ $provider }`.
auth-pool-no-keys-field = El grupo de `{ $provider }` no tiene array de claves.
auth-pool-key-not-found = No se encontró ninguna clave con env_var `{ $env_var }` en el grupo de `{ $provider }`.
auth-pool-key-removed-pool-empty = Clave `{ $env_var }` eliminada del grupo de `{ $provider }`. El grupo se ha quedado vacío y se ha eliminado por completo. Reinicia el daemon o recarga la configuración en caliente para aplicarlo.
auth-pool-key-removed = Clave `{ $env_var }` eliminada del grupo de `{ $provider }`. Reinicia el daemon o recarga la configuración en caliente para aplicarlo.
auth-pool-unknown-strategy = Estrategia desconocida `{ $strategy }`. Válidas: fill_first, round_robin, random, least_used.
auth-pool-strategy-set = Estrategia del grupo de `{ $provider }` definida como `{ $strategy }`. Reinicia el daemon o recarga la configuración en caliente para aplicarlo.
vault-empty = La bóveda está vacía.
vault-stored-count = Credenciales guardadas ({ $count }):

# --- Scanned & Extracted keys ---
# init.rs
init-upgrade-failed-create-backups-dir = Error al crear el directorio de copias de seguridad: { $error }
init-upgrade-failed-backup-config = Error al hacer copia de seguridad de la configuración: { $error }
init-error-write-config-example = No se pudo escribir config.example.toml: { $error }

# acp.rs
acp-attached-uds = librefang acp: conectado al daemon (UDS { $path })
acp-attached-pipe = librefang acp: conectado al daemon (pipe con nombre)
acp-in-process = librefang acp: kernel en proceso (no se detectó ningún daemon)
acp-error-boot-kernel = Error al iniciar el kernel: { $error }
acp-error-resolve-agent = Error al resolver el agente '{ $name }': { $error }
acp-error-server = Error del servidor ACP: { $error }
acp-error-uds-connect = Falló la conexión UDS de ACP en { $path }: { $error }
acp-error-pipe-connect = Falló la conexión por pipe con nombre de ACP en { $name }: { $error }
acp-error-agent-attached = El listener ACP del daemon en ejecución solo sirve al agente '{ $default }', así que '--agent { $name }' no puede respetarse mientras esté activo; omite --agent o detén el daemon para ejecutar '{ $name }' en proceso


# auth.rs
auth-write-failed = Error al escribir { $path }: { $error }
auth-password-empty = La contraseña no puede estar vacía.
auth-passwords-mismatch = Las contraseñas no coinciden.
auth-password-hash-failed = Error al aplicar hash a la contraseña: { $error }
vault-enter-value-prompt = Introduce el valor de { $key }: 
auth-enter-password-prompt = Introduce la contraseña: 
auth-confirm-password-prompt = Confirma la contraseña: 

# agent.rs
agent-spawn-choose-target-or-template = Elige un destino posicional o `--template`, no ambos.
agent-spawn-choose-target-or-template-fix = Usa `librefang spawn coder` o `librefang spawn --template agents/custom/my-agent.toml`.
agent-spawn-name-requires-template = `--name` requiere un nombre de plantilla o una ruta de manifiesto.
agent-spawn-name-requires-template-fix = Usa `librefang spawn coder --name backend-coder` o `librefang spawn --template path/to/agent.toml --name backend-coder`.
agent-spawn-dry-run-requires-template = La simulación necesita un nombre de plantilla o una ruta de manifiesto.
agent-spawn-dry-run-requires-template-fix = Usa `librefang spawn coder --dry-run` o `librefang spawn --template path/to/agent.toml --dry-run`.
agent-spawn-template-or-path-not-found = No se encontró la plantilla o la ruta del manifiesto: { $target }
agent-spawn-template-or-path-not-found-fix = Ejecuta `librefang agent new` para explorar plantillas, o pasa una ruta de manifiesto válida.
agent-manifest-parse-failed = Error al analizar el manifiesto del agente desde { $source }: { $error }
agent-manifest-parse-failed-fix = Comprueba la sintaxis TOML del manifiesto y los campos obligatorios.
agent-manifest-serialize-failed = Error al serializar el manifiesto actualizado: { $error }
agent-dry-run-title = Simulación de agente
agent-dry-run-success = Manifiesto analizado correctamente. No se creó ningún agente.
agent-delete-warning-text = AVISO: eliminar el agente "{ $name }" borrará de forma permanente su UUID canónico
    y todas las memorias y sesiones asociadas.
    Esta acción no se puede deshacer.
label-confirm-prompt = ¿Confirmar?
label-aborted = Cancelado.
agent-delete-no-uuid = No hay UUID canónico registrado para el agente '{ $name }'; no hay nada que eliminar.
agent-deleted-success = Agente "{ $name }" eliminado (UUID canónico purgado).
agent-delete-failed-with-reason = Error al eliminar el agente: { $error }
agent-reset-uuid-warning-text = AVISO: restablecer el UUID canónico de "{ $name }" dejará huérfanas todas las sesiones
    y memorias vinculadas a su UUID actual. La próxima creación con este
    nombre empezará con un UUID nuevo. Esta acción no se puede deshacer.
agent-reset-uuid-success = UUID canónico de "{ $name }" restablecido (era { $previous }).
agent-reset-uuid-failed-with-reason = Error al restablecer el UUID canónico: { $error }
agent-reset-uuid-not-found = No hay UUID canónico registrado para el agente '{ $name }'.
agent-merge-history-not-implemented = merge-history aún no está implementado (seguimiento en #4614).
    La reasignación de sesiones y memorias de { $from } al UUID canónico
    del agente "{ $name }" requiere cirugía SQL entre tablas en el sustrato
    de memoria que se está tratando por separado.
agent-set-model-success = Modelo del agente { $id } definido como { $value }.
agent-set-model-failed-with-reason = Error al definir el modelo: { $error }
agent-set-no-daemon = No se encontró ningún daemon en ejecución. Inicia uno con: librefang start
agent-set-unknown-field = Campo desconocido: { $field }. Campos admitidos: model, temperature, max_tokens, top_p, frequency_penalty, presence_penalty, top_k, min_p, repeat_penalty, context_window, max_output_tokens
agent-set-field-success = { $field } del agente { $id } definido como { $value }.
agent-new-no-templates = No se encontraron plantillas de agente
agent-new-no-templates-fix = Ejecuta `librefang init` para configurar el directorio de agentes
agent-new-template-not-found = Plantilla '{ $name }' no encontrada
agent-new-template-not-found-fix = Ejecuta `librefang agent new` para ver las plantillas disponibles
agent-new-choose-template-prompt =   Elige plantilla [1]: 
agent-sessions-none-active = No hay sesiones activas.
agent-sessions-none-found = No se encontraron sesiones.
message-invalid-session-id = UUID de sesión no válido: { $value }
message-invalid-session-id-fix = Pasa el UUID de una sesión existente de este agente, u omite --session-id para usar su sesión canónica. `librefang sessions <agent>` las lista.

label-source = Origen
label-name = Nombre
label-captured = Capturado
label-module = Módulo
label-tools = Herramientas
label-tags = Etiquetas
label-description = Descripción

# daemon.rs
daemon-first-run-setup = Primera ejecución detectada; iniciando la configuración rápida...
daemon-config-not-found = No se encontró el archivo de configuración: { $path }
daemon-config-not-found-fix = Ejecuta `librefang init` para crear una configuración predeterminada en ~/.librefang/config.toml, o comprueba la ruta de --config.
daemon-log-file-not-found = No se encontró el archivo de registro
daemon-log-file-not-found-fix = Se esperaba en: { $path }
daemon-log-not-found-showing-tui = No se encontró el registro del daemon; mostrando el registro de la TUI en { $path }

# hand.rs
hand-install-error-no-toml = Error: no se encontró HAND.toml en { $path }
hand-install-error-read-toml = Error al leer { $path }: { $error }
hand-error-prefix = Error: { $error }
hand-installed-success = Hand instalado: { $name } ({ $id })
hand-activate-hint = Usa `librefang hand activate { $id }` para iniciarlo.
hand-none-available = No hay Hands disponibles.
hand-list-activate-hint =
    Usa `librefang hand activate <id>` para activar un Hand.
hand-none-active = No hay Hands activos.
label-hand = Hand
label-instance = Instancia
label-agent = Agente
hand-status-title = Estado de los Hands
label-status-inactive = inactivo
hand-not-found = No se encontró ningún Hand activo o instalado para '{ $id }'.
hand-activated-success = Hand '{ $id }' activado (instancia: { $instance }, agente: { $agent })
hand-activate-failed = Error al activar el Hand '{ $id }': { $error }
hand-deactivated-success = Hand '{ $id }' desactivado.
label-failed-reason = Falló: { $error }
hand-no-active-instance = No se encontró ninguna instancia activa del Hand '{ $id }'.
hand-info-not-found = Hand no encontrado: { $error }
hand-no-settings = El Hand '{ $id }' no tiene ajustes configurables.
hand-settings-title = Ajustes de '{ $id }'
hand-setting-default-marker = (predeterminado)
hand-setting-value-ignored = el valor guardado "{ $value }" no es válido para este ajuste y se ignoró
hand-settings-unknown-keys = Claves guardadas que este Hand no declara e ignoró: { $keys }
hand-set-setting-success = { $key }={ $value } definido para el Hand '{ $id }'.
hand-reloaded-summary = Hands recargados: { $added } añadidos, { $updated } actualizados, { $total } en total.
label-chat-with = Chat con
hand-chat-quit-hint = (escribe /quit para salir)
hand-chat-prompt-you = tú >
label-no-response = [sin respuesta]
# mcp_cmds.rs
mcp-catalog-unknown-entry = Entrada desconocida del catálogo MCP: '{ $name }'
mcp-catalog-available-header =
    Servidores MCP disponibles (catálogo):
mcp-failed-read-config = Error al leer { $path }: { $error }
mcp-invalid-toml = { $path } no es TOML válido: { $error }
mcp-already-configured = El servidor MCP '{ $name }' ya está configurado. Ejecuta `librefang mcp remove { $name }` primero si quieres reinstalarlo.
mcp-failed-write-config = Error al escribir config.toml: { $error }
mcp-add-credentials-hint =
    Para añadir credenciales:
mcp-get-it-here =   Consíguelo aquí: { $url }
mcp-not-configured = El servidor MCP '{ $name }' no está configurado
mcp-failed-update-config = Error al actualizar config.toml: { $error }
mcp-removed-success = { $name } eliminado.
mcp-catalog-no-matches = Ninguna entrada del catálogo MCP coincide con '{ $query }'.
mcp-catalog-none-available = No hay entradas disponibles en el catálogo MCP.
mcp-catalog-summary =   { $total } entradas del catálogo ({ $installed } instaladas)
mcp-catalog-install-hint =   Usa `librefang mcp add <id>` para instalar un servidor MCP.
mcp-none-configured = No hay servidores MCP configurados.
mcp-list-catalog-hint =   Usa `librefang mcp catalog` para listar las entradas instalables.
mcp-vault-set-hint =   librefang vault set { $name }  # { $help }
mcp-header-name = nombre
mcp-header-template-id = template_id
mcp-header-transport = transporte
mcp-header-details = detalles

# monitoring.rs
monitoring-audit-reset-destructive = restablecer la auditoría es destructivo; vuelve a ejecutarlo con `--confirm` para continuar
monitoring-db-not-found = no se encontró la base de datos en { $path }
monitoring-db-open-failed = no se pudo abrir { $path }: { $error }
monitoring-db-truncate-failed = no se pudo truncar audit_entries: { $error }
monitoring-audit-reset-anchor-deleted = , ancla eliminada en { $path }
monitoring-audit-reset-anchor-none =  (no hay archivo de ancla que eliminar)
monitoring-audit-reset-success = Registro de auditoría restablecido: se eliminaron { $count } fila(s) de audit_entries{ $anchor_detail }.
monitoring-audit-reset-would-header =   Haría lo siguiente:
monitoring-audit-reset-would-delete =     1. DELETE de todas las filas de `audit_entries` en { $path }
monitoring-audit-reset-would-remove-anchor =     2. Eliminar el archivo de ancla { $path }
monitoring-audit-reset-would-restart =   La cadena Merkle se reiniciará a partir del siguiente evento de auditoría.
monitoring-daemon-running-error = el daemon se ejecuta en { $url }; se rechaza tocar la base de datos de auditoría
monitoring-daemon-running-error-fix = detén el daemon primero: `librefang stop`
monitoring-anchor-remove-failed = no se pudo eliminar el ancla { $path }: { $error }
monitoring-audit-reset-seed-fresh = El próximo arranque del daemon sembrará una cadena Merkle nueva desde la punta actual.
monitoring-audit-reanchor-verified = La cadena de auditoría verifica; no hay nada que reparar.
monitoring-audit-reanchor-diagnose-failed = no se puede leer la cadena de auditoría: { $error }
monitoring-audit-reanchor-break = Ruptura de cadena en seq { $seq } ({ $kind }): se esperaba { $expected }, se encontró { $found }.
monitoring-audit-reanchor-counts = { $rows_before } fila(s) en audit_entries: { $surviving } por debajo de la ruptura siguen verificando, { $severed } en ella o después se archivan y se eliminan.
monitoring-audit-reanchor-would-header =   Haría lo siguiente:
monitoring-audit-reanchor-would-archive =     1. Archivar las { $severed } fila(s) cortadas como JSON Lines en { $path }
monitoring-audit-reanchor-would-delete =     2. DELETE de esas { $severed } fila(s) de `audit_entries` en { $path }
monitoring-audit-reanchor-would-marker =     3. Añadir una entrada ChainReanchored enlazada a la última fila superviviente, con el SHA-256 del archivo
monitoring-audit-reanchor-would-anchor =     4. Reescribir el archivo de ancla { $path }
monitoring-audit-reanchor-dry-run = audit reanchor modifica el registro de auditoría; vuelve a ejecutarlo con `--confirm` para continuar
monitoring-audit-reanchor-failed = falló audit reanchor: { $error }
monitoring-audit-reanchor-success = Cadena de auditoría reparada: se cortaron { $severed } fila(s) desde seq { $seq }, marcador escrito en seq { $marker }, quedan { $rows } fila(s).
monitoring-audit-reanchor-archive = Filas cortadas conservadas en { $path } (sha256 { $digest }).
monitoring-audit-reanchor-verify-hint = Inicia el daemon y ejecuta `librefang security verify` para confirmar que la cadena está intacta.
# skill.rs
skill-install-progress = Instalando { $source }

# system.rs
migrate-error-home-dir = Error: no se pudo determinar el directorio personal
migrate-start-msg = Migrando desde { $source } ({ $path })...
migrate-dry-run-hint =   (simulación; no se hará ningún cambio)
migrate-progress-label = Ejecutando la migración
migrate-complete-msg = Migración completada
migrate-warn-report-save-failed = Aviso: no se pudo guardar el informe de migración: { $error }
migrate-report-saved =
      Informe guardado en: { $path }
migrate-failed-msg = La migración falló: { $error }

# maintenance.rs
maintenance-service-install-root-error = Ejecutando como root: el servicio se instalará para la cuenta root, no para tu usuario. Ejecuta sin sudo.
maintenance-service-unsupported = El servicio de inicio automático no es compatible con esta plataforma.
maintenance-failed-create-dir = Error al crear { $path }: { $error }
maintenance-failed-write-file = Error al escribir { $path }: { $error }
maintenance-wrote-file = Se escribió { $path }
maintenance-systemctl-reload-failed = falló systemctl --user daemon-reload
maintenance-service-enabled = Servicio habilitado (se iniciará en el próximo inicio de sesión)
maintenance-service-start-hint = Inícialo ahora con: systemctl --user start librefang.service
maintenance-service-linger-hint = Para servidores sin interfaz, ejecuta también: loginctl enable-linger
maintenance-systemctl-enable-failed = falló systemctl --user enable librefang.service
maintenance-launchagent-loaded = LaunchAgent cargado (se iniciará al iniciar sesión y ahora)
maintenance-service-install-system-hint = Para un servicio que arranque al iniciar el sistema en vez de al iniciar sesión, usa: sudo librefang service install --system
maintenance-service-system-needs-root = --system escribe en /Library/LaunchDaemons, lo que requiere root.
maintenance-service-system-sudo-hint = Vuelve a ejecutarlo con sudo: sudo librefang service install --system
maintenance-service-system-macos-only = --system instala un LaunchDaemon de macOS y no está implementado en esta plataforma.
maintenance-service-system-linux-hint = En NixOS usa el módulo services.librefang; en otras distribuciones de Linux escribe una unidad de sistema en /etc/systemd/system/ o conserva el servicio de usuario que crea `librefang service install`.
maintenance-service-system-no-sudo-user = No se pudo determinar con qué cuenta debe ejecutarse el daemon: SUDO_USER no está definido, así que parece un inicio de sesión directo como root y no sudo.
maintenance-service-system-chown-failed = Error al dar a { $user } la propiedad de { $path }: { $error }
maintenance-service-system-agent-conflict = Ya hay un LaunchAgent por usuario instalado en { $path }. Lleva la misma etiqueta ai.librefang.daemon y el mismo directorio de estado que el LaunchDaemon de arranque, así que instalar ambos dejaría al agente fallando y relanzándose tras cada inicio de sesión.
maintenance-service-system-agent-conflict-fix = Elimínalo primero con tu propio usuario, no con sudo: librefang service uninstall
maintenance-service-system-chmod-failed = Error al definir los permisos de { $path }: { $error }
maintenance-launchdaemon-loaded = LaunchDaemon cargado; arranca con el sistema y se ejecuta como { $user }
maintenance-launchdaemon-removed = LaunchDaemon eliminado
maintenance-launchdaemon-remove-failed = Error al eliminar el plist del LaunchDaemon: { $error }
maintenance-launchdaemon-not-found = No se encontró ningún LaunchDaemon; no hay nada que eliminar.
maintenance-launchdaemon-status-registered = El LaunchDaemon de arranque está registrado
maintenance-launchdaemon-status-not-registered = No hay ningún LaunchDaemon de arranque registrado (el daemon solo arranca tras el inicio de sesión).
maintenance-status-label-daemon-loaded =   LaunchDaemon cargado
maintenance-launchctl-load-failed = falló launchctl load: { $error }
maintenance-launchctl-run-failed = Error al ejecutar launchctl: { $error }
maintenance-windows-startup-added = Añadido al inicio de Windows (HKCU\Software\Microsoft\Windows\CurrentVersion\Run)
maintenance-windows-registry-write-failed = Error al escribir en el registro: { $error }
maintenance-windows-reg-run-failed = Error al ejecutar reg.exe: { $error }
maintenance-systemd-removed = Servicio de usuario de systemd eliminado
maintenance-systemd-remove-failed = Error al eliminar el archivo del servicio: { $error }
maintenance-systemd-not-found = No se encontró ningún servicio de usuario de systemd; no hay nada que eliminar.
maintenance-launchagent-removed = LaunchAgent eliminado
maintenance-launchagent-remove-failed = Error al eliminar el plist: { $error }
maintenance-launchagent-not-found = No se encontró ningún LaunchAgent; no hay nada que eliminar.
maintenance-windows-startup-removed = Eliminado del inicio de Windows
maintenance-windows-startup-not-found = No se encontró ninguna entrada de inicio; no hay nada que eliminar.
maintenance-systemd-status-registered = El servicio de usuario de systemd está registrado
maintenance-status-label-enabled =   Habilitado
maintenance-status-label-active =   Activo
maintenance-systemd-status-not-registered = No hay ningún servicio de usuario de systemd registrado.
maintenance-service-install-hint = Ejecuta `librefang service install` para configurarlo.
maintenance-launchagent-status-registered = El LaunchAgent está registrado
maintenance-status-label-loaded =   Cargado
maintenance-launchagent-status-not-registered = No hay ningún LaunchAgent registrado.
maintenance-windows-status-registered = La entrada de inicio de Windows está registrada
maintenance-windows-status-not-registered = No hay ninguna entrada de inicio registrada.
reset-confirm-message =   Esto eliminará todos los datos de { $path }
      Incluidos: configuración, base de datos, manifiestos de agentes y credenciales.
reset-confirm-prompt =   ¿Seguro? Escribe 'yes' para confirmar: 
reset-not-needed = Nada que restablecer: { $path } no existe.
maintenance-update-section = Actualización
maintenance-update-error-exe-path = No se puede determinar la ruta del ejecutable actual: { $error }
maintenance-update-error-check-release = Error al comprobar la última versión: { $error }
maintenance-update-warn-resolve-release = No se pudo resolver la última versión publicada: { $error }
maintenance-update-warn-resolve-release-fix = Vuelve a intentarlo más tarde, o pasa `--version <tag>` para apuntar a una versión concreta.
maintenance-update-available = Hay una versión publicada más reciente disponible: { $tag }
maintenance-update-run-hint = Ejecuta `librefang update` para instalarla.
maintenance-update-same-core = La versión publicada { $tag } usa el mismo núcleo de versión de la CLI que el binario actual ({ $current }).
maintenance-update-same-core-hint = Ejecuta `librefang update` si quieres la compilación publicada más reciente para esta línea de versión.
maintenance-update-ahead = La versión del binario actual { $current } está por delante de la versión publicada { $tag }.
maintenance-update-compare-unknown = No se pudo comparar el binario actual con la etiqueta de versión { $tag }.
maintenance-update-compare-unknown-hint = Si quieres esa versión exacta, ejecuta `librefang update --version <tag>`.
maintenance-update-unable-to-determine = No se puede determinar si hay una actualización disponible.
maintenance-update-unable-to-determine-hint = Vuelve a intentarlo más tarde, cuando GitHub Releases esté accesible.
maintenance-update-cannot-compare-safely = No se pudo comparar el binario actual con la etiqueta de versión { $tag } de forma segura.
maintenance-update-cannot-compare-safely-hint = Vuelve a ejecutar con `librefang update --version { $tag }` para instalarla explícitamente.
maintenance-update-windows-daemon-running-error = Detén el daemon en ejecución antes de actualizar en Windows.
maintenance-update-windows-daemon-running-error-fix = Ejecuta `librefang stop`, después `librefang update` y después `librefang start`.
maintenance-update-cli-success = CLI de LibreFang actualizada.
maintenance-update-merging-config-defaults = Combinando los nuevos valores predeterminados de configuración...
maintenance-update-restart-daemon-hint = Si el daemon está en ejecución, reinícialo con `librefang restart`.
maintenance-update-background-launched = Actualización lanzada en segundo plano.
maintenance-update-background-hint-terminal = Abre una terminal nueva cuando termine y ejecuta `librefang --version`.
maintenance-update-background-hint-restart = Si el daemon está en ejecución, reinícialo cuando termine la actualización.
maintenance-update-failed-error = La actualización falló: { $error }
maintenance-update-cargo-blocked = Este binario se instaló con cargo. Ejecutar `cargo install` desde el ejecutable activo está bloqueado a propósito.
maintenance-update-unofficial-path = La actualización automática solo admite la ruta de instalación oficial ({ $path }). Este binario se ejecuta desde otra ubicación.
maintenance-update-package-manager-hint = Si este binario vino de otro gestor de paquetes, actualízalo con ese gestor.

# doctor_cmd.rs
doctor-check-librefang-dir-ok = Directorio de LibreFang: { $path }
doctor-check-librefang-dir-fail = No se encontró el directorio de LibreFang.
doctor-check-librefang-dir-created = Directorio de LibreFang creado
doctor-check-librefang-dir-create-fail = Error al crear el directorio
doctor-check-librefang-dir-not-found-init = No se encontró el directorio de LibreFang. Ejecuta `librefang init` primero.
doctor-check-env-ok = Archivo .env (permisos correctos)
doctor-check-env-fixed = Archivo .env (permisos corregidos a 0600)
doctor-check-env-ok-generic = Archivo .env
doctor-check-env-loose-warn = El archivo .env tiene permisos laxos ({ $mode }), deberían ser 0600
doctor-check-env-not-found-warn = No se encontró el archivo .env (créalo con: librefang config set-key <provider>)
doctor-check-config-ok = Archivo de configuración: { $path }
doctor-check-config-syntax-fail = El archivo de configuración tiene errores de sintaxis: { $error }
doctor-check-config-not-found = No se encontró el archivo de configuración.
doctor-check-config-created = config.toml predeterminado creado
doctor-check-config-create-fail = Error al crear config.toml
doctor-check-cli-version = Versión de la CLI: { $version } (canal: { $channel })
doctor-check-update-available-warn = Actualización disponible: { $current } -> { $latest } (consulta https://github.com/librefang/librefang/releases)
doctor-check-cli-up-to-date = La CLI está al día
doctor-check-update-fail-warn = No se pudieron comprobar las actualizaciones (red no disponible)
doctor-check-daemon-running = Daemon en ejecución en { $url }
doctor-check-daemon-not-running-warn = El daemon no está en ejecución (inícialo con `librefang start`)
doctor-check-port-available = El puerto { $address } está disponible
doctor-check-port-in-use-warn = El puerto { $address } lo usa otro proceso
doctor-check-stale-daemon-json-removed = daemon.json obsoleto eliminado
doctor-check-stale-daemon-json-warn = Se encontró un daemon.json obsoleto (el daemon no está en ejecución). Ejecuta con --repair para limpiarlo.
doctor-check-db-ok = Archivo de base de datos (SQLite válido)
doctor-check-db-invalid-fail = El archivo de base de datos existe pero no es SQLite válido
doctor-check-db-not-found-warn = No hay archivo de base de datos (se creará en la primera ejecución)
doctor-check-disk-space-low-warn = Poco espacio en disco: { $count }MB disponibles
doctor-check-disk-space-ok = Espacio en disco: { $count }MB disponibles
doctor-check-manifests-ok = Los manifiestos de agente son válidos
doctor-check-manifest-invalid-fail = Manifiesto no válido { $file }: { $error }
doctor-check-home-dir-fail = No se pudo determinar el directorio personal
doctor-check-provider-key-rejected-warn = { $name } ({ $env_var }) - clave rechazada (401/403)
doctor-check-endpoint-reachable = Endpoint de { $name } accesible ({ $endpoint })
doctor-check-endpoint-unreachable-warn = Endpoint de { $name } inaccesible ({ $endpoint })
doctor-check-channel-token-format-warn = { $name } ({ $env_var }) - formato de token inesperado
doctor-check-config-env-missing-warn = La configuración hace referencia a { $env_var } pero no está definida en el entorno ni en .env
doctor-check-config-deser-ok = La configuración se deserializa en KernelConfig
doctor-check-exec-policy = Política de ejecución: mode={ $mode }, safe_bins={ $count }
doctor-check-include-file-ok = Archivo incluido: { $path }
doctor-check-include-file-missing-warn = Falta el archivo incluido: { $path }
doctor-check-include-file-not-found-fail = Archivo incluido no encontrado: { $path }
doctor-check-mcp-servers-count = Servidores MCP configurados: { $count }
doctor-check-mcp-empty-command-warn = El servidor MCP '{ $name }' tiene el comando vacío
doctor-check-mcp-empty-url-warn = El servidor MCP '{ $name }' tiene la URL vacía
doctor-check-mcp-empty-base-url-warn = El servidor MCP '{ $name }' tiene base_url vacía
doctor-check-mcp-no-compat-tools-warn = El servidor MCP '{ $name }' no tiene herramientas http_compat configuradas
doctor-check-mcp-compat-header-empty-name-warn = El servidor MCP '{ $name }' tiene una cabecera http_compat con el nombre vacío
doctor-check-mcp-compat-header-no-value-warn = El servidor MCP '{ $name }' tiene una cabecera http_compat sin value/value_env
doctor-check-mcp-compat-tool-empty-name-warn = El servidor MCP '{ $name }' tiene una herramienta http_compat con el nombre vacío
doctor-check-mcp-compat-tool-empty-path-warn = El servidor MCP '{ $name }' tiene una herramienta http_compat con la ruta vacía
doctor-check-config-deser-fail = La configuración falla la deserialización en KernelConfig: { $error }
doctor-check-skills-loaded = Habilidades cargadas: { $count }
doctor-check-skills-load-fail-warn = Error al cargar las habilidades: { $error }
doctor-check-skills-injection-ok = Todas las habilidades pasan el escaneo de inyección de prompts
doctor-check-mcp-catalog-templates = Plantillas del catálogo MCP: { $templates }
doctor-check-mcp-configured-servers = Servidores MCP configurados: { $configured }
doctor-check-running-agents = Agentes en ejecución: { $count }
doctor-check-daemon-uptime = Tiempo activo del daemon: { $hours }h { $mins }m
doctor-check-db-connectivity-ok = Conectividad con la base de datos: OK
doctor-check-db-status-fail = Estado de la base de datos: { $status }
doctor-check-health-detail-status-warn = El detalle de estado devolvió { $status }
doctor-check-health-detail-fail-warn = Error al consultar el estado del daemon: { $error }
doctor-check-skills-loaded-daemon = Habilidades cargadas en el daemon: { $count }
doctor-check-rust-version = Rust: { $version }
doctor-check-rust-not-found-fail = No se encontró el toolchain de Rust
doctor-check-python-version = Python: { $version }
doctor-check-python-not-found-warn = Python no encontrado (necesario para el runtime de habilidades en Python)
doctor-check-node-version = Node.js: { $version }
doctor-check-node-not-found-warn = Node.js no encontrado (necesario para el runtime de habilidades en Node)
doctor-prompt-create-dir =     ¿Crearlo ahora? [Y/n] 
doctor-prompt-create-config =     ¿Crear la configuración predeterminada? [Y/n] 
doctor-section-providers =   Proveedores LLM:
doctor-section-connectivity = 

  Conectividad de red:
doctor-section-channels = 

  Integraciones de canales:
doctor-section-config-val = 

  Validación de la configuración:
doctor-section-skills = 

  Habilidades:
doctor-check-skills-injection-warn = Aviso de inyección de prompt en la habilidad: { $name }
doctor-section-mcp-servers =
  Servidores MCP:
doctor-section-daemon-health =
  Estado del daemon:
doctor-check-daemon-mcp-status = Servidores MCP: { $configured } configurados, { $connected } conectados
doctor-check-daemon-mcp-health = Estado de los servidores MCP: { $healthy }/{ $total } correctos

doctor-suggest-groq = https://console.groq.com       (gratis, rápido)
doctor-suggest-gemini = https://aistudio.google.com    (nivel gratuito)
doctor-suggest-deepseek = https://platform.deepseek.com  (bajo coste)

desktop-install-launched = App de escritorio lanzada.
desktop-install-launch-fail = Error al lanzar { $path }: { $error }
desktop-install-launch-fail-generic = Error al lanzar la app de escritorio: { $error }
desktop-install-not-installed = LibreFang Desktop no está instalado.
desktop-install-prompt =   ¿Descargarlo e instalarlo ahora? [Y/n] 
desktop-install-skipped = Omitido. Puedes instalarlo más tarde:
desktop-install-skipped-brew =   brew install --cask librefang   (macOS)
desktop-install-skipped-manual =   O descárgalo de https://github.com/librefang/librefang/releases
desktop-install-fetching = Obteniendo la información de la última versión...
desktop-install-unsupported = Plataforma no compatible con la instalación automática de la app de escritorio.
desktop-install-download-manual = Descárgalo manualmente: https://github.com/librefang/librefang/releases
desktop-install-github-fail = Error al contactar con GitHub: { $error }
desktop-install-parse-fail = Error al analizar la información de la versión: { $error }
desktop-install-kv-asset = Recurso
desktop-install-downloading = Descargando...
desktop-install-download-fail = La descarga falló: { $error }
desktop-install-download-complete = Descarga completada.
desktop-install-installing = Instalando...
desktop-install-success = LibreFang Desktop instalado correctamente.
desktop-install-fail = La instalación falló: { $error }
desktop-install-running-installer = Ejecutando el instalador...

doctor-audit-vault-key-unset = LIBREFANG_VAULT_KEY sin definir; cifrado de la bóveda desactivado.
doctor-audit-vault-key-invalid-base64 = LIBREFANG_VAULT_KEY no es base64 válido: { $error }
doctor-audit-vault-key-invalid-base64-hint = Genera una con: openssl rand -base64 32
doctor-audit-vault-key-wrong-length = LIBREFANG_VAULT_KEY se decodifica a { $count } bytes; deben ser exactamente 32. Ten en cuenta que 32 caracteres ASCII NO son 32 bytes tras decodificar base64.
doctor-audit-vault-key-wrong-length-hint = Genera una clave nueva de 32 bytes: openssl rand -base64 32 (salida de 44 caracteres)
doctor-audit-vault-key-ok = LIBREFANG_VAULT_KEY se decodifica a 32 bytes.

doctor-audit-api-listen-no-config = config.toml no encontrado; se omite la comprobación de api_listen.
doctor-audit-api-listen-invalid-toml = config.toml no es TOML válido: { $error }
doctor-audit-api-listen-invalid-toml-hint = Edita ~/.librefang/config.toml o ejecuta `librefang doctor --repair`.
doctor-audit-api-listen-unset = api_listen sin definir en la configuración; el kernel usará el valor predeterminado.
doctor-audit-api-listen-invalid-addr = api_listen `{ $address }` no es una dirección de socket válida: { $error }
doctor-audit-api-listen-invalid-addr-hint = Usa el formato `host:port`, p. ej. `127.0.0.1:4545` o `[::1]:4545`.
doctor-audit-api-listen-port-zero = api_listen `{ $address }` usa el puerto 0 (efímero asignado por el SO); los clientes no podrán descubrir la URL del daemon tras el bind.
doctor-audit-api-listen-port-zero-hint = Elige un puerto explícito (predeterminado 4545), p. ej. `127.0.0.1:4545`.
doctor-audit-api-listen-privileged = El puerto { $port } de api_listen es privilegiado (<1024); el daemon fallará al hacer bind sin root.
doctor-audit-api-listen-privileged-hint = Usa un puerto >= 1024 (predeterminado 4545) salvo que necesites root a propósito.
doctor-audit-api-listen-ok = api_listen `{ $address }` se analiza correctamente.

doctor-audit-config-not-found = { $path } no existe.
doctor-audit-config-not-found-hint = Ejecuta `librefang init` para crear una configuración predeterminada.
doctor-audit-config-read-fail = Error al leer { $path }: { $error }
doctor-audit-config-ok = { $path } se analiza como TOML.
doctor-audit-config-syntax-error = { $path } tiene errores de sintaxis TOML: { $error }
doctor-audit-config-syntax-error-hint = Edita { $path } o restaura desde una copia de seguridad.

doctor-everyapi-absent = No se detectó ninguna pasarela EveryAPI (ni CLI en PATH ni credenciales); no hay nada que conectar.
doctor-everyapi-cli-not-logged-in = La CLI de EveryAPI está en PATH pero no se encontró ningún archivo de credenciales; ejecuta `everyapi login` primero si quieres enrutar LibreFang a través de ella.
doctor-everyapi-credentials-incomplete = El archivo de credenciales de EveryAPI existe pero no lleva ningún relay_key utilizable; vuelve a ejecutar `everyapi login` para actualizarlo.
doctor-everyapi-not-connected = Se encontraron credenciales de EveryAPI; LibreFang detectará automáticamente el proveedor gestionado y usará la clave de relay actual sin copiarla.
doctor-everyapi-provider-only = Entrada de proveedor EveryAPI activa en { $path }; los drivers de API se enrutan por la pasarela.
doctor-everyapi-provider-without-credentials = Existe una entrada de proveedor EveryAPI en { $path }, pero no se encontró ninguna clave de relay utilizable; las solicitudes a través de la pasarela fallarán la autenticación.
doctor-everyapi-provider-without-credentials-hint = Ejecuta `everyapi login` y después `librefang models connect everyapi` para actualizar la clave guardada.
doctor-everyapi-env-route-only = La variable de entorno { $var } redirige los drivers de subproceso de CLI (claude-code, codex-cli) a { $host }; LibreFang no tiene una entrada de proveedor correspondiente, así que los drivers de API siguen usando sus proveedores configurados.
doctor-everyapi-both-routes = Hay dos rutas de pasarela independientes activas a la vez: { $var } envía los drivers de subproceso de CLI (claude-code, codex-cli) a { $host }, mientras que la entrada de proveedor en { $path } enruta los drivers de API por separado. Cuál se aplica a un agente depende de su driver, no de ninguno de los dos ajustes.
doctor-everyapi-both-routes-hint = Conserva ambas solo si querías cubrir por separado los drivers de CLI y de API; si no, quita { $var } o elimina { $path } para que haya una única ruta activa.

# `librefang models connect everyapi`
everyapi-connect-title = Conectando la pasarela EveryAPI
everyapi-connect-unknown-target = Pasarela desconocida '{ $target }'.
everyapi-connect-unknown-target-fix = El único destino admitido hoy es `librefang models connect everyapi`.
everyapi-connect-credentials-missing = No se pudo determinar dónde guarda EveryAPI sus credenciales.
everyapi-connect-credentials-missing-fix = Ejecuta `everyapi login` primero y después vuelve a ejecutar este comando.
everyapi-connect-credentials-unreadable = No se pudo leer el archivo de credenciales de EveryAPI en { $path }.
everyapi-connect-credentials-malformed = El archivo de credenciales de EveryAPI en { $path } no es JSON válido.
everyapi-connect-credentials-no-api-base = El archivo de credenciales de EveryAPI en { $path } no lleva api_base.
everyapi-connect-credentials-no-relay-key = El archivo de credenciales de EveryAPI en { $path } no lleva relay_key.
everyapi-connect-models-fetch-failed = No se pudo obtener la lista de modelos de la pasarela; registrando el proveedor sin modelos.
everyapi-connect-models-fetch-failed-fix = Comprueba que la pasarela es accesible y vuelve a ejecutar este comando para completar la lista de modelos.
everyapi-connect-models-fetch-unauthorized = La pasarela rechazó la clave de relay, así que no se escribió nada.
everyapi-connect-models-fetch-unauthorized-fix = Ejecuta `everyapi login` para actualizar la clave y vuelve a ejecutar este comando.
everyapi-connect-models-fetch-failed-would-clobber = No se pudo obtener la lista de modelos de la pasarela y la entrada de proveedor existente ya lleva modelos; se rechaza reemplazarla por una vacía.
everyapi-connect-provider-rejected = El daemon rechazó la definición del proveedor: { $error }
everyapi-connect-pricing-fetch-failed = No se pudo obtener la información de precios de la pasarela; registrando los modelos sin precio por token.
everyapi-connect-pricing-fetch-failed-fix = Comprueba que la pasarela es accesible y vuelve a ejecutar este comando para completar los precios. Hasta entonces, el gasto en estos modelos no se contará en ningún presupuesto.
everyapi-connect-serialize-failed = Error al construir el catálogo de proveedores: { $error }
everyapi-connect-key-saved = Clave de relay guardada en el archivo .env de LibreFang como { $env_var }.
everyapi-connect-key-save-failed = Error al guardar la clave de relay: { $error }
everyapi-connect-provider-written-daemon = Proveedor registrado mediante el daemon en ejecución en { $path }; no hace falta reiniciar.
everyapi-connect-provider-written-file = Proveedor escrito en { $path }.
everyapi-connect-provider-write-failed = Error al escribir el archivo del proveedor: { $error }
everyapi-connect-restart-required = Reinicia el daemon para aplicar el nuevo proveedor.
everyapi-connect-models-registered = { $count } modelos registrados.
everyapi-connect-models-skipped = { $count } modelos omitidos.
everyapi-connect-skip-no-metadata = { $model }: no se conoce la ventana de contexto ni el límite de salida de este modelo, y un modelo de texto sin ambos se descarta al cargar el catálogo.
everyapi-connect-skip-no-context-window = { $model }: se conoce un límite de salida para este modelo pero no la ventana de contexto, y un modelo de texto sin ambos se descarta al cargar el catálogo.
everyapi-connect-skip-no-output-limit = { $model }: se conoce la ventana de contexto de este modelo pero no el límite de salida, y un modelo de texto sin ambos se descarta al cargar el catálogo.
everyapi-connect-token-limits-borrowed = La pasarela no publica ventana de contexto ni límite de salida para estos modelos, así que el dato que falta se tomó de la instantánea integrada de OpenRouter y describe esa copia del modelo, no la de esta pasarela: { $models }
everyapi-connect-models-unpriced = Estos modelos no llevan precio por token (no aparecen en la información de precios de la pasarela o se facturan por llamada), así que su gasto se registra como desconocido en lugar de gratuito: { $models }
everyapi-connect-streaming-only = Estos modelos solo aceptan solicitudes en streaming y fallarán cualquier llamada sin streaming (compactación, memoria proactiva, taller de habilidades, ampliación web): { $models }
everyapi-connect-default-hint = Vuelve a ejecutar con --set-default para hacer de { $model } el modelo predeterminado del daemon.
everyapi-connect-default-set = Modelo predeterminado definido como { $model }.
everyapi-connect-default-failed = Error al definir { $model } como modelo predeterminado (HTTP { $status }).
everyapi-connect-default-needs-daemon = El daemon no está en ejecución, así que el modelo predeterminado no cambió. Inícialo y vuelve a ejecutar `librefang models connect everyapi --set-default`.
everyapi-connect-default-url-pin-failed = No se pudo fijar la URL de la pasarela en config.toml; el modelo predeterminado se definió igualmente, pero un daemon reiniciado puede no resolverlo.
everyapi-connect-default-no-candidate = No se registró ningún modelo de texto, así que no hay nada que definir como predeterminado.

doctor-audit-desktop-deps-no-pkg-config = pkg-config no encontrado; no se puede verificar el stack GTK/WebKit que necesita la app de escritorio. Una instalación solo de CLI no lo necesita.
doctor-audit-desktop-deps-ok = Stack GTK/WebKit de escritorio presente; pkg-config resuelve { $module }.
doctor-audit-desktop-deps-webkit-missing = pkg-config resuelve { $gtk } pero no WebKitGTK ({ $webkit }); la app de escritorio no puede renderizar su ventana.
doctor-audit-desktop-deps-stack-missing = No se encontró ningún stack de escritorio GTK/WebKit; la app de escritorio necesita { $modules }, la CLI no.
doctor-audit-desktop-deps-distro-unknown = esta distribución de Linux
doctor-audit-desktop-deps-hint-apt = En { $distro }, busca los paquetes que proporcionan { $modules } con `apt-cache search webkit2gtk` y luego instálalos con `sudo apt install <package>`. Los nombres de paquete cambian entre versiones, así que busca en lugar de suponer.
doctor-audit-desktop-deps-hint-pacman = En { $distro }, busca los paquetes que proporcionan { $modules } con `pacman -Ss webkit2gtk` y luego instálalos con `sudo pacman -S <package>`. Los nombres de paquete cambian entre versiones, así que busca en lugar de suponer.
doctor-audit-desktop-deps-hint-dnf = En { $distro }, busca los paquetes que proporcionan { $modules } con `dnf search webkit2gtk` y luego instálalos con `sudo dnf install <package>`. Los nombres de paquete cambian entre versiones, así que busca en lugar de suponer.
doctor-audit-desktop-deps-hint-nix = En { $distro }, instalar bibliotecas de desarrollo de forma imperativa no funciona; usa el devShell del flake o el paquete librefang-desktop en lugar de instalar { $modules } a mano.
doctor-audit-desktop-deps-hint-generic = En { $distro }, instala los paquetes de desarrollo de GTK 3 y WebKitGTK que proporcione tu gestor de paquetes, hasta que pkg-config resuelva { $modules }.

# launcher menu items
launcher-menu-get-started = Empezar
launcher-menu-get-started-hint = Proveedores, claves API, modelos, migración
launcher-menu-settings = Ajustes
launcher-menu-settings-hint = Proveedores, claves API, modelos, enrutamiento
launcher-menu-chat = Chatear con un agente
launcher-menu-chat-hint = Chat rápido en la terminal
launcher-menu-dashboard = Abrir el panel
launcher-menu-dashboard-hint = Lanza la interfaz web en tu navegador
launcher-menu-desktop = Abrir la app de escritorio
launcher-menu-desktop-hint = Lanza la app de escritorio nativa
launcher-menu-tui = Abrir la interfaz de terminal
launcher-menu-tui-hint = Panel TUI interactivo completo
launcher-menu-help = Mostrar todos los comandos
launcher-menu-help-hint = Imprime la salida completa de --help

# launcher screen strings
launcher-welcome = ¡Bienvenido! Vamos a configurarlo.
launcher-checking-daemon = Buscando el daemon…
launcher-daemon-running = Daemon en ejecución en { $url }
launcher-daemon-agents = { $count ->
    [one]  ({ $count } agente)
   *[other]  ({ $count } agentes)
}
launcher-daemon-no-running = No hay ningún daemon en ejecución
launcher-provider = Proveedor: { $provider }
launcher-no-keys = No se detectaron claves API
launcher-hint-re-run =   Ejecuta 'Volver a configurar' para configurar un proveedor
launcher-hint-get-started =   Selecciona 'Empezar' para configurar
launcher-migration-question = ¿Vienes de { $source }? 
launcher-migration-hint = 'Empezar' incluye la migración automática.
launcher-menu-hints = ↑↓/jk navegar  1-9 selección rápida  enter confirmar  q salir
launcher-help-title = Todos los comandos
launcher-help-subtitle =   — q/Esc para volver
launcher-help-hints = ↑↓/jk desplazar  PgUp/PgDn  g/G inicio/fin  q volver

# CLI shared UI strings
common-warning-config-default = aviso: { $error }; usando valores de configuración predeterminados para este comando
ui-brand-tagline = El sistema operativo de agentes de código abierto
ui-brand-title = LibreFang Agent OS
ui-label-hint = sugerencia:
ui-label-next-steps = Siguientes pasos
ui-label-fix = corrección:
ui-label-try = prueba:
ui-provider-not-set = { $env_var } sin definir
progress-fail = [FALLÓ]

# Table headers / Shared labels
label-header-name = NOMBRE
label-header-kind = TIPO
label-header-configured = CONFIGURADO
label-header-token = TOKEN
label-header-alias = ALIAS
label-header-provider = PROVEEDOR
label-header-id = ID
label-header-agent = AGENTE
label-header-category = CATEGORÍA
label-header-description = DESCRIPCIÓN
label-header-hand = HAND
label-header-instance = INSTANCIA
label-header-model = MODELO
label-header-status = ESTADO
label-header-type = TIPO
label-header-timestamp = MARCA DE TIEMPO
label-header-event = EVENTO
label-header-key = CLAVE
label-header-value = VALOR
label-header-enabled = HABILITADO
label-header-url = URL

# Channel command specific keys
# Scoped by channel TYPE, not by sidecar instance — usage_events.channel stores the type, so every bot of the same type shares the count (#6606).
channel-header-msgs-24h-by-type = MENSAJES 24H (POR TIPO)
channel-header-connected = CONECTADO
channel-header-in-out = ENT/SAL
channel-liveness-not-started = NO INICIADO
channel-last-errors-heading = Último error notificado por canal (persistente: se registra al fallar y no se borra al recuperarse):
channel-last-error-entry = { $name }: { $error }
channel-error-save-failed-no-body = falló el guardado (sin cuerpo de error)

# Models command specific keys
model-none-in-catalog = No hay modelos en el catálogo.
model-header-model = MODELO
model-header-tier = NIVEL
model-header-context = CONTEXTO
model-header-resolves-to = SE RESUELVE A
model-header-auth = AUTENTICACIÓN
model-header-models = MODELOS
model-header-base-url = URL BASE
model-picker-item =     { $idx }. { $id } { $tier }
model-overrides-cleared = Valores sobrescritos borrados para el modelo { $model }.
model-overrides-clear-failed = Error al borrar los valores sobrescritos de { $model }: { $status }
model-overrides-header-field = CAMPO
model-overrides-header-value = VALOR
model-overrides-none-set = No hay valores sobrescritos definidos para { $model }.
model-overrides-saved = Valores sobrescritos guardados para { $model }.
model-overrides-save-failed = Error al guardar los valores sobrescritos de { $model }: { $status }

# Approvals command specific keys
approval-none-pending = No hay aprobaciones pendientes.
approval-header-status = ESTADO
approval-header-request = SOLICITUD

# Auth command specific keys
auth-error-create-home-dir = Error al crear el directorio personal de LibreFang: { $error }
auth-error-write-secrets = Error al escribir secrets.env: { $error }
auth-error-parse-config = Error al analizar config.toml: { $error }
auth-error-default-model-not-table = default_model no es una tabla
auth-error-write-config = Error al escribir config.toml: { $error }
auth-pool-add-hint = Añade uno con:
auth-pool-add-example =   librefang auth pool add openai OPENAI_API_KEY_1 --label Primary --priority 10
auth-pool-header = { $provider }  ({ $strategy })
auth-pool-keys-available =   claves: { $available }/{ $total } disponibles
auth-pool-cooldown-left = (quedan { $secs }s)
auth-pool-status-invalid = no válida
auth-pool-status-exhausted = agotada
auth-pool-status-cooldown = enfriamiento
auth-pool-status-env-missing = falta en el entorno
auth-pool-status-healthy = correcta
auth-pool-key-requests = solicitudes={ $count }
auth-pool-key-item =     - [{ $label }] { $key_display }  priority={ $pri }{ $reqs_str }  status={ $status }
auth-hash-add-config-hint = Añade a config.toml:
auth-hash-config-entry =   dashboard_pass_hash = "{ $hash }"
auth-enter-api-key-prompt = Introduce la clave API:
auth-api-key-empty = La clave API no puede estar vacía.
auth-api-key-generated = Nueva clave API (entrégala a tus clientes; solo se muestra una vez):
auth-api-key-config-entry =   api_key_hash = "{ $hash }"
auth-api-key-remove-plaintext-hint = Después elimina la línea `api_key` en texto plano. Los clientes siguen enviando la clave; solo cambia la copia que guarda el daemon.
auth-api-key-cli-driver-caveat = Excepción: si usas un driver basado en CLI (claude-code), conserva una copia transmisible: `api_key = "vault:NAME"` o la variable LIBREFANG_API_KEY. Esos drivers llaman al endpoint /mcp del propio daemon, y un hash no puede enviarse como token bearer, así que sin ninguna de las dos cada llamada a herramienta recibirá 401.

# Agent command specific keys
agent-spawn-id-label =   ID:   { $id }
agent-spawn-name-label =   Nombre: { $name }
error-unknown = Error desconocido
label-unknown = <desconocido>
label-header-state = ESTADO
label-header-created = CREADO
label-header-msgs = MSJS
label-header-last-active = ÚLTIMA ACTIVIDAD
label-session-state-running = en ejecución
label-session-state-idle = inactiva

# Daemon command specific keys
daemon-error-resolve-exe = resolver el ejecutable actual: { $error }
daemon-error-create-log-dir = crear el directorio de registros { $path }: { $error }
daemon-error-open-log = abrir el registro del daemon { $path }: { $error }
daemon-error-clone-log-handle = clonar el handle del registro del daemon { $path }: { $error }
daemon-error-spawn-detached = lanzar el daemon independiente: { $error }
daemon-error-failed-create-log-dir = Error al crear el directorio de registros { $path }: { $error }
daemon-error-failed-open-log = Error al abrir el archivo de registro del daemon { $path }: { $error }

# --- Skill commands ---
skill-name-empty = el nombre de la habilidad está vacío
skill-name-unsafe = nombre de habilidad inseguro '{ $name }': debe ser un único componente de ruta (sin '/', '\', '..' ni ruta absoluta)
skill-hand-not-found = Hand '{ $hand }' no encontrado en { $path }
skill-openclaw-detected = Formato de habilidad OpenClaw detectado. Convirtiendo...
skill-install-refused = Se rechaza instalar la habilidad: { $error }
skill-write-manifest-failed = Error al escribir el manifiesto: { $error }
skill-openclaw-installed-to-hand = Habilidad OpenClaw '{ $name }' instalada en el Hand '{ $hand }'
skill-openclaw-installed = Habilidad OpenClaw instalada: { $name }
skill-openclaw-convert-failed = Error al convertir la habilidad OpenClaw: { $error }
skill-no-toml = No se encontró skill.toml en { $path }
skill-read-toml-failed = Error al leer skill.toml: { $error }
skill-parse-toml-failed = Error al analizar skill.toml: { $error }
skill-installed-to-hand = Habilidad '{ $name }' v{ $version } instalada en el Hand '{ $hand }'
skill-installed = Habilidad instalada: { $name } v{ $version }
skill-installed-hub-to-hand = { $source } { $version } instalada en el Hand '{ $hand }'
skill-installed-hub = { $source } { $version } instalada
skill-install-failed = Error al instalar la habilidad: { $error }
skill-list-none-hand = No hay habilidades instaladas para el Hand '{ $hand }'.
skill-list-none = No hay habilidades instaladas.
skill-list-count-hand = { $count } habilidad(es) instalada(s) para el Hand '{ $hand }':
skill-list-count = { $count } habilidad(es) instalada(s):
skill-list-load-failed = Error al cargar las habilidades: { $error }
skill-removed-from-hand = Habilidad '{ $name }' eliminada del Hand '{ $hand }'
skill-removed = Habilidad eliminada: { $name }
skill-remove-failed = Error al eliminar la habilidad: { $error }
skill-search-none = No se encontraron habilidades para "{ $query }".
skill-search-results-header = Habilidades que coinciden con "{ $query }":
skill-search-install-hint = instala con: librefang skill install { $id }
skill-search-failed = La búsqueda falló: { $error }
skill-validation-failed = Falló la validación de la habilidad: { $error }
skill-execution-failed = Falló la ejecución de la habilidad: { $error }
skill-package-failed = Error al empaquetar la habilidad: { $error }
skill-determine-dir-failed = No se pudo determinar el directorio actual: { $error }
skill-unsupported-runtime = Runtime no compatible '{ $runtime }'. Elige uno de: python, node, wasm.
skill-create-dir-failed = Error al crear el directorio de la habilidad: { $error }
skill-create-exists = Ya existe una habilidad en { $path }. Elige otro nombre o elimínala primero.
skill-create-write-failed = Error al escribir { $path }: { $error }
skill-create-name-invalid = Nombre de habilidad no válido '{ $name }': usa solo letras, dígitos, '-' y '_'.
skill-registry-load-failed = Error al cargar el registro de habilidades: { $error }
skill-not-found = Habilidad '{ $name }' no encontrada en { $path }
skill-read-file-failed = Error al leer { $path }: { $error }
skill-create-skills-dir-failed = Error al crear el directorio de habilidades: { $error }
skill-create-failed = La creación falló: { $error }
skill-update-failed = La actualización falló: { $error }
skill-patch-failed = El parche falló: { $error }
skill-delete-failed = El borrado falló: { $error }
skill-rollback-failed = La reversión falló: { $error }
skill-write-file-failed = Falló la escritura de archivo: { $error }
skill-remove-file-failed = Falló el borrado de archivo: { $error }
skill-serialize-history-failed = Error al serializar el historial: { $error }
skill-read-pending-failed = Error al leer el directorio de pendientes: { $error }
skill-load-candidate-failed = Error al cargar el candidato: { $error }
skill-render-candidate-failed = Error al renderizar el candidato como TOML: { $error }
skill-approve-candidate-failed = La aprobación falló: { $error }
skill-reject-candidate-failed = El rechazo falló: { $error }
skill-publish-failed = La publicación falló: { $error }
skill-marketplace-config-invalid = [skills.promotion] no válido en config.toml: { $error }
skill-evolution-label = Habilidad: { $name }
skill-version-label = Versión actual: { $version }
skill-use-count-label = Veces usada: { $count }
skill-evolution-count-label = Veces evolucionada: { $count }
skill-no-history = No hay historial de versiones registrado.
skill-no-pending = No hay candidatos de habilidad pendientes.{ $filter }
skill-pending-filter =  (filtro: agente { $agent })
skill-approved-candidate = Candidato { $id } aprobado → habilidad '{ $name }' instalada (v{ $version }).
skill-rejected-candidate = Candidato { $id } rechazado y eliminado.
skill-validated = Habilidad validada: { $name } v{ $version }
skill-validated-runtime =   Runtime: { $runtime }
skill-validated-source =   Origen: { $path }
skill-validated-description =   Descripción: { $description }
skill-validated-tools =   Herramientas: { $tools }
skill-refusing-warnings = Se rechaza ejecutar una habilidad con avisos críticos de validación.
skill-validated-only = Solo validación: no hay ninguna herramienta declarada que ejecutar.
skill-invalid-input-json = JSON de --input no válido: { $error }
skill-tool-result-header = Resultado de la herramienta ({ $name }):
skill-validation-complete = Validación completada.
skill-execution-skipped = Ejecución omitida: { $message }
skill-preparing = Preparando la habilidad: { $name } v{ $version }
skill-refusing-publish = Se rechaza publicar una habilidad con avisos críticos de validación.
skill-bundle-created = Paquete creado: { $path }
skill-bundle-sha =   SHA256: { $sha }
skill-bundle-size =   Tamaño: { $size } bytes
skill-dry-run = Solo simulación.
skill-dry-run-repo =   Repo: { $repo }
skill-dry-run-tag =   Etiqueta: { $tag }
skill-github-token-required = No se encontró ningún token de GitHub. Define GITHUB_TOKEN o GH_TOKEN, guarda uno en la bóveda con `librefang vault set GITHUB_TOKEN`, o vuelve a ejecutar con --dry-run.
skill-publishing-progress = Publicando { $name }@{ $tag }
skill-publish-success = { $name } publicado en { $repo }@{ $tag }
skill-publish-release-url = Versión: { $url }
skill-warnings-none =   Avisos: ninguno
skill-warnings-header =   Avisos:
skill-prompt-name = Nombre de la habilidad: 
skill-prompt-description = Descripción: 
skill-prompt-runtime = Runtime (python/node/wasm) [python]: 
skill-created = Habilidad creada: { $path }
skill-created-files-header = Archivos:
skill-created-next-steps-header = Siguientes pasos:
skill-created-step-edit =   { $step }. Edita el punto de entrada para implementar la lógica de tu habilidad
skill-created-step-test =   { $step }. Prueba en local: librefang skill test { $path }
skill-created-step-install =   { $step }. Instala: librefang skill install { $path }

# --- Monitoring & Status ---
monitoring-no-audit = No hay entradas de auditoría.
monitoring-no-memory = No hay entradas de memoria para el agente '{ $agent }'.
monitoring-no-devices = No hay dispositivos emparejados.
monitoring-no-webhooks = No hay webhooks configurados.
label-last-seen = ÚLTIMA VEZ VISTO
status-watch-header =   { $status } (actualizando cada { $interval }s, Ctrl+C para salir)
status-warning-config = aviso: { $error }; usando valores de configuración predeterminados para mostrar el estado
status-summary-up = librefang { $version } estado={ $state } tiempo activo={ $uptime } { $auth } ({ $base })
status-peers-connected = { $connected } conectados / { $total } conocidos
status-agents-active = { $active } en ejecución / { $total } en total
status-mb = { $mb } MB
status-summary-down = librefang detenido inicio={ $home } predeterminado={ $provider }/{ $model }
status-uptime-format = { $hours }h { $mins }m
# --- Brand/proper names ---
brand-openai = OpenAI
brand-openrouter = OpenRouter
brand-deepseek = DeepSeek
brand-deepinfra = DeepInfra
brand-byteplus = BytePlus
brand-azure-openai = Azure OpenAI
brand-github-copilot = GitHub Copilot
brand-huggingface = Hugging Face
brand-openai-codex = OpenAI Codex
brand-claude-code = Claude Code
brand-vertex-ai = Vertex AI
brand-nvidia-nim = NVIDIA NIM
brand-zai = Z.ai
brand-kimi-coding = Kimi Coding
brand-alibaba-coding-plan = Alibaba Coding Plan
brand-slack-app = Slack App
brand-slack-bot = Slack Bot
brand-telegram = Telegram
brand-discord = Discord
brand-openclaw-openfang = OpenClaw / OpenFang
brand-openclaw = OpenClaw
brand-openfang = OpenFang

# --- Number/unit formatting templates ---
format-bytes-gib = { $value } GiB
format-bytes-mib = { $value } MiB
format-bytes-kib = { $value } KiB
format-bytes-b = { $value } B
format-size-mb = ({ $value } MB)

format-uptime-s = { $secs }s
format-uptime-ms = { $mins }m { $secs }s
format-uptime-hm = { $hours }h { $mins }m
format-uptime-hms = { $hours }h { $mins }m { $secs }s
format-uptime-dh = { $days }d { $hours }h
format-uptime-dhm = { $days }d { $hours }h { $mins }m

# --- Desktop install & Update errors ---
desktop-install-unsupported-platform = Plataforma no compatible
desktop-install-error-hdiutil-attach = falló hdiutil attach: { $error }
desktop-install-error-app-not-found = No se encontró LibreFang.app en el DMG
desktop-install-error-remove-old = Error al eliminar la instalación anterior: { $error }
desktop-install-error-cp = falló cp: { $error }
desktop-install-error-copy-applications = Error al copiar a /Applications: { $error }
desktop-install-error-run-installer = Error al ejecutar el instalador: { $error }
desktop-install-error-installer-status = El instalador terminó con: { $status }
desktop-install-error-localappdata = No se puede determinar %LOCALAPPDATA%
desktop-install-error-binary-not-found = El instalador terminó pero no se encontró el binario en la ubicación esperada
desktop-install-error-home-dir = No se puede determinar el directorio personal
desktop-install-error-create-dir = Error al crear { $path }: { $error }
desktop-install-error-copy-appimage = Error al copiar el AppImage: { $error }
desktop-install-error-http = La solicitud HTTP falló: { $error }
desktop-install-error-create = No se puede crear { $path }: { $error }
desktop-install-error-write = Error de escritura: { $error }
desktop-install-error-too-large = el instalador descargado supera el límite de seguridad de { $limit } MiB (observados al menos { $bytes } bytes); abortando
desktop-install-gatekeeper-notice = macOS puede pedirte confirmación para abrir LibreFang la primera vez; es Gatekeeper verificando la notarización de la app.

maintenance-error-github-request = La solicitud a GitHub falló: { $error }
maintenance-error-github-status = La API de GitHub devolvió { $status }
maintenance-error-decode-release = Error al decodificar los metadatos de la versión: { $error }
maintenance-error-missing-tag = Faltan los metadatos de la versión `tag_name`
maintenance-error-decode-list = Error al decodificar la lista de versiones: { $error }
maintenance-error-no-release = No se encontró ninguna versión para el canal '{ $channel }'
maintenance-error-http-client = Error al crear el cliente HTTP: { $error }
maintenance-error-powershell-updater = Error al lanzar el actualizador de PowerShell: { $error }
maintenance-error-run-installer = Error al ejecutar el instalador: { $error }
maintenance-error-installer-status = El instalador terminó con estado { $status }
maintenance-error-download-fail = La descarga falló: { $error }
maintenance-error-download-status = La descarga devolvió { $status }
maintenance-error-read-response = Error al leer el cuerpo de la respuesta: { $error }
maintenance-error-create-dir = Error al crear el directorio del actualizador: { $error }
maintenance-error-create-script = Error al crear el script del actualizador: { $error }
maintenance-error-write-script = Error al escribir el script del actualizador: { $error }

common-error-find-exe = No se encuentra el ejecutable: { $error }
common-error-spawn-daemon = Error al lanzar el daemon: { $error }
common-error-daemon-timeout = El daemon no estuvo listo en 10 segundos

# tui/chat_runner.rs
chat-runner-owner-notice = [owner_notice] { $preview }
chat-runner-error-prefix = Error: { $error }
chat-runner-no-active-connection = No hay conexión activa
chat-runner-unknown-command = Comando desconocido: { $command }. Escribe /help
chat-runner-status-mode-daemon = Modo: daemon ({ $url })
chat-runner-status-agent = Agente: { $name }
chat-runner-status-mode-inprocess = Modo: en proceso
chat-runner-status-agents-count = Agentes: { $count }
chat-runner-status-mode-disconnected = Modo: desconectado
chat-runner-chat-history-cleared = Historial de chat borrado.
chat-runner-session-reset = Sesión restablecida; empezando de cero.
chat-runner-session-reset-failed = No se pudo restablecer la sesión.
chat-runner-thinking-set = Esfuerzo de razonamiento definido como { $level } para esta sesión.
chat-runner-thinking-cleared = Esfuerzo de razonamiento borrado; se aplica el valor predeterminado del agente.
chat-runner-thinking-invalid = Argumento desconocido '{ $argument }'. { $usage }
chat-runner-thinking-failed = No se pudo definir el esfuerzo de razonamiento: { $error }
chat-runner-thinking-no-session = No se pudo resolver la sesión de este chat en el daemon.
chat-runner-agent-killed = Agente "{ $name }" terminado.
chat-runner-failed-kill-agent = Error al terminar el agente "{ $name }".
chat-runner-kill-failed = La terminación falló: { $error }
chat-runner-no-backend-connected = No hay ningún backend conectado.
chat-runner-goal-daemon-unsupported = /goal aún no es compatible cuando estás conectado a un daemon; crea e inicia el objetivo desde el panel.
chat-runner-no-models-available = No hay modelos disponibles.
chat-runner-switched-model = Cambiado a { $model }
chat-runner-failed-switch-model = Error al cambiar a { $model }
chat-runner-switch-failed = El cambio falló: { $error }
chat-runner-welcome-help-hint = /help para los comandos • /exit para salir
chat-runner-spawning-agent = Creando el agente '{ $name }'…
chat-runner-no-agent-templates = No se encontraron plantillas de agente. Ejecuta `librefang init`.
chat-runner-invalid-template = Plantilla no válida '{ $name }': { $error }
chat-runner-spawn-failed = La creación falló: { $error }
chat-runner-booting-kernel = Iniciando el kernel…
chat-runner-booting-kernel-hint =   Puede tardar un momento mientras el kernel se inicializa.
chat-runner-failed-start = Error al iniciar
chat-runner-press-esc-to-exit =   Pulsa Esc para salir.

# tui/event.rs
tui-event-workflow-completed = Flujo de trabajo completado
tui-event-workflow-still-running = Flujo de trabajo iniciado; sigue en ejecución (ejecución { $id })
tui-event-workflow-run-failed = La ejecución del flujo de trabajo falló ({ $status }): { $detail }
tui-event-workflow-exec-not-available-in-process = La ejecución de flujos de trabajo no está disponible en modo en proceso
tui-event-workflow-create-not-available-in-process = La creación de flujos de trabajo no está disponible en modo en proceso
tui-event-workflow-steps-empty = Los pasos son obligatorios: introduce un array JSON de objetos de paso
tui-event-workflow-steps-invalid = Los pasos no son JSON válido: { $error }
tui-event-workflow-steps-not-array = Los pasos deben ser un array JSON de objetos de paso
tui-event-workflow-create-failed = Falló la creación del flujo de trabajo ({ $status }): { $detail }
tui-event-trigger-create-not-available-in-process = La creación de activadores no está disponible en modo en proceso
tui-event-trigger-delete-failed = Error al eliminar el activador { $trigger_id }
tui-event-trigger-delete-not-available-in-process = La eliminación de activadores no está disponible en modo en proceso
tui-event-agent-kill-failed = Error al terminar el agente { $agent_id }
tui-event-agent-invalid-id = ID de agente no válido: { $agent_id }
tui-event-skills-fetch-failed = Error al obtener las habilidades
tui-event-mcp-fetch-failed = Error al obtener los servidores MCP
tui-event-channels-fetch-failed = Error al obtener los canales
tui-event-skills-update-failed = Error al actualizar las habilidades
tui-event-manifest-history-fetch-failed = Error al cargar el historial de configuración del agente
tui-event-model-params-fetch-failed = Error al cargar los parámetros del modelo del agente
tui-event-model-params-update-failed = Error al guardar los parámetros del modelo
tui-event-model-params-daemon-only = Los parámetros del modelo se editan a través del daemon; inícialo y vuelve a conectarte.
tui-event-skills-update-error = Actualización de habilidades: { $error }
tui-event-mcp-update-failed = Error al actualizar los servidores MCP
tui-event-channels-update-failed = Error al actualizar los canales
tui-event-mcp-update-error = Actualización de MCP: { $error }
tui-event-channels-update-error = Actualización de canales: { $error }
tui-event-session-delete-failed = Error al eliminar la sesión { $session_id }
tui-event-session-management-not-available-in-process = La gestión de sesiones no está disponible en modo en proceso
tui-event-promote-missing-pr-url = Falta la URL del PR en la respuesta
tui-event-promote-http-error = { $status }: { $detail }
tui-event-promote-not-available-in-process = Promover solo está disponible cuando se está conectado a un daemon
tui-event-kv-save-failed = Error al guardar el par KV
tui-event-kv-not-available-in-process = La memoria KV no está disponible en modo en proceso
tui-event-kv-delete-failed = Error al eliminar el par KV
tui-event-daemon-http-status = HTTP { $status }
tui-event-daemon-failure-detail = { $summary }: { $detail }
tui-event-dashboard-status-fetch-failed = Error al cargar el estado del daemon
tui-event-dashboard-audit-fetch-failed = Error al cargar la actividad reciente
tui-event-workflows-fetch-failed = Error al cargar los flujos de trabajo
tui-event-triggers-fetch-failed = Error al cargar los activadores
tui-event-sessions-fetch-failed = Error al cargar las sesiones
tui-event-agents-fetch-failed = Error al cargar los agentes
tui-event-kv-fetch-failed = Error al cargar la memoria del agente { $agent_id }
tui-event-clawhub-search-failed = La búsqueda en ClawHub falló
tui-event-clawhub-browse-failed = Error al explorar ClawHub
tui-event-templates-fetch-failed = Error al cargar las plantillas de agente
tui-event-providers-fetch-failed = Error al cargar los proveedores
tui-event-security-fetch-failed = Error al cargar el estado de seguridad
tui-event-audit-fetch-failed = Error al cargar el registro de auditoría
tui-event-usage-summary-fetch-failed = Error al cargar el resumen de uso
tui-event-usage-by-model-fetch-failed = Error al cargar el uso por modelo
tui-event-usage-by-agent-fetch-failed = Error al cargar el uso por agente
tui-event-tools-fetch-failed = Error al cargar las herramientas
tui-event-groups-fetch-failed = Error al cargar los grupos
tui-event-peers-fetch-failed = Error al cargar los pares
tui-event-logs-fetch-failed = Error al cargar los registros
tui-event-goals-fetch-failed = Error al cargar los objetivos
tui-event-hands-fetch-failed = Error al cargar los Hands
tui-event-active-hands-fetch-failed = Error al cargar los Hands activos
tui-event-extensions-fetch-failed = Error al cargar las extensiones
tui-event-extension-health-fetch-failed = Error al cargar el estado de las extensiones
tui-event-comms-topology-fetch-failed = Error al cargar la topología de agentes
tui-event-comms-events-fetch-failed = Error al cargar los eventos de comunicación
tui-event-agent-model-label-fetch-failed = Error al cargar el modelo del agente { $agent_id }
tui-event-skill-install-failed = Error al instalar { $slug }
tui-event-skill-install-not-available-in-process = La instalación de habilidades no está disponible en modo en proceso
tui-event-skill-uninstall-failed = Error al desinstalar { $name }
tui-event-skill-uninstall-not-available-in-process = La desinstalación de habilidades no está disponible en modo en proceso
tui-event-security-verification-complete = Verificación completada
tui-event-security-chain-not-applicable = Modo en proceso: la cadena no es aplicable
tui-event-provider-save-key-failed = Error al guardar la clave de { $name }
tui-event-provider-key-management-not-available-in-process = La gestión de claves de proveedor no está disponible en modo en proceso
tui-event-vault-not-available-in-process = La gestión de claves de la bóveda no está disponible en modo en proceso
tui-event-vault-save-failed = Error al guardar { $key }
tui-event-vault-delete-failed = Error al borrar { $key }
tui-event-vault-list-failed = Error al cargar las claves de la bóveda
tui-event-backups-list-failed = Error al cargar las copias de seguridad
tui-event-backups-need-daemon = Las copias de seguridad necesitan un daemon en ejecución
tui-event-backup-create-failed = Error al crear la copia de seguridad
tui-event-backup-delete-failed = Error al eliminar la copia de seguridad { $filename }
tui-event-backup-restore-failed = Falló la restauración desde { $filename }
tui-event-provider-delete-key-failed = Error al eliminar la clave de { $name }
tui-event-provider-connection-ok = Conexión correcta
tui-event-provider-test-failed = La prueba falló
tui-event-provider-test-not-available-in-process = La prueba de proveedores no está disponible en modo en proceso
tui-event-models-load-failed = Error al cargar el catálogo de modelos
tui-event-models-not-available-in-process = La configuración de modelos necesita un daemon en ejecución; no está disponible en modo en proceso
tui-event-model-limits-save-failed = Error al guardar los límites de { $model }
tui-event-model-limits-reset-failed = Error al restablecer los límites de { $model }
tui-event-hand-activation-failed = La activación falló
tui-event-hand-activation-failed-error = La activación falló: { $error }
tui-event-hand-deactivate-failed = Error al desactivar { $instance_id }
tui-event-hand-deactivate-failed-error = La desactivación falló: { $error }
tui-event-hand-invalid-instance-id = ID de instancia no válido: { $error }
tui-event-hand-pause-failed = Error al pausar { $instance_id }
tui-event-hand-pause-failed-error = La pausa falló: { $error }
tui-event-hand-resume-failed = Error al reanudar { $instance_id }
tui-event-hand-resume-failed-error = La reanudación falló: { $error }
tui-event-extension-install-failed = Error al instalar { $id }
tui-event-extension-install-not-supported = La instalación en modo en proceso no es compatible; usa la CLI
tui-event-extension-remove-failed = Error al eliminar { $id }
tui-event-extension-remove-not-supported = La eliminación en modo en proceso no es compatible; usa la CLI
tui-event-extension-reconnect-failed = Error al reconectar { $id }
tui-event-extension-reconnect-not-supported = La reconexión en modo en proceso no es compatible
tui-event-comms-message-sent = Mensaje enviado
tui-event-comms-send-failed = El envío falló
tui-event-comms-send-not-supported-in-process = El envío no es compatible en modo en proceso
tui-event-comms-task-posted = Tarea publicada
tui-event-comms-post-failed = La publicación falló
tui-event-comms-post-not-supported-in-process = La publicación de tareas no es compatible en modo en proceso
tui-event-stream-connection-failed = La conexión falló: { $error }
tui-event-agent-spawn-failed-fallback = Error al crear el agente

# tui/mod.rs
tui-mod-session-deleted = Sesión { $id } eliminada.
tui-mod-saved-key = Clave guardada: { $key }
tui-mod-deleted-key = Clave eliminada: { $key }
tui-mod-skill-installed = Instalada: { $name }
tui-mod-skill-uninstalled = Desinstalada: { $name }
tui-mod-key-saved-for = Clave guardada para { $name }
tui-mod-vault-key-saved = { $key } guardada
tui-mod-vault-key-deleted = { $key } borrada
tui-mod-vault-key-saved-env-override = { $key } guardada, pero el daemon la lee de su propio entorno; el valor guardado no es el que usa
tui-mod-vault-key-deleted-env-override = { $key } borrada de la bóveda, pero el daemon sigue leyéndola de su propio entorno; no se revocó nada
tui-mod-key-deleted-for = Clave eliminada para { $name }
tui-mod-backup-created = Copia de seguridad creada: { $filename }
tui-mod-backup-deleted = Copia de seguridad eliminada: { $filename }
tui-mod-backup-restored = { $files } archivos restaurados desde { $filename }. Reinicia el daemon para aplicar todos los cambios.
tui-mod-backup-restored-with-errors = { $files } archivos restaurados desde { $filename }, { $errors } fallaron. Reinicia el daemon para aplicar todos los cambios.
tui-mod-hand-activated = Activado: { $name }
tui-mod-hand-deactivated = Desactivado: { $id }
tui-mod-hand-paused = Hand pausado
tui-mod-hand-resumed = Hand reanudado
tui-mod-extension-installed = Instalada: { $id }
tui-mod-extension-removed = Eliminada: { $id }
tui-mod-extension-reconnected = { $id } reconectada: { $tools } herramientas
tui-mod-no-agents-running = No hay agentes en ejecución.
tui-mod-agent-killed = Agente "{ $name }" terminado.
tui-mod-failed-kill-agent = Error al terminar el agente "{ $name }".
tui-mod-invalid-manifest = Manifiesto no válido: { $error }
tui-mod-spawn-failed = La creación falló: { $error }
tui-mod-help-help = /help         — mostrar esta ayuda
tui-mod-help-model = /model        — abrir el selector de modelos (Ctrl+M)
tui-mod-help-model-arg = /model <name> — cambiar directamente de modelo
tui-mod-help-status = /status       — conexión e información del agente
tui-mod-help-agents = /agents       — listar los agentes en ejecución
tui-mod-help-clear = /clear        — borrar el historial de chat
tui-mod-help-new = /new          — restablecer la sesión (empezar de cero)
tui-mod-help-kill = /kill         — terminar el agente actual
tui-mod-help-exit = /exit         — finalizar la sesión de chat
tui-mod-status-mode-daemon = Modo: daemon ({ $url })
tui-mod-status-agent = Agente: { $name }
tui-mod-status-mode-inprocess = Modo: en proceso
tui-mod-status-agents-count = Agentes: { $count }
tui-mod-status-mode-disconnected = Modo: desconectado
tui-mod-chat-history-cleared = Historial de chat borrado.
tui-mod-session-reset = Sesión restablecida; empezando de cero.
tui-mod-session-reset-failed = No se pudo restablecer la sesión.
tui-mod-available-hands = Hands disponibles ({ $count }):
tui-mod-active-hands = Hands activos ({ $count }):
tui-mod-hands-info-requires-inprocess = La información de Hands requiere el modo en proceso. Usa la pestaña Hands.
tui-mod-unknown-command = Comando desconocido: { $command }. Escribe /help
tui-mod-error-symbol =  ✘ { $error }
tui-mod-press-ctrl-c-again-to-quit = Pulsa Ctrl+C otra vez para salir
tui-mod-ctrl-c-status-bar = Ctrl+C×2 salir  Tab/Ctrl+←→ cambiar
tui-mod-trigger-deleted = Activador { $id } eliminado.
tui-mod-agent-killed-status = Agente { $id } terminado.
tui-mod-agent-kill-failed = La terminación falló: { $error }
tui-mod-agent-skills-updated = Habilidades actualizadas para el agente { $id }.
tui-mod-agent-mcp-updated = Servidores MCP actualizados para el agente { $id }.
tui-mod-agent-channels-updated = Canales actualizados para el agente { $id }.
tui-mod-agent-workspaces-updated = Carpetas compartidas actualizadas para el agente { $id }.
tui-mod-agent-model-params-updated = Parámetros del modelo actualizados para el agente { $id }.
tui-mod-ready = Listo
tui-mod-setup = Configuración
tui-mod-workflow-created = ¡Flujo de trabajo creado!
tui-mod-trigger-created = ¡Activador creado!
tui-tab-dashboard = Panel
tui-tab-agents = Agentes
tui-tab-chat = Chat
tui-tab-sessions = Sesiones
tui-tab-workflows = Flujos
tui-tab-triggers = Activadores
tui-tab-goals = Objetivos
tui-tab-memory = Memoria
tui-tab-models = Modelos
tui-tab-skills = Habilidades
tui-tab-hands = Hands
tui-tab-extensions = Ext
tui-tab-templates = Plant
tui-tab-peers = Pares
tui-tab-comms = Coms
tui-tab-security = Seg
tui-tab-audit = Auditoría
tui-tab-usage = Uso
tui-tab-settings = Config
tui-tab-logs = Registros
# welcome.rs
tui-welcome-menu-connect = Conectar al daemon
tui-welcome-menu-connect-hint = hablar con agentes en ejecución vía API
tui-welcome-menu-chat = Chat rápido
tui-welcome-menu-chat-hint = iniciar el kernel en local, sin daemon
tui-welcome-menu-setup = Asistente de configuración
tui-welcome-menu-setup-hint = configurar proveedores y canales
tui-welcome-menu-exit = Salir
tui-welcome-menu-exit-hint = salir de LibreFang
tui-welcome-tagline = Sistema operativo de agentes
tui-welcome-ctrl-c-quit = Pulsa Ctrl+C otra vez para salir
tui-welcome-hint-bar = ↑↓ navegar  enter seleccionar  q salir
tui-welcome-checking-daemon = Buscando el daemon…
tui-welcome-agent-count =
    { $count ->
        [one]  • { $count } agente
       *[other]  • { $count } agentes
    }
tui-welcome-daemon-status = Daemon: { $url }
tui-welcome-no-daemon = No hay ningún daemon en ejecución
tui-welcome-provider = Proveedor: { $provider }
tui-welcome-no-api-keys = Sin claves API
tui-welcome-run-hint-prefix =  — ejecuta 
tui-welcome-setup-complete = ¡Configuración completada!

# sessions.rs
tui-sessions-title = Sesiones
tui-sessions-filter = (filtro: "{ $query }")
tui-sessions-count =
    { $count ->
        [one] 1 sesión
       *[other] { $count } sesiones
    }
tui-sessions-header-agent = Agente
tui-sessions-header-id = ID de sesión
tui-sessions-header-msgs = Msjs
tui-sessions-header-created = Creada
tui-sessions-loading = Cargando sesiones…
tui-sessions-empty = Aún no hay sesiones. Inicia un chat para crear una.
tui-sessions-delete-confirm = ¿Eliminar esta sesión? [y] Sí  [cualquier tecla] Cancelar
tui-sessions-hints = ↑↓ Navegar  Enter Abrir  d Eliminar  / Buscar  r Actualizar

# models.rs
tui-models-title = Modelos
tui-models-count =
    { $count ->
        [one] 1 modelo
       *[other] { $count } modelos
    }
tui-models-header-model = Modelo
tui-models-header-provider = Proveedor
tui-models-header-window = Ventana
tui-models-header-catalog = Catálogo
tui-models-header-max-output = Salida máx.
tui-models-loading = Cargando el catálogo de modelos…
tui-models-empty = No hay modelos. Configura un proveedor en Ajustes o añade uno con `librefang model add`.
tui-models-window-unknown = desconocida
tui-models-hints = ↑↓ Navegar  / Filtrar  e Editar límites  d Restablecer al catálogo  r Actualizar
tui-models-status-no-override = Este modelo no tiene valores sobrescritos; no hay nada que restablecer.
tui-models-status-saved = Límites guardados para { $model }
tui-models-status-reset = { $model } revertido al catálogo
tui-models-field-context-window = Ventana de contexto
tui-models-field-max-output = Tokens de salida máximos
tui-models-field-cleared = (borrado; se usa el catálogo)
tui-models-field-catalog-hint = catálogo: { $value }
tui-models-edit-explainer = Se aplica a todos los agentes con este modelo y sobrevive a una sincronización del registro.
tui-models-edit-not-max-tokens = La ventana de contexto es el presupuesto total del modelo, no el límite de salida por respuesta.
tui-models-edit-hints = Tab Cambiar de campo  0-9 Editar  Enter Guardar  Esc Cancelar

# peers.rs
tui-peers-title = Pares
tui-peers-network = Red de pares OFP
tui-peers-count =
    { $count ->
        [one] 1 par
       *[other] { $count } pares
    }
tui-peers-header-node-id = ID de nodo
tui-peers-header-name = Nombre
tui-peers-header-address = Dirección
tui-peers-header-status = Estado
tui-peers-header-agents = Agentes
tui-peers-header-protocol = Protocolo
tui-peers-status-active = Activo
tui-peers-status-offline = Desconectado
tui-peers-status-pending = Pendiente
tui-peers-loading = Descubriendo pares…
tui-peers-empty = No hay pares conectados. Activa la red OFP en config.toml.
tui-peers-hints = ↑↓ Navegar  r Actualizar  (se actualiza automáticamente cada 15s)

# usage.rs
tui-usage-title = Uso
tui-usage-hints = [1] Resumen  [2] Por modelo  [3] Por agente  [r] Actualizar
tui-usage-tab-summary = 1 Resumen
tui-usage-tab-model = 2 Por modelo
tui-usage-tab-agent = 3 Por agente
tui-usage-loading = Cargando datos de uso…
tui-usage-loading-simple = Cargando…
tui-usage-card-input = Tokens de entrada
tui-usage-card-output = Tokens de salida
tui-usage-card-cost = Coste total
tui-usage-card-calls = Llamadas API
tui-usage-header-model = Modelo
tui-usage-header-input = Tokens de entrada
tui-usage-header-output = Tokens de salida
tui-usage-header-cost = Coste
tui-usage-header-calls = Llamadas
tui-usage-header-agent = Agente
tui-usage-header-total-tokens = Tokens totales
tui-usage-header-tool-calls = Llamadas a herramientas
tui-usage-empty = No hay datos de uso. Envía mensajes para ver estadísticas de tokens.

# hands.rs
tui-hands-title = Hands
tui-hands-tab-marketplace = Mercado
tui-hands-tab-active = Activos
tui-hands-loading = Cargando Hands…
tui-hands-loading-active = Cargando Hands activos…
tui-hands-empty-marketplace = No hay definiciones de Hand cargadas.
tui-hands-empty-active = No hay Hands activos. Pulsa [1] para explorar el mercado.
tui-hands-status-ready = Listo
tui-hands-status-setup = Configuración
tui-hands-status-active = Activo
tui-hands-status-paused = Pausado
tui-hands-status-unknown = Desconocido
tui-hands-hints-marketplace =   [↑↓] Navegar  [a/Enter] Activar  [r] Actualizar
tui-hands-hints-active =   [↑↓] Navegar  [p] Pausar/Reanudar  [d] Desactivar  [r] Actualizar
tui-hands-confirm-deactivate =   ¿Desactivar este Hand? [y] Sí  [cualquier tecla] Cancelar
tui-hands-header-name = Nombre
tui-hands-header-category = Categoría
tui-hands-header-status = Estado
tui-hands-header-description = Descripción
tui-hands-header-agent = Agente
tui-hands-header-hand = Hand
tui-hands-header-since = Desde
tui-hands-category-content = Contenido
tui-hands-category-security = Seguridad
tui-hands-category-development = Desarrollo
tui-hands-category-productivity = Productividad

# logs.rs
tui-logs-title = Registros
tui-logs-badge-auto = auto
tui-logs-badge-paused = en pausa
tui-logs-label-level = Nivel
tui-logs-filter-all = Todos
tui-logs-filter-error = Error
tui-logs-filter-warn = Aviso
tui-logs-filter-info = Info
tui-logs-filter-active =   │ filtro: "{ $query }"
tui-logs-entries-count =   │ { $count } entradas
tui-logs-header-timestamp = Marca de tiempo
tui-logs-header-level = Nivel
tui-logs-header-action = Acción
tui-logs-header-agent = Agente
tui-logs-header-detail = Detalle
tui-logs-loading = Cargando registros…
tui-logs-empty = No hay entradas de registro. Inicia el daemon para ver registros.
tui-logs-hints =   [↑↓] Navegar  [f] Filtrar nivel  [/] Buscar  [a] Actualización automática  [r] Actualizar

# security.rs
tui-security-title = Seguridad
tui-security-active-features =   { $active }/{ $total } funciones activas
tui-security-sections-sub =   │  Núcleo · Configurable · Monitorización
tui-security-section-core = Seguridad del núcleo
tui-security-section-configurable = Configurable
tui-security-section-monitoring = Monitorización
tui-security-header-feature = Función
tui-security-header-status = Estado
tui-security-header-description = Descripción
tui-security-status-active = Activa
tui-security-status-inactive = Inactiva
tui-security-verifying = Verificando la cadena de auditoría…
tui-security-verify-prompt = Pulsa [v] para verificar la integridad de la cadena de auditoría
tui-security-verify-success = Cadena de auditoría verificada
tui-security-verify-failed = La verificación de la cadena de auditoría falló
tui-security-hints =   [↑↓] Desplazar  [v] Verificar cadena  [r] Actualizar
tui-security-feat-path-traversal-name = Prevención de salto de ruta
tui-security-feat-path-traversal-desc = safe_resolve_path bloquea ataques ../
tui-security-feat-ssrf-name = Protección SSRF
tui-security-feat-ssrf-desc = Bloquea IP privadas y endpoints de metadatos en las peticiones HTTP
tui-security-feat-subprocess-name = Aislamiento de subprocesos
tui-security-feat-subprocess-desc = env_clear() + variables selectivas en los procesos hijos
tui-security-feat-wasm-name = Medición doble WASM
tui-security-feat-wasm-desc = Fuel + interrupción por epoch con hilo watchdog
tui-security-feat-capability-name = Herencia de capacidades
tui-security-feat-capability-desc = validate_capability_inheritance evita la escalada de privilegios
tui-security-feat-secret-name = Borrado de secretos
tui-security-feat-secret-desc = Zeroizing<String> borra las claves API de la memoria automáticamente
tui-security-feat-ed25519-name = Firma de manifiestos Ed25519
tui-security-feat-ed25519-desc = Manifiestos de agente firmados con verificación Ed25519
tui-security-feat-taint-name = Seguimiento de contaminación
tui-security-feat-taint-desc = Seguimiento del flujo de información entre límites de herramientas
tui-security-feat-ofp-name = Autenticación OFP
tui-security-feat-ofp-desc = Autenticación mutua HMAC-SHA256 con nonce
tui-security-feat-rbac-name = RBAC multiusuario
tui-security-feat-rbac-desc = Control de acceso basado en roles con jerarquía de usuarios
tui-security-feat-rate-name = Limitación de tasa
tui-security-feat-rate-desc = Limitador GCRA con tokens según el coste
tui-security-feat-headers-name = Cabeceras de seguridad
tui-security-feat-headers-desc = Middleware CSP, X-Frame-Options y HSTS
tui-security-feat-merkle-name = Registro de auditoría Merkle
tui-security-feat-merkle-desc = Registro de auditoría en cadena de hashes con detección de manipulación
tui-security-feat-heartbeat-name = Monitor de latidos
tui-security-feat-heartbeat-desc = Comprobaciones de estado en segundo plano con límites de reinicio
tui-security-feat-prompt-name = Escáner de inyección de prompts
tui-security-feat-prompt-desc = Detecta intentos de anulación y exfiltración de datos

# templates.rs
tui-templates-title = Tipos de agente
tui-templates-cat-all = Todos
tui-templates-cat-general = General
tui-templates-cat-development = Desarrollo
tui-templates-cat-research = Investigación
tui-templates-cat-writing = Redacción
tui-templates-cat-business = Negocio
tui-templates-cat-custom = Personalizado
tui-templates-header-template = Tipo de agente
tui-templates-header-category = Categoría
tui-templates-header-provider-model = Proveedor/Modelo
tui-templates-header-description = Descripción
tui-templates-loading = Cargando tipos de agente…
tui-templates-empty = No hay tipos de agente disponibles.
tui-templates-detail-provider =   Proveedor: { $provider }/{ $model }  
tui-templates-hints =   [↑↓] Navegar  [Enter] Crear agente  [f] Filtrar categoría  [p] Promover  [r] Actualizar  [R] Restaurar  [v] Historial
tui-templates-promote-only-custom = Solo los tipos de agente personalizados pueden promoverse al registro
tui-templates-promoting = Promoviendo '{ $name }' al registro…
tui-templates-promoted = PR del registro abierto: { $url }
tui-templates-promote-failed = No se pudo promover '{ $name }': { $error }
tui-templates-provider-not-configured = Proveedor '{ $provider }' no configurado. Define primero la clave API en Ajustes.
tui-templates-manifest-unavailable = No se pudo cargar el manifiesto de '{ $name }'. Revisa el registro del daemon.
tui-templates-restore-invalid-name = Nombre de plantilla no válido
tui-templates-restore-success = Restaurado desde el registro
tui-templates-restore-daemon-only = La restauración desde el registro requiere el modo daemon
tui-templates-restore-custom-only = La restauración desde el registro solo está disponible para plantillas personalizadas

tui-templates-confirm-restore = ¿Sobrescribir este tipo de agente con la copia del registro? [y] Sí  [cualquier tecla] Cancelar
tui-templates-confirm-restore-version = ¿Revertir este tipo de agente a la versión seleccionada? [y] Sí  [cualquier tecla] Cancelar
tui-templates-restore-ok = ✓ { $name }: { $message }
tui-templates-restore-fail = ✗ { $name }: { $message }
tui-templates-restoring = Restaurando { $name } desde el registro…
tui-templates-restore-failed = No se pudo restaurar desde el registro
tui-templates-history-failed = No se pudo cargar el historial de versiones
tui-templates-history-loading = Cargando el historial de versiones de { $name }…
tui-templates-history-daemon-only = El historial de versiones requiere el modo daemon
tui-templates-history-title = Historial de versiones de { $name }
tui-templates-history-col-id = ID de versión
tui-templates-history-col-created = Creada
tui-templates-history-col-source = Origen
tui-templates-history-error = No se pudo cargar el historial de versiones: { $detail }
tui-templates-history-shape = La respuesta del historial de versiones del daemon tiene una forma inesperada
tui-templates-history-empty = No hay historial de versiones disponible
tui-templates-history-manifest-only = El historial de versiones solo está disponible para plantillas personalizadas
tui-templates-source-toml = Edición directa del TOML
tui-templates-source-create = Creada
tui-templates-source-dashboard = Edición desde el panel
tui-templates-source-registry-restore = Restaurada desde el registro
tui-templates-source-pre-registry-restore = Instantánea previa a la restauración
tui-templates-source-restore = Restaurada
tui-templates-source-agent-tool = Herramienta de agente
tui-templates-source-unknown = Origen desconocido
tui-templates-version-restoring = Restaurando { $name } a la versión seleccionada…
tui-templates-version-restore-failed = No se pudo restaurar la versión
tui-templates-history-hints = [Esc] Volver  [↑↓] Navegar  [Enter] Restaurar
tui-templates-name-general-assistant = Asistente general
tui-templates-desc-general-assistant = Asistente de IA versátil para tareas cotidianas
tui-templates-name-code-helper = Ayudante de código
tui-templates-desc-code-helper = Asistente de programación con revisión de código y depuración
tui-templates-name-researcher = Investigador
tui-templates-desc-researcher = Investigación y análisis en profundidad con búsqueda web
tui-templates-name-writer = Redactor
tui-templates-desc-writer = Asistente de redacción creativa y técnica
tui-templates-name-data-analyst = Analista de datos
tui-templates-desc-data-analyst = Análisis de datos, visualización y consultas SQL
tui-templates-name-devops-engineer = Ingeniero DevOps
tui-templates-desc-devops-engineer = Infraestructura, CI/CD y asistencia en despliegues
tui-templates-name-customer-support = Atención al cliente
tui-templates-desc-customer-support = Agente profesional de atención al cliente
tui-templates-name-tutor = Tutor
tui-templates-desc-tutor = Asistente educativo paciente para aprender cualquier materia
tui-templates-name-api-designer = Diseñador de API
tui-templates-desc-api-designer = Diseño y documentación de API REST/GraphQL
tui-templates-name-meeting-notes = Notas de reunión
tui-templates-desc-meeting-notes = Transcripción, resumen y tareas de reuniones

# audit.rs
tui-audit-title = Registro de auditoría
tui-audit-filter-all = Todo
tui-audit-filter-spawn = Agente creado
tui-audit-filter-kill = Agente terminado
tui-audit-filter-tool = Herramienta usada
tui-audit-filter-network = Red
tui-audit-filter-shell = Ejecución de shell
tui-audit-action-spawn = Agente creado
tui-audit-action-kill = Agente terminado
tui-audit-action-tool = Herramienta usada
tui-audit-action-network = Acceso a la red
tui-audit-action-shell = Ejecución de shell
tui-audit-action-denied = Acceso denegado
tui-audit-action-config = Configuración modificada
tui-audit-label-filter = Filtro:
tui-audit-entries-count = { $count } entradas
tui-audit-header-timestamp = Marca de tiempo
tui-audit-header-action = Acción
tui-audit-header-agent = Agente
tui-audit-header-hash = Hash
tui-audit-header-detail = Detalle
tui-audit-loading = Cargando el registro de auditoría…
tui-audit-empty = Aún no hay entradas de auditoría. Las acciones de los agentes aparecerán aquí.
tui-audit-chain-unverified = Cadena: sin verificar
tui-audit-chain-verified = Cadena: verificada
tui-audit-chain-failed = Cadena: la verificación falló
tui-audit-hints =   [↑↓] Navegar  [f] Filtrar  [v] Verificar cadena  [r] Actualizar

# dashboard.rs
tui-dashboard-title = Panel
tui-dashboard-hints =   [r] Actualizar  [a] Agentes  [↑↓] Desplazar  [PgUp/PgDn] Desplazamiento rápido
tui-dashboard-dreams-title = DREAMS
tui-dashboard-auto-dream-enabled = sueño automático activado
tui-dashboard-auto-dream-disabled = sueño automático desactivado
tui-dashboard-dream-details = fase={ $phase }  herramientas={ $tools }  mems={ $mems }
tui-dashboard-stat-agents = AGENTES
tui-dashboard-stat-uptime = TIEMPO ACTIVO
tui-dashboard-stat-provider = PROVEEDOR
tui-dashboard-stat-model = MODELO
tui-dashboard-audit-time = Hora
tui-dashboard-audit-agent = Agente
tui-dashboard-audit-action = Acción
tui-dashboard-audit-detail = Detalle
tui-dashboard-loading = Cargando…
tui-dashboard-no-audit = Aún no hay entradas de auditoría.

# comms.rs
tui-comms-title = Comunicaciones
tui-comms-tab-topology = Topología ({ $agents } agentes, { $edges } aristas)
tui-comms-tab-events = Eventos ({ $count })
tui-comms-hints =   [s] enviar  [t] tarea  [r] actualizar  [Tab] foco  [↑↓] desplazar
tui-comms-loading = Cargando la topología…
tui-comms-empty = No hay agentes en ejecución. Inicia agentes para ver la comunicación.
tui-comms-events-empty = Aún no hay eventos entre agentes. La actividad aparecerá aquí.
tui-comms-modal-send-title =  Enviar mensaje 
tui-comms-modal-send-from = De (ID del agente):
tui-comms-modal-send-to = A (ID del agente):
tui-comms-modal-send-msg = Mensaje:
tui-comms-modal-send-hints = [Tab] campo  [Enter] enviar  [Esc] cancelar
tui-comms-modal-task-title =  Publicar tarea 
tui-comms-modal-task-title-field = Título:
tui-comms-modal-task-desc = Descripción:
tui-comms-modal-task-assign = Asignar a (ID del agente, opcional):
tui-comms-modal-task-hints = [Tab] campo  [Enter] publicar  [Esc] cancelar

# settings.rs
tui-settings-title = Ajustes
tui-settings-hints-input =   [Enter] Guardar  [Esc] Cancelar
tui-settings-hints-providers =   [↑↓] Navegar  [e] Definir clave  [d] Eliminar clave  [t] Probar  [r] Actualizar
tui-settings-hints-models =   [↑↓] Navegar  [r] Actualizar
tui-settings-hints-tools =   [↑↓] Navegar  [r] Actualizar
tui-settings-tab-providers = 1 Proveedores
tui-settings-tab-models = 2 Modelos
tui-settings-tab-tools = 3 Herramientas
tui-settings-providers-header-provider = Proveedor
tui-settings-providers-header-status = Estado
tui-settings-providers-header-env = Variable de entorno
tui-settings-providers-loading = Cargando proveedores…
tui-settings-providers-empty = No hay proveedores configurados. Ejecuta `librefang init` para configurar.
tui-settings-providers-status-online = En línea ({ $ms }ms)
tui-settings-providers-status-offline = Sin conexión
tui-settings-providers-status-local = Local
tui-settings-providers-status-configured = Configurado
tui-settings-providers-status-notset = Sin definir
tui-settings-providers-input-prompt = Introduce la clave API de { $provider }: 
tui-settings-providers-latency = Latencia: { $ms }ms
tui-settings-models-header-id = ID del modelo
tui-settings-models-header-provider = Proveedor
tui-settings-models-header-tier = Nivel
tui-settings-models-header-context = Contexto
tui-settings-models-header-cost = Coste (ent/sal por 1M)
tui-settings-models-loading = Cargando modelos…
tui-settings-models-empty = No hay modelos disponibles.
tui-settings-tools-header-name = Nombre de la herramienta
tui-settings-tools-header-desc = Descripción
tui-settings-tools-empty = No hay herramientas disponibles.
tui-settings-tab-backups = 4 Copias de seguridad
tui-settings-hints-backups =   [↑↓] Navegar  [c] Crear  [Enter] Restaurar  [d] Eliminar  [r] Actualizar
tui-settings-hints-restore =   [↑↓] Mover  [Espacio] Alternar  [Enter] Restaurar  [Esc] Cancelar
tui-settings-backups-header-filename = Archivo
tui-settings-backups-header-size = Tamaño
tui-settings-backups-header-created = Creada
tui-settings-backups-header-components = Componentes
tui-settings-backups-loading = Cargando copias de seguridad…
tui-settings-backups-empty = Aún no hay copias de seguridad. Pulsa [c] para crear una.
tui-settings-backups-creating = Creando copia de seguridad…
tui-settings-backups-delete-confirm =   ¿Eliminar esta copia de seguridad? [y] Sí  [cualquier tecla] Cancelar
tui-settings-tab-vault = 7 Bóveda
tui-settings-hints-vault =   [↑↓] Navegar  [e] Definir valor  [d] Borrar  [r] Actualizar
tui-settings-vault-header-key = CLAVE
tui-settings-vault-header-status = ESTADO
tui-settings-vault-loading = Cargando claves de la bóveda...
tui-settings-vault-empty = Este daemon no expone ninguna clave de bóveda escribible
tui-settings-vault-status-set = Definida
tui-settings-vault-status-notset = Sin definir
tui-settings-vault-status-env = Reemplazada por el entorno
tui-settings-vault-input-prompt = Introduce el nuevo valor de { $key }
tui-settings-vault-note = Los valores guardados nunca se vuelven a mostrar. [e] reemplaza, [d] borra.
tui-settings-vault-note-env-override = { $keys } proviene del entorno del propio daemon, que tiene prioridad sobre la bóveda. [e] y [d] cambian la copia guardada, no la credencial que usa el daemon.
tui-settings-vault-delete-confirm =   ¿Borrar esta clave de la bóveda? [y] Sí  [cualquier tecla] Cancelar
tui-settings-backups-restore-title = Restaurar { $filename }
tui-settings-backups-restore-warning = La restauración sobrescribe el estado actual. Reinicia el daemon después.
tui-settings-backups-restore-keep-config = Conservar el config.toml de esta máquina (modo clon)
tui-settings-backups-restore-all = Todo lo que hay en el archivo está seleccionado.
tui-settings-backups-restore-none = Selecciona al menos un componente, o pulsa [Esc] para cancelar.
tui-settings-tab-auxiliary = 5 Auxiliares
tui-settings-hints-auxiliary =   [↑↓] Navegar  [Enter] Editar cadena  [r] Actualizar  [Esc] Cancelar
tui-settings-auxiliary-header-task = Tarea
tui-settings-auxiliary-header-chain = Cadena de respaldo
tui-settings-auxiliary-loading = Cargando la configuración auxiliar…
tui-settings-auxiliary-empty = No hay tareas auxiliares configuradas.
tui-settings-auxiliary-not-configured = (sin configurar)
tui-settings-auxiliary-editing = Editando la cadena de { $task } (separada por comas provider:model):
# chat.rs
tui-chat-input-staged =   ({ $count } en espera)
tui-chat-hints-modelpicker =     [↑↓] Navegar  [Enter] Seleccionar  [Esc] Cerrar  [escribe] Filtrar
tui-chat-hints-streaming =     [Enter] Encolar  [↑↓] Desplazar  [Esc] Detener
tui-chat-hints-history =     [Enter] Enviar  [↑↓] Historial  [PgUp/PgDn] Desplazar  [Esc] Volver
tui-chat-hints-normal =     [Enter] Enviar  [Ctrl+M] Modelos  [↑↓] Historial  [PgUp/PgDn] Desplazar  [Esc] Volver
tui-chat-modelpicker-title =  Cambiar modelo 
tui-chat-modelpicker-empty = Ningún modelo coincide
tui-chat-welcome-ready = Listo para chatear
tui-chat-welcome-suggest =   Prueba a preguntar:
tui-chat-welcome-q1 = "Explícame este código"
tui-chat-welcome-q2 = "Escribe un test unitario para..."
tui-chat-welcome-q3 = "¿Qué hace esta función?"
tui-chat-welcome-footer =   Escribe /help para los comandos  •  Ctrl+M para cambiar de modelo
tui-chat-tokens-estimated =   ~{ $count } tokens
tui-chat-tokens-detail =   [tokens: { $in } ent / { $out } sal{ $cost }]
tui-chat-tool-input = entrada: 
tui-chat-tool-error = error: 
tui-chat-tool-result = resultado: 
tui-chat-tool-running = en ejecución…
tui-chat-thinking = pensando…
tui-chat-mode-daemon = daemon
tui-chat-mode-inprocess = en proceso

# free_provider_guide.rs
tui-guide-hint-groq = nivel gratuito, inferencia muy rápida
tui-guide-hint-gemini = nivel gratuito, cuota generosa (cuenta de Google)
tui-guide-hint-deepseek = 5M de tokens gratis para cuentas nuevas
tui-guide-label-apikey =  Clave API 
tui-guide-warn-env = .env: { $error }

# init_wizard.rs
tui-init-welcome-tagline = Sistema operativo de agentes
tui-init-welcome-sec1 = Ejecución en sandbox, aislamiento WASM, protección SSRF
tui-init-welcome-sec2 = Manifiestos firmados, registro de auditoría, seguimiento de contaminación
tui-init-welcome-sec3 = RBAC, comprobaciones de capacidades, borrado de secretos
tui-init-welcome-sec4 = Claves API nunca registradas, permisos de archivo 0600
tui-init-welcome-resp1 = Los agentes pueden ejecutar código, acceder a la red y actuar
tui-init-welcome-resp2 = en tu nombre.
tui-init-welcome-resp-warn = Tú eres responsable de lo que hagan.
tui-init-welcome-hints =   [Enter] Lo entiendo    [Esc] Cancelar
tui-init-migrate-checking =   Buscando instalaciones existentes...
tui-init-migrate-openfang-detected =   Instalación de OpenFang detectada
tui-init-migrate-openclaw-detected =   Instalación de OpenClaw detectada
tui-init-migrate-openfang-summary = Configuración y datos de OpenFang
tui-init-migrate-openclaw-agents = { $count } agentes ({ $names })
tui-init-migrate-openclaw-no-agents = Sin agentes
tui-init-migrate-openclaw-channels = { $count } canales ({ $names })
tui-init-migrate-openclaw-no-channels = Sin canales
tui-init-migrate-openclaw-skills = { $count } habilidades
tui-init-migrate-openclaw-no-skills = Sin habilidades
tui-init-migrate-openclaw-memory = Archivos de memoria
tui-init-migrate-openclaw-no-memory = Sin archivos de memoria
tui-init-migrate-openclaw-config = Configuración
tui-init-migrate-opt-yes = Sí
tui-init-migrate-opt-yes-desc = migrar ajustes y datos
tui-init-migrate-opt-no = No
tui-init-migrate-opt-no-desc = empezar de cero
tui-init-migrate-hints =   [↑↓] Navegar  [Enter] Seleccionar  [Esc] Omitir
tui-init-migrate-running-openfang =  Migrando desde OpenFang...
tui-init-migrate-running-openclaw =  Migrando desde OpenClaw...
tui-init-migrate-done-failed = La migración falló: { $error }
tui-init-migrate-done-config = Configuración migrada
tui-init-migrate-done-agents = { $count } agentes importados ({ $names })
tui-init-migrate-done-channels = { $count } canales ({ $names })
tui-init-migrate-done-memory = Archivos de memoria copiados
tui-init-migrate-done-skills = { $count } habilidades importadas
tui-init-migrate-done-sessions = { $count } sesiones importadas
tui-init-migrate-done-skipped = { $name} omitido ({ $reason })
tui-init-migrate-done-summary =   { $imported } importados, { $skipped } omitidos, { $warnings } avisos
tui-init-migrate-done-continue =   [Enter] Continuar  
tui-init-migrate-done-autoadvancing = (avanzando automáticamente...)
tui-init-provider-prompt =   Elige tu proveedor LLM:
tui-init-provider-cli-detected = CLI detectada
tui-init-provider-no-key-needed = no hace falta clave API
tui-init-provider-local-no-key = local, no hace falta clave
tui-init-provider-requires-with-hint = requiere { $env_var } ({ $hint })
tui-init-provider-requires = requiere { $env_var }
tui-init-provider-hints =   [↑↓/jk] Navegar  [Enter] Seleccionar  [Esc] Cancelar
tui-init-hint-freetier = nivel gratuito
tui-init-hint-cheap = barato
tui-init-hint-fast = inferencia rápida
tui-init-hint-pat = mediante PAT
tui-init-hint-nokey = sin clave API
tui-init-hint-local = local
tui-init-apikey-prompt =   Introduce tu clave API de { $provider }:
tui-init-apikey-env-hint =     O define la variable de entorno { $env_var }
tui-init-apikey-testing =  Probando la clave API...
tui-init-apikey-verified = Clave API verificada
tui-init-apikey-saved =     Guardada en ~/.librefang/.env
tui-init-apikey-verify-failed = No se pudo verificar (puede funcionar igualmente)
tui-init-apikey-save-failed = Verificada, pero el guardado en .env falló
tui-init-apikey-save-failed-hints =     [Enter] reintentar guardado  ·  [Esc] editar clave  (la clave ya está verificada, nada en disco)
tui-init-apikey-hints =   [Enter] Confirmar  [Esc] Volver
tui-init-model-prompt =   Elige el modelo predeterminado para { $provider }:
tui-init-model-hints =   [↑↓/jk] Navegar  [Enter] Seleccionar  [Esc] Volver    * = predeterminado
tui-init-routing-title =   Enrutamiento inteligente de modelos
tui-init-routing-desc1 =   Elige automáticamente el modelo adecuado según la complejidad de la tarea.
tui-init-routing-desc2 =   Las tareas simples usan modelos baratos/rápidos; las complejas,
tui-init-routing-desc3 =   modelos de frontera. Ahorra coste sin sacrificar calidad.
tui-init-routing-opt-yes = Sí
tui-init-routing-opt-yes-desc = elegir 3 modelos (rápido / equilibrado / frontera)
tui-init-routing-opt-no = No
tui-init-routing-opt-no-desc = usar un solo modelo para todo
tui-init-routing-hints =   [↑↓] Navegar  [Enter] Seleccionar  [Esc] Volver
tui-init-routing-pick-hints =   [↑↓/jk] Navegar  [Enter] Seleccionar  [Esc] Volver
tui-init-routing-tier-fast = Rápido
tui-init-routing-tier-balanced = Equilibrado
tui-init-routing-tier-frontier = Frontera
tui-init-routing-tier-fast-desc = consultas rápidas, saludos, preguntas simples
tui-init-routing-tier-balanced-desc = conversación estándar, tareas generales
tui-init-routing-tier-frontier-desc = razonamiento de varios pasos, generación de código
tui-init-complete-success-daemon = Configuración completada; daemon en ejecución
tui-init-complete-success = ¡Configuración completada!
tui-init-complete-label-provider =   Proveedor:   
tui-init-complete-label-model =   Modelo:      
tui-init-complete-label-daemon =   Daemon:      
tui-init-complete-daemon-running = en ejecución en { $url }
tui-init-complete-daemon-not-running = no está en ejecución
tui-init-complete-daemon-pending = pendiente
tui-init-complete-question =   ¿Cómo quieres usar LibreFang?
tui-init-complete-desktop-desc-installed = ventana nativa con bandeja del sistema
tui-init-complete-desktop-desc-not-installed = no instalada
tui-init-complete-opt-desktop = App de escritorio
tui-init-complete-opt-desktop-badge = (recomendado)
tui-init-complete-opt-dashboard = Panel web
tui-init-complete-opt-dashboard-desc = se abre en tu navegador predeterminado
tui-init-complete-opt-chat = Chat en la terminal
tui-init-complete-opt-chat-desc = chat interactivo aquí mismo
tui-init-complete-hints =   [↑↓/jk] Navegar  [Enter] Lanzar  [1/2/3] Selección rápida
tui-init-step-label = { $current } de { $total }
tui-init-complete-err-no-provider = No se seleccionó ningún proveedor
tui-init-complete-err-home-dir = No se pudo determinar el directorio personal
tui-init-complete-err-write-config = Error al escribir la configuración: { $error }
tui-init-complete-err-daemon-failed = El daemon falló: { $error }
tui-init-routing-pick-prefix = Elige el modelo
tui-init-routing-pick-suffix = ({ $step }/3):
tui-init-complete-setup-prefix = Configuración completada: 

# agents.rs
tui-agents-tool-file-read-desc = Leer archivos
tui-agents-tool-file-write-desc = Escribir archivos
tui-agents-tool-file-list-desc = Listar el contenido de un directorio
tui-agents-tool-memory-store-desc = Guardar datos en la memoria del agente
tui-agents-tool-memory-recall-desc = Recuperar datos de la memoria
tui-agents-tool-memory-list-desc = Listar todas las claves de memoria guardadas
tui-agents-tool-web-fetch-desc = Obtener páginas web
tui-agents-tool-shell-exec-desc = Ejecutar comandos de shell
tui-agents-tool-agent-send-desc = Enviar mensajes a otros agentes
tui-agents-tool-agent-list-desc = Listar los agentes en ejecución

tui-agents-title-create-method = Crear agente
tui-agents-title-templates = Plantillas
tui-agents-title-custom-name = Personalizado: nombre
tui-agents-title-custom-desc = Personalizado: descripción
tui-agents-title-custom-prompt = Personalizado: prompt del sistema
tui-agents-detail-tokens = Huella de tokens  ($ para actualizar)
tui-agents-detail-tokens-injected = inyectados
tui-agents-token-usage-failed = No se pudo cargar la huella de tokens
tui-agents-title-custom-tools = Personalizado: herramientas
tui-agents-title-custom-skills = Personalizado: habilidades
tui-agents-workspaces-help = a añadir · d eliminar · Enter editar · Tab/Shift+Tab campo · s guardar · Esc volver
tui-agents-workspaces-empty = Aún no hay carpetas compartidas. Pulsa a para añadir una.
tui-agents-workspaces-loading = Cargando carpetas compartidas…
tui-agents-workspaces-row-dropped = Una fila con el nombre o la ruta vacíos no se guardará. Pulsa s otra vez para guardar el resto.
tui-agents-workspaces-row-invalid = La carpeta compartida "{ $name }" no se guardó: el nombre solo admite letras, dígitos, "-" y "_", y la ruta debe ser relativa y sin "..".
tui-agents-workspaces-mode-invalid = "{ $value }" no es un modo de carpeta; se conservó el anterior. Usa "readonly" o "readwrite".
tui-event-workspaces-update-failed = Error al actualizar las carpetas compartidas
tui-event-workspaces-manifest-read-failed = Error al leer el manifiesto del agente
tui-event-workspaces-duplicate-name = Nombre de carpeta duplicado { $name }; cada carpeta compartida necesita un nombre único
tui-event-workspaces-not-available-in-process = La edición de carpetas compartidas necesita un daemon en ejecución.
tui-agents-title-custom-mcp = Personalizado: servidores MCP
tui-agents-title-edit-channels = Editar: canales
tui-agents-title-spawning = Creando...
tui-agents-title-screen = Agentes
tui-agents-title-detail = Detalle del agente

tui-agents-prompt-create-method =   ¿Cómo quieres crear tu agente?
tui-agents-prompt-name = Nombre del agente:
tui-agents-prompt-desc = Descripción:
tui-agents-prompt-prompt = Prompt del sistema:
tui-agents-prompt-tools =   Selecciona herramientas (Espacio para alternar):
tui-agents-prompt-skills =   Selecciona habilidades (ninguna marcada = todas):
tui-agents-prompt-mcp =   Selecciona servidores MCP (ninguno marcado = todos):
tui-agents-prompt-edit-skills =   Espacio para alternar, Enter para guardar (ninguna marcada = todas):
tui-agents-prompt-edit-channels =   Espacio para alternar, Enter para guardar (ninguno marcado = todos los canales):
tui-agents-prompt-spawning =   Creando el agente...
tui-agents-label-no-agent-selected = No hay ningún agente seleccionado.
tui-agents-label-none-available = (ninguno disponible)

tui-agents-opt-templates =   Elegir entre plantillas
tui-agents-opt-templates-desc =   (agentes predefinidos)
tui-agents-opt-custom =   Crear un agente personalizado
tui-agents-opt-custom-desc =   (nombre, herramientas, prompt)

tui-agents-header-state = Estado
tui-agents-header-name = Nombre
tui-agents-header-model = Modelo
tui-agents-header-id = ID
tui-agents-opt-create-new = Crear un agente nuevo

tui-agents-hints-filter =   [Escribe] Filtrar  [Enter] Aceptar  [Esc] Cancelar búsqueda
tui-agents-hints-list =   [↑↓] Navegar  [Enter] Detalle  [/] Buscar  [Esc] Volver
tui-agents-title-manifest-history = Historial de configuración
tui-agents-label-manifest-history-loading = Cargando el historial de configuración...
tui-agents-label-manifest-history-empty = Aún no hay cambios de configuración registrados para este agente.
tui-agents-hints-manifest-history =   [↑↓/jk] Explorar versiones  [PgUp/PgDn] Desplazar  [Esc] Volver
tui-agents-manifest-source-model = Cambio de modelo
tui-agents-manifest-source-skills = Cambio de habilidades
tui-agents-manifest-source-workspaces = Cambio de carpetas compartidas
tui-agents-manifest-source-channels = Cambio de canales
tui-agents-manifest-source-routing = Cambio de enrutamiento
tui-agents-manifest-source-schedule = Cambio de programación
tui-agents-manifest-source-tool-filters = Cambio de filtros de herramientas
tui-agents-manifest-source-mcp-servers = Cambio de servidores MCP
tui-agents-manifest-source-api = Edición por API
tui-agents-manifest-source-provisioning = Aprovisionado
tui-agents-manifest-source-restore = Restaurado
tui-agents-manifest-source-suspend = Suspendido
tui-agents-manifest-source-resume = Reanudado
tui-agents-manifest-source-suspend-persist-failed = Falló la escritura de la suspensión
tui-agents-manifest-source-resume-persist-failed = Falló la escritura de la reanudación
tui-agents-manifest-source-unknown = Origen desconocido
tui-agents-hints-detail =   [s] Editar habilidades  [m] Editar MCP  [n] Editar canales  [w] Carpetas compartidas  [p] Parámetros del modelo  [$] Tokens  [h] Historial de configuración  [c] Chat  [k] Terminar  [Esc] Volver
tui-agents-title-model-params = Parámetros del modelo
tui-agents-prompt-model-params = Los ajustes propios de este agente tienen prioridad sobre los del modelo. `inherit` significa que se aplica el ajuste del modelo.
tui-agents-hints-model-params =   [←→] Cambiar  [i] Heredar  [e] Valor personalizado  [Enter] Guardar  [Esc] Cancelar
tui-agents-hints-model-params-custom =   Escribe un valor  [Enter] Confirmar  [Esc] Cancelar  (vacío = heredar)
tui-agents-hints-navigate =     [↑↓] Navegar  [Enter] Seleccionar  [Esc] Volver
tui-agents-hints-input =     [Enter] Siguiente  [Esc] Volver
tui-agents-hints-tools =     [↑↓] Navegar  [Espacio] Alternar  [Enter] Crear  [Esc] Volver
tui-agents-hints-skills =     [↑↓] Navegar  [Espacio] Alternar  [Enter] Siguiente  [Esc] Volver
tui-agents-hints-mcp =     [↑↓] Navegar  [Espacio] Alternar  [Enter] Crear  [Esc] Volver
tui-agents-hints-save =     [↑↓] Navegar  [Espacio] Alternar  [Enter] Guardar  [Esc] Cancelar


tui-agents-placeholder-name = mi-agente
tui-agents-placeholder-desc = Un agente personalizado
tui-agents-placeholder-prompt = Eres un agente servicial.
tui-agents-label-placeholder =     marcador: { $placeholder }

tui-agents-detail-id =   ID:       
tui-agents-detail-name =   Nombre:   
tui-agents-detail-state =   Estado:   
tui-agents-detail-provider =   Proveedor: 
tui-agents-detail-model =   Modelo:    
tui-agents-detail-created =   Creado:  
tui-agents-detail-active =   Activo:   
tui-agents-detail-tags =   Etiquetas: 
tui-agents-detail-caps =   Caps:     
tui-agents-detail-parent =   Padre:   
tui-agents-detail-children =   Hijos: 
tui-agents-detail-skills =   Habilidades: 
tui-agents-detail-mcp =   MCP:      
tui-agents-detail-channels =   Canales: 
tui-agents-detail-all-skills = [Todas las habilidades]
tui-agents-detail-all-servers = [Todos los servidores]
tui-agents-detail-all-channels = [Todos los canales]
tui-agents-detail-none = [Ninguno]
tui-agents-default-desc = Un agente { $name } personalizado
tui-agents-default-prompt = Eres { $name }, un agente servicial.

# --- Workflows screen ---
tui-workflows-title-screen = Flujos de trabajo
tui-workflows-header-id = ID
tui-workflows-header-name = Nombre
tui-workflows-header-steps = Pasos
tui-workflows-header-created = Creado
tui-workflows-loading = Cargando flujos de trabajo...
tui-workflows-empty-state = No hay flujos de trabajo definidos. Crea uno con [n].
tui-workflows-create-new-option =   + Crear un flujo de trabajo
tui-workflows-hints-list =   [↑↓] Navegar  [Enter] Ver ejecuciones  [x] Ejecutar  [n] Nuevo  [r] Actualizar
tui-workflows-title-runs = Ejecuciones de: { $name }
tui-workflows-header-run-id = ID de ejecución
tui-workflows-header-state = Estado
tui-workflows-header-progress = Progreso
tui-workflows-header-duration = Duración
tui-workflows-runs-empty = Aún no hay ejecuciones. Pulsa [x] desde la lista para ejecutar.
tui-workflows-badge-auto = auto
tui-workflows-badge-paused = en pausa
tui-workflows-hints-runs =   [↑↓] Navegar  [a] Actualización automática  [r] Actualizar  [Esc] Volver
tui-workflows-title-create = Crear un flujo de trabajo
tui-workflows-create-step =   Paso { $current } de { $total }
tui-workflows-label-name = Nombre del flujo de trabajo:
tui-workflows-placeholder-name = mi-flujo
tui-workflows-label-desc = Descripción:
tui-workflows-placeholder-desc = Qué hace este flujo de trabajo
tui-workflows-hint-steps = El origen del agente determina qué nombra el valor: el ID de un agente en ejecución, el nombre de un agente o un tipo de agente, que nombra una plantilla y se crea cuando no hay ninguno de ese tipo en ejecución. La sesión "Nueva en cada ejecución" da al paso una sesión nueva en cada ejecución.
tui-workflows-label-steps = Pasos:
tui-workflows-steps-empty = Aún no hay pasos. Pulsa [a] para añadir uno.
tui-workflows-step-field-name = Nombre del paso
tui-workflows-step-field-source = Origen del agente
tui-workflows-step-field-prompt = Prompt
tui-workflows-step-field-session = Sesión
tui-workflows-source-id = ID del agente
tui-workflows-source-name = Nombre del agente
tui-workflows-source-type = Tipo de agente
tui-workflows-session-persistent = Persistente
tui-workflows-session-new = Nueva en cada ejecución
tui-workflows-placeholder-agent-id = UUID de un agente en ejecución
tui-workflows-placeholder-agent-name = nombre de un agente existente
tui-workflows-placeholder-agent-type = nombre de plantilla, se crea si no hay ninguno en ejecución
tui-workflows-placeholder-prompt = qué pide este paso; {"{{input}}"} es la entrada del flujo de trabajo
tui-workflows-steps-none = Añade al menos un paso con [a].
tui-workflows-step-agent-required = Paso { $step }: { $source } es obligatorio
tui-workflows-step-prompt-required = Paso { $step }: el prompt es obligatorio
tui-workflows-label-review = Revisión: pulsa Enter para crear
tui-workflows-review-name =   Nombre:  
tui-workflows-review-desc =   Desc.:   
tui-workflows-review-steps =   Pasos:   
tui-workflows-hints-create-submit =   [Enter] Crear  [Esc] Volver
tui-workflows-hints-create-next =   [Enter] Siguiente  [Esc] Volver
tui-workflows-hints-create-steps-list =   [a] Añadir  [d] Eliminar  [↑↓] Seleccionar paso  [Tab] Campos  [Enter] Siguiente  [Esc] Volver
tui-workflows-hints-create-steps-text =   [Tab/Shift-Tab] Campos  [↑↓] Seleccionar paso  [Enter] Siguiente  [Esc] Volver
tui-workflows-hints-create-steps-select =   [Espacio/←→] Cambiar  [Tab/Shift-Tab] Campos  [↑↓] Seleccionar paso  [Enter] Siguiente  [Esc] Volver
tui-workflows-title-run-input = Ejecutar: { $name }
tui-workflows-label-run-input =   Entrada (JSON o texto):
tui-workflows-placeholder-run-input = introduce la entrada del flujo de trabajo...
tui-workflows-param-required = { $name } es obligatorio
tui-workflows-params-none = Este flujo de trabajo no declara parámetros; la entrada se envía como texto libre.
tui-workflows-params-failed = No se pudieron cargar los parámetros declarados; la entrada se envía como texto libre.
tui-workflows-hints-run-input =   [Enter] Ejecutar  [Tab/↑↓] Campos  [Esc] Cancelar
tui-workflows-title-run-result = Resultado de la ejecución del flujo de trabajo
tui-workflows-running = Ejecutando el flujo de trabajo...
tui-workflows-result-complete = Completado
tui-workflows-result-empty = Sin resultado.
tui-workflows-hints-run-result =   [Enter/Esc] Volver

# --- Triggers screen ---
tui-triggers-title-screen = Activadores
tui-triggers-header-agent = Agente
tui-triggers-header-pattern = Patrón
tui-triggers-header-fires = Disparos
tui-triggers-header-status = Estado
tui-triggers-loading = Cargando activadores...
tui-triggers-empty-state = No hay activadores configurados. Crea uno con [n].
tui-triggers-status-active = ● Activo
tui-triggers-status-off = ○ Inactivo
tui-triggers-create-new-option =   + Crear un activador
tui-triggers-hints-list =   [↑↓] Navegar  [Enter] Crear  [d] Eliminar  [r] Actualizar
tui-triggers-title-create = Crear un activador
tui-triggers-create-step =   Paso { $current } de { $total }
tui-triggers-label-agent-id = ID del agente:
tui-triggers-placeholder-agent-id = agent-uuid
tui-triggers-label-pattern-picker =   Selecciona el tipo de patrón:
tui-triggers-prompt-param = Parámetro del patrón para { $type }:
tui-triggers-placeholder-pattern-param = p. ej. .*error.*
tui-triggers-label-prompt = Plantilla del prompt:
tui-triggers-placeholder-prompt = Gestiona esto: {"{"}event{"}"}
tui-triggers-label-max-fires = Disparos máximos (0 = ilimitado):
tui-triggers-placeholder-max-fires = 0
tui-triggers-review-agent =   Agente:   
tui-triggers-review-pattern =   Patrón: 
tui-triggers-review-prompt =   Prompt:  
tui-triggers-review-max =   Máx.:     
tui-triggers-review-unlimited = ilimitado
tui-triggers-review-confirm = Pulsa Enter para crear este activador.
tui-triggers-hints-create-submit =   [Enter] Crear  [Esc] Volver
tui-triggers-hints-create-select =   [↑↓] Navegar  [Enter] Seleccionar  [Esc] Volver
tui-triggers-hints-create-next =   [Enter] Siguiente  [Esc] Volver

tui-triggers-type-lifecycle-name = Lifecycle
tui-triggers-type-lifecycle-desc = Eventos del ciclo de vida del agente (inicio, parada, error)
tui-triggers-type-agentspawned-name = AgentSpawned
tui-triggers-type-agentspawned-desc = Se dispara cuando se crea un agente nuevo
tui-triggers-type-contentmatch-name = ContentMatch
tui-triggers-type-contentmatch-desc = Coincidencia en el contenido del mensaje (regex)
tui-triggers-type-schedule-name = Schedule
tui-triggers-type-schedule-desc = Activador con programación tipo cron
tui-triggers-type-webhook-name = Webhook
tui-triggers-type-webhook-desc = Activador por webhook HTTP
tui-triggers-type-channelmessage-name = ChannelMessage
tui-triggers-type-channelmessage-desc = Mensaje recibido en un canal

# --- Memory screen ---
tui-memory-title-screen = Memoria
tui-memory-config-remembering = Qué se recuerda
tui-memory-config-auto-memorize = Recordar automáticamente
tui-memory-config-auto-retrieve = Recuperar automáticamente
tui-memory-config-extraction-model = Modelo que escribe las memorias
tui-memory-config-inherited = (heredado del valor predeterminado del sistema)
tui-memory-config-pending = guardado pendiente
tui-memory-config-restart-required = guardado; reinicia para usarlo
tui-memory-config-searching = Cómo se buscan las memorias
tui-memory-config-embedding-provider = Proveedor de búsqueda
tui-memory-config-embedding-model = Modelo de búsqueda
tui-memory-config-on = activado
tui-memory-config-off = desactivado
tui-memory-config-loading = Leyendo la configuración...
tui-memory-config-saving = Guardando...
tui-memory-config-saved = Guardado
tui-memory-config-save-failed = El guardado falló: { $error }
tui-memory-config-unsaved = Cambios sin guardar; pulsa s para guardar, Esc para descartar
tui-memory-config-save-partial = Guardado en disco, pero la recarga en caliente falló; hace falta reiniciar
tui-memory-config-hint =   j/k seleccionar · espacio/Enter alternar o editar · s guardar · r actualizar · Esc volver
tui-memory-label-select-agent =   Selecciona un agente para explorar su memoria:
tui-memory-header-agent-name = Nombre del agente
tui-memory-header-id = ID
tui-memory-loading-agents = Cargando agentes...
tui-memory-empty-agents = No hay entradas de memoria. Los agentes guardan datos aquí automáticamente.
tui-memory-hints-agent-select =   ↑↓ Navegar  Enter Explorar KV  r Actualizar
tui-memory-pairs-count =   │ { $count } pares
tui-memory-header-key = Clave
tui-memory-header-value = Valor
tui-memory-loading = Cargando...
tui-memory-empty-kv = No hay pares clave-valor. Pulsa a para añadir uno.
tui-memory-confirm-delete =   ¿Eliminar esta clave? [y] Sí  [cualquier tecla] Cancelar
tui-memory-hints-kv-browser =   ↑↓ Navegar  a Añadir  e Editar  d Eliminar  Esc Volver  r Actualizar
tui-memory-title-add = ┼ Añadir par clave-valor
tui-memory-title-edit = ✎ Editar valor
tui-memory-field-key = Clave:
tui-memory-placeholder-key = introduce la clave...
tui-memory-field-value = Valor:
tui-memory-placeholder-value = introduce el valor...
tui-memory-hints-edit =   Tab Cambiar de campo  Enter Guardar  Esc Cancelar

# --- Extensions screen ---
tui-extensions-title-screen = Extensiones
tui-extensions-tab-browse = 1 Explorar
tui-extensions-tab-installed = 2 Instaladas
tui-extensions-tab-health = 3 Estado
tui-extensions-status-ready = Listo
tui-extensions-status-setup = Configuración
tui-extensions-status-error = Error
tui-extensions-status-off = Apagado
tui-extensions-status-installed = Instalada
tui-extensions-status-available = Disponible
tui-extensions-header-name = Nombre
tui-extensions-header-category = Categoría
tui-extensions-header-status = Estado
tui-extensions-header-desc = Descripción
tui-extensions-header-id = ID
tui-extensions-header-server = Servidor
tui-extensions-header-tools = Herramientas
tui-extensions-header-connected = Conectado
tui-extensions-header-fails = Fallos
tui-extensions-header-last-error = Último error
tui-extensions-loading = Cargando servidores MCP...
tui-extensions-empty = No hay extensiones instaladas. Explora el mercado con [b].
tui-extensions-remove-confirm =   Pulsa y para confirmar la eliminación, cualquier otra tecla para cancelar
tui-extensions-hints-search =   Escribe para buscar • Esc cancelar • Enter confirmar
tui-extensions-hints-browse =   j/k navegar • Enter instalar • / buscar • r actualizar
tui-extensions-hints-installed =   j/k navegar • d eliminar • r actualizar
tui-extensions-hints-health =   j/k navegar • r/Enter reconectar • reconexión automática activa

# --- Skills screen ---
tui-skills-title-screen = Habilidades
tui-skills-tab-installed = 1 Instaladas
tui-skills-tab-clawhub = 2 ClawHub
tui-skills-tab-mcp = 3 Servidores MCP
tui-skills-header-name = Nombre
tui-skills-header-runtime = Runtime
tui-skills-header-source = Origen
tui-skills-header-desc = Descripción
tui-skills-header-downloads = Descargas
tui-skills-header-server = Servidor
tui-skills-header-status = Estado
tui-skills-header-tools = Herramientas
tui-skills-loading = Cargando habilidades...
tui-skills-empty = No hay habilidades instaladas. Explora ClawHub para encontrar habilidades.
tui-skills-uninstall-confirm =   ¿Desinstalar esta habilidad? [y] Sí  [cualquier tecla] Cancelar
tui-skills-hints-installed =   [↑↓] Navegar  [u] Desinstalar  [r] Actualizar
tui-skills-sort =   Ordenar: { $sort }
tui-skills-sort-trending = tendencia
tui-skills-sort-popular = populares
tui-skills-sort-recent = recientes
tui-skills-searching = Buscando en ClawHub...
tui-skills-empty-clawhub = Sin resultados. Pulsa [/] para buscar o [s] para cambiar el orden.
tui-skills-hints-clawhub =   [↑↓] Navegar  [i] Instalar  [/] Buscar  [s] Ordenar  [r] Actualizar
tui-skills-loading-mcp = Cargando servidores MCP...
tui-skills-empty-mcp = No hay servidores MCP configurados. Añade servidores en config.toml.
tui-skills-hints-mcp =   [↑↓] Navegar  [r] Actualizar
tui-skills-mcp-status-connected = Conectado
tui-skills-mcp-status-disconnected = Desconectado
tui-skills-mcp-tools-count = { $count } herramientas

# --- Setup Wizard screen ---
tui-wizard-title = Configuración
tui-wizard-step-1 = Paso 1 de 3
tui-wizard-step-2 = Paso 2 de 3
tui-wizard-step-3 = Paso 3 de 3
tui-wizard-step-saving = Guardando...
tui-wizard-step-complete = Completado
tui-wizard-prompt-provider = Elige tu proveedor LLM:
tui-wizard-hint-cli-detected = CLI detectada
tui-wizard-hint-no-key-needed = no hace falta clave API
tui-wizard-hint-local-no-key = local, no hace falta clave
tui-wizard-hint-env-detected = { $env } detectada
tui-wizard-hint-env-required = requiere { $env }
tui-wizard-hints-provider =     [↑↓] Navegar  [Enter] Seleccionar  [Esc] Cancelar
tui-wizard-prompt-api-key = Introduce tu clave API de { $provider }:
tui-wizard-hint-env-alternative = O define la variable de entorno { $env }
tui-wizard-hints-confirm-back =     [Enter] Confirmar  [Esc] Volver
tui-wizard-prompt-model-name = Nombre del modelo:
tui-wizard-hint-model-default = predeterminado: { $model }
tui-wizard-status-no-provider = No se seleccionó ningún proveedor
tui-wizard-status-no-home = No se pudo determinar el directorio personal
tui-wizard-status-saved = Configuración guardada: { $provider } / { $model }
tui-wizard-status-save-fail = Error al guardar la configuración: { $error }
tui-wizard-status-continuing = Continuando...
tui-agents-param-temperature = Temperatura
tui-agents-param-temperature-hint = temperature — cuánto varía el modelo su redacción. Más bajo es más estable.
tui-agents-param-top-p = Top-p
tui-agents-param-top-p-hint = top_p — muestreo de núcleo. Normalmente se deja en heredar.
tui-agents-param-frequency-penalty = Penalización por frecuencia
tui-agents-param-frequency-penalty-hint = frequency_penalty — desincentiva repetir las mismas palabras.
tui-agents-param-presence-penalty = Penalización por presencia
tui-agents-param-presence-penalty-hint = presence_penalty — desincentiva volver a los mismos temas.
tui-agents-param-top-k = Top-K
tui-agents-param-top-k-hint = top_k — considera solo los K tokens más probables. Solo Anthropic, Gemini, Ollama, OpenRouter y servidores locales.
tui-agents-param-min-p = Probabilidad mínima
tui-agents-param-min-p-hint = min_p — descarta tokens mucho menos probables que el primero. Solo Ollama, vLLM, llama.cpp y OpenRouter.
tui-agents-param-repeat-penalty = Penalización por repetición
tui-agents-param-repeat-penalty-hint = repeat_penalty — desincentiva los tokens repetidos recientemente; 1.0 lo desactiva. Solo Ollama, OpenRouter y servidores locales.
tui-agents-param-max-tokens = Longitud de la respuesta
tui-agents-param-max-tokens-hint = max_tokens — qué respuesta tan larga pide este agente. La tuya tiene prioridad sobre la del modelo.
tui-agents-param-context-window = Ventana de contexto
tui-agents-param-context-window-hint = context_window — cuánto puede leer este endpoint. Es un límite, no una preferencia.
tui-agents-param-max-output-tokens = Límite de salida
tui-agents-param-max-output-tokens-hint = max_output_tokens — el límite de salida del propio endpoint. Es un límite, no una preferencia.
tui-agents-param-not-a-number = { $field } debe ser un número
tui-agents-param-not-whole = { $field } debe ser un número entero mayor que 0
tui-agents-param-out-of-range = { $field } debe estar entre { $min } y { $max }
agent-set-invalid-integer = { $field } debe ser un número entero positivo; se recibió '{ $value }'
agent-set-invalid-decimal = { $field } debe ser un número decimal; se recibió '{ $value }'
agent-set-limit-warning = aviso: { $message }

# Model routing editor (profile-based routing)
tui-agents-title-model-routing = Enrutamiento de modelos
tui-agents-label-routing-fixed = fijo — usar siempre el modelo propio de este agente
tui-agents-label-routing-flexible = flexible — dejar que el enrutador elija según la tarea
tui-agents-hint-routing-mode = [Tab] cambiar modo
tui-agents-label-routing-fixed-explainer = Este agente usa siempre el modelo de su propio manifiesto. Pulsa Tab para que el enrutador elija según la tarea.
tui-agents-label-routing-stable-inert = El kernel se ejecuta en modo Stable: no se ejecuta ningún enrutador y este agente usa { $model }. Los ajustes guardados aquí se aplicarán cuando el kernel salga del modo Stable.
tui-agents-label-routing-stable-inert-manifest = El kernel se ejecuta en modo Stable: no se ejecuta ningún enrutador y este agente usa el modelo de su manifiesto. Los ajustes guardados aquí se aplicarán cuando el kernel salga del modo Stable.
tui-agents-label-no-router-profiles = No hay perfiles de modelo disponibles. Añádelos en ~/.librefang/model_profiles.toml.
tui-agents-label-routing-any-profile = cualquiera
tui-agents-hints-model-routing = [Tab] Modo  [↑↓] Navegar  [Espacio] Alternar perfil  [+/-] Presupuesto de coste  [Enter] Guardar  [Esc] Cancelar
tui-agents-model-routing-not-loaded = Los ajustes de enrutamiento de este agente no se han cargado; espera un momento, o pulsa Esc y vuelve a abrirlo con r si la consulta falló.
tui-event-model-routing-fetch-failed = Error al obtener el enrutamiento de modelos
tui-event-model-routing-update-failed = Error al actualizar el enrutamiento de modelos
tui-mod-agent-model-routing-updated = Enrutamiento de modelos actualizado para el agente { $id }.

# --- Goals screen (TUI) ---
tui-goals-title = ⌖ Objetivos
tui-goals-count = { $count } objetivos
tui-goals-filter = filtro: { $query }
tui-goals-loading = Cargando objetivos...
tui-goals-empty = Aún no hay objetivos. Pulsa [n] para crear uno.
tui-goals-hints = [↑↓] Navegar  [Enter] Detalle  [n] Nuevo  [s] Iniciar/parar  [p] Pausar/reanudar  [d] Eliminar  [/] Buscar  [r] Actualizar
tui-goals-confirm-delete = ¿Eliminar este objetivo? [y] confirmar  [cualquier otra tecla] cancelar
tui-goals-none-selected = No hay ningún objetivo seleccionado.
tui-goals-agent-none = sin asignar
tui-goals-label-status = Estado:
tui-goals-label-description = Descripción:
tui-goals-label-agent = Agente:
tui-goals-label-progress = Progreso:
tui-goals-label-phase = Fase de ejecución:
tui-goals-label-iteration = Iteración:
tui-goals-label-title = Título:
tui-goals-label-loop-engineering = Ingeniería de bucle:
tui-goals-label-verifier = Agente verificador:
tui-goals-label-evaluator = Modelo evaluador:
tui-goals-label-verify-rounds = Rondas de verificación:
tui-goals-loop-engineering-on = activada
tui-goals-loop-engineering-off = desactivada
tui-goals-verifier-none = ninguno
tui-goals-evaluator-none = ninguno, decide el marcador del propio agente
tui-goals-hint-stop = [s] detener ejecución
tui-goals-hint-start = [s] iniciar ejecución
tui-goals-hint-rounds = [+/-] Rondas (verify_max_retries)
tui-goals-detail-hints = [q/Esc] Cerrar { $run_hint } [r] Actualizar
tui-goals-verify-rounds-default = predeterminado
tui-goals-verify-rounds-next = { $current } (siguiente: { $next })
tui-goals-new-title = Nuevo objetivo
tui-goals-step = paso { $n }/{ $total }
tui-goals-title-hint = Un nombre corto para el objetivo, por ejemplo "Arreglar el timeout de inicio de sesión".
tui-goals-description-hint = Qué debe conseguir el agente. Esto se convierte en su prompt.
tui-goals-agent-hint = Nombre o UUID del agente que perseguirá el objetivo. Obligatorio: un objetivo sin agente no puede ejecutarse.
tui-goals-loop-engineering-hint = Condiciona cada iteración al veredicto de un segundo agente en lugar de a la afirmación de finalización del propio agente. Desactivado reproduce exactamente el bucle de objetivo simple. [espacio] lo alterna, [y]/[n] lo definen. Campo: loop_engineering.
tui-goals-verifier-hint = UUID del agente que evalúa el trabajo. Déjalo vacío para no tener puerta de verificación. Un agente no evalúa su propio trabajo. Campo: verify_agent_id.
tui-goals-evaluator-hint = Modelo que juzga si se ha cumplido el objetivo, independientemente del marcador GOAL_DONE del agente. Un modelo pequeño y barato es la opción prevista. Déjalo vacío para depender solo del marcador. Campo: evaluator_model.
tui-goals-nav-next = [Enter] Siguiente  [Esc] Volver
tui-goals-nav-submit = [Enter] Crear  [Esc] Volver
tui-goals-create-incomplete = Se requieren tanto un título como un agente.
tui-goals-label-tick = Intervalo de tick:
tui-goals-tick-hint = Cuánto espera el runner entre iteraciones del bucle, en segundos (tick_interval_secs). Déjalo vacío para el valor predeterminado de { $default }. Rango admitido { $min }–{ $max }.
tui-goals-tick-value = { $secs }s
tui-goals-tick-default = { $secs }s (predeterminado)
tui-goals-tick-out-of-range = El intervalo de tick debe estar entre { $min } y { $max } segundos.
tui-goals-phase-actv = ● ACTV
tui-goals-phase-done = ○ HECHO
tui-goals-phase-canc = ○ CANC
tui-goals-phase-fail = ● FALL
tui-goals-phase-pend = ○ PEND
tui-goals-phase-paused = ⏸ PAUS
tui-goals-phase-rate-limited = ⚡ RLIM
tui-goals-phase-max-iterations = ⏱ MXIT
tui-goals-run-running = en ejecución
tui-goals-run-paused = en pausa
tui-goals-run-finished = finalizado
tui-goals-run-max-iterations = límite de iteraciones alcanzado
tui-goals-run-rate-limited = detenido por el limitador de tasa
tui-goals-run-stopped = detenido por un operador
tui-goal-created = Objetivo creado: { $id }
tui-goal-deleted = Objetivo eliminado: { $id }
tui-goal-run-started = Ejecución iniciada: { $id }
tui-goal-run-stopped = Ejecución detenida: { $id }
tui-goal-run-paused = Ejecución pausada: { $id }
tui-goal-run-resumed = Ejecución reanudada: { $id }
tui-goal-create-failed = No se pudo crear el objetivo.
tui-goal-create-error = No se pudo crear el objetivo: { $error }
tui-goal-delete-failed = No se pudo eliminar el objetivo.
tui-goal-start-failed = No se pudo iniciar la ejecución.
tui-goal-start-error = No se pudo iniciar la ejecución: { $error }
tui-goal-stop-failed = No se pudo detener la ejecución.
tui-goal-pause-failed = No se pudo pausar la ejecución.
tui-goal-resume-failed = No se pudo reanudar la ejecución.
tui-goal-inproc-unavailable = La gestión de objetivos no está disponible en modo en proceso.

# --- goal command (CLI) ---
cmd-goal-agent-required = Un objetivo necesita un agente que lo persiga.
cmd-goal-agent-required-fix = Pasa --agent <name-or-uuid>. Ejecuta `librefang agents list` para ver los agentes disponibles.
cmd-goal-agent-unknown = Ningún agente coincide con '{ $agent }'.
cmd-goal-created = Objetivo creado: { $id }
cmd-goal-create-error = Error al crear el objetivo: { $error }
cmd-goal-start-error = Error al iniciar la ejecución: { $error }
cmd-goal-watching = Observando la ejecución. Ctrl+C deja de observar; la ejecución continúa.
cmd-goal-progress = [{ $current }/{ $total }] { $phase } — { $progress }% completado
cmd-goal-finished = El objetivo terminó correctamente.
cmd-goal-max-iterations = Objetivo detenido: límite de iteraciones alcanzado.
cmd-goal-rate-limited = Objetivo detenido: límite de tasa del proveedor.
cmd-goal-stopped = Ejecución del objetivo detenida.
cmd-goal-paused = Ejecución del objetivo pausada.
cmd-goal-error = error: { $error }
cmd-goal-unknown-error = error desconocido
cmd-goal-watch-poll-error = No se pudo leer el estado de la ejecución ({ $count }/{ $max }); reintentando…
cmd-goal-watch-poll-gave-up = Se dejó de observar la ejecución tras { $count } sondeos fallidos; el resultado es desconocido y la ejecución puede seguir en curso.



# ── User groups (#7745) ─────────────────────────────────────────────────────
group-none = No hay grupos configurados.
group-section-detail = Grupo
group-label-name = Nombre
group-label-description = Descripción
group-label-roles = Roles
group-label-members = Miembros
group-label-groups = Grupos
group-label-unregistered = Miembros no registrados
group-header-members = MIEMBROS
group-header-roles = ROLES
group-header-description = DESCRIPCIÓN
group-created = Grupo { $name } creado.
group-create-failed = Error al crear el grupo: { $error }
group-deleted = Grupo { $name } eliminado.
group-delete-failed = Error al eliminar el grupo: { $error }
group-show-failed = Error al leer el grupo: { $error }
group-member-added = { $user } añadido a { $group }.
group-member-removed = { $user } eliminado de { $group }.
group-member-failed = Error al cambiar la pertenencia: { $error }
group-user-none = { $user } no pertenece a ningún grupo.
tui-tab-groups = Grupos
tui-groups-title = Grupos
tui-groups-count = { $count } grupo(s)
tui-groups-header-name = NOMBRE
tui-groups-header-members = MIEMBROS
tui-groups-header-roles = ROLES
tui-groups-header-description = DESCRIPCIÓN
tui-groups-loading = Cargando grupos…
tui-groups-empty = No hay grupos configurados. Crea uno con `librefang group create <name>`.
tui-groups-hints = ↑/↓ navegar · r actualizar

tui-goals-judge-label = { "  " }Juez del objetivo:{ " " }
tui-goals-phase-label = { "  " }Fase de ejecución:{ " " }
# purge command
purge-failed-config = Purga abortada: no se pudo cargar la configuración, así que se desconoce la instalación que purgar: { $error }
purge-database-line = Base de datos: { $path }
purge-home-line = Inicio (tareas cron, activadores de eventos, plantillas de tipo de agente, espacios de trabajo): { $path }
purge-failed-no-database = La purga falló: no hay base de datos en { $path }
purge-failed-open-database = La purga falló: abrir la base de datos: { $error }
purge-nothing-to-purge = Nada que purgar: '{ $agent }' no dejó rastro en esta instalación.
purge-purged-header = '{ $agent }' purgado:
purge-removed-roster-entry =   - entrada del registro, sesiones, memorias y filas KV
purge-removed-orphaned-data =   - sesiones, memorias y filas KV huérfanas (la entrada del registro ya había desaparecido)
purge-removed-identity-record =   - registro del UUID canónico en agent_identities.toml
purge-removed-workspace =   - directorio del espacio de trabajo
purge-workspace-unresolved =   - NOTA: no se pudo localizar el directorio del espacio de trabajo. No sobrevive ningún manifiesto de este agente, así que no se habría encontrado una ruta de espacio de trabajo personalizada; revisa tú mismo el directorio de espacios de trabajo.
purge-workspace-shared =   - NOTA: el directorio del espacio de trabajo se dejó intacto. Resuelve a la propia raíz de espacios de trabajo, o a un directorio al que también resuelve el manifiesto de otro agente registrado; borrarlo habría destruido más que los datos de este agente; revisa tú mismo el directorio de espacios de trabajo.
purge-removed-agent-type =   - plantilla de tipo de agente
purge-removed-avatar =   - imagen de avatar
purge-removed-cron-jobs =   - tareas cron
purge-removed-trigger-jobs =   - activadores de eventos
purge-removed-channel-bindings =   - enlaces de enrutamiento de canal/conversación
purge-other-orphans-present =   - NOTA: esta instalación tiene filas huérfanas bajo un id que no se pudo atribuir a ningún nombre (probablemente un agente hijo o efímero cuya entrada del registro ya desapareció); se dejaron intactas.
purge-failure-line =   - FALLÓ: { $error }
purge-confirm-header = A punto de purgar de forma permanente '{ $agent }':
purge-confirm-warning = Esto no se puede deshacer.
purge-dry-run-header = Simulación: se purgaría '{ $agent }' (abrir la base de datos puede aplicar migraciones de esquema pendientes y reindexar el índice de búsqueda, aunque no se borre nada):
purge-failed-daemon-running = Purga abortada: hay un daemon en ejecución en { $url } que retiene los datos de esta instalación. Deténlo primero, o pasa --force si sabes lo que haces.

# --- Channels screen (#8044) ---
tui-tab-channels = Canales
tui-channels-title-screen = Canales
tui-channels-header-instance = INSTANCIA
tui-channels-header-agent = AGENTE
tui-channels-header-status = ESTADO
tui-channels-header-traffic = ENT/SAL
tui-channels-loading = Cargando instancias de canal…
tui-channels-empty-state = No hay instancias de canal configuradas. Añade una con [a].
tui-channels-agent-unbound = (ninguno)
tui-channels-group-count = { " " }({ $count })
tui-channels-add-new-option =   + Añadir una instancia de canal
tui-channels-status-online = ● En línea
tui-channels-status-degraded = ◐ Degradado
tui-channels-status-stopped = ○ Detenido
tui-channels-status-unsupervised = ○ No iniciado
tui-channels-confirm-delete =   ¿Eliminar la instancia de canal { $name }? [y/n]
tui-channels-hints-list =   [↑↓] Navegar  [Enter/e] Editar  [a] Añadir  [d] Eliminar  [r] Actualizar  [R] Recargar adaptadores
tui-channels-title-picker = Elige el adaptador en el que se ejecuta la nueva instancia
tui-channels-picker-empty = No hay adaptadores sidecar disponibles. Instala el SDK de Python y luego recarga.
tui-channels-no-schema = sin esquema; configura a mano
tui-channels-hints-picker =   [↑↓] Navegar  [Enter] Seleccionar  [Esc] Volver
tui-channels-title-create = Añadir instancia de canal
tui-channels-title-edit = Editar instancia de canal
tui-channels-label-instance-name = Nombre de la instancia
tui-channels-label-instance-name-locked = Nombre de la instancia (fijo)
tui-channels-label-agent = Agente predeterminado
tui-channels-placeholder-agent = (ningún agente vinculado)
tui-channels-secret-stored = (guardado; vuelve a escribirlo para guardar)
tui-channels-hints-form =   [↑↓] Campo  [Espacio/←→] Alternar  [Enter] Guardar  [Esc] Cancelar
tui-channels-error-no-adapter = Elige un adaptador antes de guardar.
tui-channels-error-name-required = El nombre de la instancia no puede estar vacío.
tui-channels-error-name-invalid = { $name } no es un nombre de instancia utilizable; usa solo letras, dígitos, `-`, `_` o `.`.
tui-channels-error-name-taken = Ya existe una instancia llamada { $name }.
tui-channels-error-name-secret-collision = { $name } y la { $other } existente se asignan ambas al prefijo de secretos { $prefix }__, así que compartirían sus secretos. Elige un nombre que difiera en algo más que la puntuación.
tui-channels-error-field-required = { $label } es obligatorio.
tui-channels-error-secret-required = { $label } está guardado pero no se puede leer de vuelta; vuelve a escribirlo para guardar.
tui-mod-channel-saved = Instancia de canal { $name } guardada.
tui-mod-channel-saved-restart-required = Instancia de canal { $name } guardada; reinicia el daemon para aplicarlo.
tui-mod-channel-saved-shadowed = Instancia de canal { $name } guardada, pero { $keys } está definida en el entorno del daemon y tiene más prioridad; quítala y reinicia.
tui-mod-channel-deleted = Instancia de canal { $name } eliminada.
tui-mod-channels-reloaded = Canales recargados ({ $started } iniciados).
tui-event-channel-instances-fetch-failed = Error al cargar las instancias de canal: { $error }
tui-event-aux-fetch-failed = Error al cargar las cadenas LLM auxiliares
tui-event-aux-save-failed = Error al guardar la cadena auxiliar de { $task }
tui-event-aux-save-not-available-in-process = La edición de cadenas auxiliares no está disponible en modo en proceso
tui-event-aux-tasks-fetch-failed = Error al cargar la lista de tareas auxiliares: { $error }
tui-event-aux-tasks-schema-missing = El esquema de configuración del daemon no llevaba ninguna lista de tareas auxiliares; mostrando solo las tareas ya configuradas.
tui-event-channels-not-available-in-process = La gestión de canales necesita un daemon en ejecución.
tui-event-channel-save-failed = Error al guardar la instancia de canal { $name }: { $error }
tui-event-channel-delete-failed = Error al eliminar la instancia de canal { $name }: { $error }
tui-event-channels-reload-failed = Error al recargar los canales: { $error }

# --- TUI fetch failures that used to be silent (#8141) ---
tui-memory-config-requires-daemon = Los ajustes de memoria vienen de la API del daemon; no están disponibles cuando la TUI está conectada en proceso.
tui-memory-config-fetch-failed = No se pudieron leer los ajustes de memoria: { $error }
tui-goals-run-requires-daemon = El estado de ejecución viene de la API del daemon; no está disponible cuando la TUI está conectada en proceso.
tui-goals-run-fetch-failed = No se pudo leer el estado de la ejecución de { $id }: { $error }

# config_editor.rs — generic config-section editor (#8165)
tui-settings-tab-config = 6 Configuración
tui-settings-hints-config =   [↑↓] Navegar  [Enter] Abrir sección  [r] Actualizar
tui-settings-hints-config-fields =   [↑↓] Navegar  [Enter] Editar / Alternar  [Esc] Volver  [r] Actualizar
tui-settings-config-loading = Cargando la configuración…
tui-settings-config-empty = No hay secciones de configuración disponibles. El daemon debe estar en ejecución.
tui-settings-config-select-section = Elige una sección a la izquierda y pulsa [Enter] para ver sus ajustes.
tui-settings-config-header-section = Sección
tui-settings-config-header-setting = Ajuste
tui-settings-config-header-value = Valor
tui-settings-config-unset = sin definir
tui-settings-config-on = activado
tui-settings-config-off = desactivado
tui-settings-config-readonly = solo lectura
tui-settings-config-readonly-msg = { $path } es de solo lectura aquí; edita ~/.librefang/config.toml directamente para cambiarlo.
tui-settings-config-complex = { $path } contiene una lista o una tabla; edita ~/.librefang/config.toml directamente para cambiarlo.
tui-settings-config-invalid = Ese no es un valor válido para { $path }.
tui-settings-config-redacted = { $path } se muestra oculto; escribe el nuevo valor completo; una entrada vacía se rechaza aquí.
tui-settings-config-prompt = Define { $path } (vacío lo borra):
tui-mod-config-value-saved = { $path } guardado
tui-mod-config-value-saved-restart = { $path } guardado; reinicia el daemon para que surta efecto
tui-mod-config-value-saved-reload-failed = { $path } guardado en config.toml, pero la recarga falló: { $error }
tui-event-config-schema-failed = Error al cargar el esquema de configuración
tui-event-config-schema-unreadable = El daemon respondió, pero no se pudo leer su esquema de configuración: { $error }
tui-event-config-failed = Error al cargar la configuración actual
tui-event-config-unreadable = El daemon respondió, pero no se pudo leer la configuración actual: { $error }
tui-event-config-set-failed = Error al guardar { $path }
tui-event-config-need-daemon = Editar la configuración necesita un daemon en ejecución

# Model routing CLI commands
agent-routing-label-mode = Modo
agent-routing-label-allowed = Perfiles permitidos
agent-routing-label-budget = Presupuesto de coste
agent-routing-label-default = Perfil predeterminado
agent-routing-any-profile = cualquiera
agent-routing-no-cap = sin límite
agent-routing-fixed-explainer = Este agente usa siempre el modelo de su propio manifiesto.
agent-routing-label-fixed = Exclusión del enrutador
agent-routing-fixed-opt-out = fijo — el enrutador nunca toca este agente
agent-routing-label-stable = Modo Stable
agent-routing-stable-inert = no se ejecuta ningún enrutador; este agente usa { $model } hasta que el kernel salga del modo Stable
agent-routing-stable-inert-manifest = no se ejecuta ningún enrutador; este agente usa el modelo de su manifiesto hasta que el kernel salga del modo Stable
agent-routing-updated = Enrutamiento de modelos del agente { $id } definido como { $mode }.
agent-routing-failed = Error al actualizar el enrutamiento de modelos: { $error }
agent-routing-profiles-header = Perfiles del enrutador de modelos (enrutador: { $enabled }):
tui-agents-line-routing-mode =   Modo: { $mode }
tui-agents-line-routing-summary =   Presupuesto de coste: { $budget }    Perfiles permitidos: { $allowed }
tui-agents-label-routing-no-cap = sin límite
tui-agents-label-routing-cheap = barato
tui-agents-label-routing-medium = medio
tui-agents-label-routing-expensive = caro
