# Plan: multi-contexto per-chat para fandangorodelo + fix NO_REPLY

## Context — lo que el usuario quiere

El bot `fandangorodelo` en Telegram tiene dos defectos observados:

1. **Cross-chat bleed.** Habla en el grupo rodela, le escribe por DM, y las respuestas se cruzan. El bot no distingue contextos.
2. **NO_REPLY literal.** A veces cuando el bot decide no contestar, manda `NO_REPLY` o `[no reply needed]` como texto del mensaje en lugar de callarse.

El usuario quiere **separación real de contextos** por chat: historial, tools activas, memoria (selectiva: global o per-chat). Además quiere, a futuro, que el agente pueda leer contenido de otros chats bajo demanda pero con lógica — no trivialmente.

## Estado upstream — lo que ya existe y lo que no

Antes de diseñar nada, búsqueda upstream (`librefang/librefang`):

- **Bug 1 — cross-chat session.** Ya filé #2349 hace dos días ("agent sessions are global per agent, not per chat_id — causes context leakage") y **lo cerré yo mismo** como "wrong root cause, real fix was cron delivery (`a04705d6`)". Fue un cierre prematuro: el tracer ha confirmado que el código de `SessionId::for_channel` sigue siendo `(agent, channel)` sin `chat_id`, y el bug sigue vivo. **Hay que reabrir #2349 o filar uno fresco con el trace.**

- **Bug 2 — NO_REPLY leak.** Existe #976 ("Silent/NO_REPLY responses leak debug fallback message to channels"), cerrado por PR #1143 ("upstream parity — 10 bug fixes from release comparison"). Ese PR arregló `routes.rs` y `ws.rs` pero **dejó dos métodos del `channel_bridge` adapter sin tocar**: `send_message_with_sender` (línea 551) y `send_message_with_blocks_and_sender` (línea 578). El Telegram adapter usa esos dos métodos, por eso el bug persiste. **Enfoque upstream:** "follow-up a #976/#1143 — dos métodos del KernelBridgeAdapter olvidados".

- **Fase 2 — per-peer memory.** **Ya implementada upstream** vía #1978 + PR #2015 ("feat(memory): per-peer memory isolation for multi-user channels", merged 2026-04-03). Prefijo de clave `peer:{peer_id}:{key}` cuando `sender_id` está presente. Archivos afectados: `crates/librefang-memory/{semantic,proactive,migration}.rs`, `crates/librefang-kernel/src/kernel.rs`, `crates/librefang-runtime/src/{tool_runner,kernel_handle,agent_loop}.rs`. **Implicación crítica:** el scoping per-peer ya existe. Pero es **per-user**, no per-chat. Mi Fase 2 original asumía que no existía plumbing — la realidad es que hay que añadir un segundo prefijo opcional `chat:{chat_id}:` encima o en lugar de `peer:`.

- **#2291 (OPEN)** — mi propia feature request "LLM-based reply-intent precheck for group conversations". Es un mecanismo para que el bot decida si contestar un mensaje de grupo o no. Ortogonal a los dos bugs pero conceptualmente relacionado a silenciar respuestas en grupos. Anotable como feature futura **después** de Fase 1.

- **#2262 (closed)** — "Agent has no group roster and lacks instructions to distinguish senders in multi-user groups". Ortogonal pero relacionado al modelo multi-contexto. No crítico ahora.

**Consecuencia para el plan:** Fase 1 sigue igual de necesaria (bugs reales, código nuevo). Fase 2 es mucho más pequeña — se apoya en el plumbing de PR #2015 y añade una dimensión de scope. Fase 3 (cross-chat read) sigue siendo R&D. Upstream queda claro: Fase 1 es PR directa, Fase 2 es feature que podría ir upstream con flag (porque la base ya está upstream).

Esto no es un parche: es una refactorización arquitectural. Va por fases. Fase 1 ships esta semana, Fases 2-4 esperan decisiones del usuario después de verificar Fase 1.

---

## Problema 1 — Cross-chat bleed

**Root cause (confirmado por trace):** `SessionId::for_channel(agent_id, channel)` en `crates/librefang-types/src/agent.rs:208-214` compone la clave como `"{agent}:telegram"`. Todos los chats de Telegram hacia el mismo agente comparten una `SessionId`, una `Session`, un historial en disco.

Uso en kernel: `crates/librefang-kernel/src/kernel.rs:4082-4088` y `:5214-5222`.

`SenderContext.chat_id` ya está propagado desde el adaptador Telegram (`crates/librefang-channels/src/telegram.rs:2571-2585`) pero **nunca se usa en la clave**.

**Efecto en producción:** un mensaje nuevo de cualquier chat extiende el historial compartido. Cuando el LLM del chat A produce su respuesta, lo hace sobre un historial contaminado por el chat B. Incluso si el outbound routing es correcto (`message.sender.chat_id`), el contenido de la respuesta es incoherente.

## Problema 2 — NO_REPLY leak

**Root cause (confirmado):** en `crates/librefang-api/src/channel_bridge.rs`, dos métodos ignoran `result.silent`:

- `send_message_with_sender` (línea 551) devuelve `Ok(result.response)` sin guard.
- `send_message_with_blocks_and_sender` (línea 578) igual.

Los tres métodos hermanos (`send_message:471`, `send_message_with_blocks:502`, `send_message_ephemeral:591`) sí aplican `if result.silent { Ok(String::new()) } else { Ok(result.response) }`.

`is_no_reply` en `crates/librefang-runtime/src/agent_loop.rs:77-85` detecta correctamente ambos centinelas y pone `silent: true`. El bug es puramente del adaptador: tira el flag.

---

## Fase 1 — Deploy esta semana

Dos fixes, un solo worktree limpio, cross-compile aarch64, deploy al Pi como `librefang.new`. Parar para swap manual.

### 1A · Per-chat session key

Cambio en `crates/librefang-kernel/src/kernel.rs` en las dos llamadas a `SessionId::for_channel`:

```rust
Some(ctx) if !ctx.channel.is_empty() => {
    let scope = match &ctx.chat_id {
        Some(cid) if !cid.is_empty() => format!("{}:{}", ctx.channel, cid),
        _ => ctx.channel.clone(),
    };
    SessionId::for_channel(agent_id, &scope)
}
```

No hace falta tocar `SessionId::for_channel` — sigue aceptando `&str`. La clave nueva es `"{agent}:telegram:-4597343960"` para el grupo rodela y `"{agent}:telegram:<dm_chat_id>"` para el DM. Son `UUID v5` distintos, sesiones físicamente separadas en disco.

**Antes de declarar "fix hecho" — probe de estado concurrente compartido.** Verificar que no hay campos mutables en la struct `Agent` o en `AgentManager` que atesoren `last_chat_id` / `current_session_id` compartidos entre turnos. Subtarea 30 min: grep por `last_chat`, `current_session`, `Mutex<.*Chat`, `RwLock<.*Session` en `crates/librefang-kernel/src/` y `crates/librefang-runtime/src/`. Si aparece algo sospechoso, documentarlo antes de tocar nada — puede que el bug tenga una segunda cara que el session key no cubre.

### 1B · NO_REPLY filter

Dos líneas en `crates/librefang-api/src/channel_bridge.rs`. Sustituir en las líneas 551 y 578:

```rust
Ok(result.response)
```

por:

```rust
if result.silent { Ok(String::new()) } else { Ok(result.response) }
```

Copia literal del patrón que ya usan los otros tres métodos.

### Migración del historial viejo

Al cambiar la clave, la `Session` antigua monolítica queda huérfana — el binario nuevo no la encontrará. Dos opciones:

**Opción A — wipe.** La sesión antigua se ignora. Todos los chats arrancan vacíos en el Pi. **Ventaja:** simple, y el contenido antiguo está contaminado por ser un merge de contextos que nunca debieron mezclarse. **Desventaja:** rodela pierde la memoria de conversación (pero no la memoria de agente vía `memory_store`, que es distinta).

**Opción B — attribute-to-main.** Migrator de un solo shot que lee la sesión vieja y la escribe bajo la clave nueva `(fandangorodelo, telegram, -4597343960)`, asumiendo que el 99% del contenido pertenece al grupo principal. Los DMs arrancan vacíos. **Ventaja:** el grupo conserva historia. **Desventaja:** si por casualidad había contenido de DMs mezclado, queda etiquetado como grupo, que es justo el problema que estamos arreglando.

**Recomendación: Opción A.** Es más limpia, el "reset" de historial es precisamente la consecuencia correcta de descubrir que el contexto estaba contaminado. El usuario puede decidir lo contrario si le importa conservar el hilo del grupo.

### Rollback path

Si Fase 1 rompe algo en el Pi:

1. Guardar una copia del `librefang` binario actual antes del swap (`cp librefang librefang.pre-1a`).
2. Si hay que volver atrás: `mv librefang.pre-1a librefang && systemctl --user restart librefang`.
3. Sesión vieja seguirá existiendo en disco, pero probablemente el binario viejo tampoco la verá porque la habrá dejado de usar. Rollback ≈ pérdida total de contexto igualmente.

**Implicación:** la rollback safety de Fase 1 es débil. Si algo va mal, no hay marcha atrás limpia — solo volver a un estado vacío. Aceptar conscientemente antes de desplegar.

### Tests (unit)

- `crates/librefang-types/src/agent.rs` (bloque existente `test_session_id_for_channel_*` alrededor de líneas 1708-1758): añadir tres tests nuevos:
  - `test_session_id_distinct_per_chat`: `(A, "telegram", "c1")` != `(A, "telegram", "c2")`.
  - `test_session_id_deterministic_with_chat`: misma tripla → mismo UUID.
  - `test_session_id_legacy_channel_only`: `(A, "telegram")` sin chat_id sigue siendo estable y != `(A, "telegram", "c1")`.
- `crates/librefang-api/src/channel_bridge.rs`: test en el módulo existente (si hay) o nuevo, que confirme que `send_message_with_sender` devuelve `Ok("")` cuando el `AgentLoopResult` tiene `silent: true` y `response: "NO_REPLY"`.

Si no existe módulo de test en channel_bridge, crear uno pequeño al final del fichero.

### Build y deploy

```
cargo check -p librefang-types -p librefang-kernel -p librefang-api --lib
cargo test -p librefang-types --lib test_session_id
cargo test -p librefang-api --lib channel_bridge
RUSTFLAGS="-D warnings" cargo check --workspace --lib
```

Cross-compile en worktree limpio derivado de `origin/main` reciente:

```
cd /tmp/librefang-rpi-build  # (ya existe del ciclo anterior)
# overlay de los dos ficheros con los cambios de Fase 1
cross build --release --target aarch64-unknown-linux-gnu -p librefang-cli
scp target/aarch64-unknown-linux-gnu/release/librefang blitz@192.168.1.144:/home/blitz/.librefang/bin/librefang.new
```

**Parar.** El usuario hace `mv librefang.new librefang && systemctl --user restart librefang`.

### Verificación live en el Pi

1. Desde el grupo rodela: "di hola A". Respuesta llega al grupo.
2. Desde DM: "di hola B". Respuesta llega al DM.
3. Volver al grupo: "qué te dije?". Respuesta menciona "hola A", **no** "hola B".
4. Volver al DM: "qué te dije?". Respuesta menciona "hola B", **no** "hola A".
5. Intentar provocar NO_REPLY: mensaje en el grupo claramente dirigido a otras personas. El bot debe **no** responder nada. Logs: `silent=true, response_len=0` (o `response_len=N` con silent=true, que también vale).
6. Grep logs por "NO_REPLY" y "[no reply needed]" — cero hits en líneas de envío a Telegram.
7. Sin 400 errors, sin reset loops.

### Upstream

Fase 1 es un candidato natural de PR upstream: son dos fixes estructurales pequeños, un solo commit claro (`fix(runtime): scope telegram sessions per-chat and suppress NO_REPLY sentinel`). Abrir PR **después** de verificar en el Pi. 5-6 ficheros tocados máximo (dos para el fix + tests).

---

## Fase 2 — Memoria selectiva per-chat (mucho más pequeña de lo que pensaba)

Arranca **solo si Fase 1 está verificada en el Pi y el usuario confirma que el aislamiento base funciona**.

### Base existente (PR #2015 / #1978)

Upstream ya tiene plumbing para scope per-peer: cuando `sender_id` está presente, las claves de memoria se namespacean como `peer:{peer_id}:{key}`. El cambio fue pequeño (~180 líneas en 9 ficheros) y funciona así:

- `KernelHandle::memory_store(key, value, peer_id)` propaga `peer_id` desde el tool runner.
- `tool_runner.rs::tool_memory_store` recibe `sender_id` (mismo `peer_id`) del `ExecCtx`.
- `crates/librefang-memory/src/semantic.rs` y `proactive.rs` concatenan el prefijo cuando `peer_id.is_some()`.
- Sin `peer_id` (API, CLI, WASM) el comportamiento es global clásico.

Así que la plumbing de "prefijo opcional en la clave" ya existe, y lo único que hace falta para el modelo per-chat es **añadir una segunda dimensión** que, cuando esté presente, se superponga al scope per-peer existente.

### Gap que queda (qué hay que añadir)

El usuario no quiere per-*peer* (que es lo que existe); quiere per-*chat*. Son cosas distintas:

- Per-peer (existente) aísla por **usuario**. Si Pakman habla en el grupo rodela y también en DM, ambos chats comparten sus memorias porque es el mismo `sender_id`.
- Per-chat (lo que se pide) aísla por **conversación**. Las memorias guardadas desde el grupo rodela son visibles a todos los miembros del grupo pero no desde los DMs — y viceversa.

Dos estrategias posibles:

**Estrategia A — reemplazar per-peer con per-chat por default en canales.** Dado que el usuario considera el *chat* como la unidad de privacidad, no el *peer*, se puede cambiar el prefijo por defecto de `peer:{peer_id}:` a `chat:{chat_id}:` en invocaciones desde canales. Per-peer queda como opt-in vía parámetro explícito.

**Estrategia B — añadir un tercer scope.** Mantener per-peer como está (PR #2015), añadir per-chat como nuevo scope opt-in via parámetro `scope` en la tool. El LLM elige entre `"global"`, `"peer"` (default, comportamiento PR #2015), `"chat"`.

**Recomendación: Estrategia B.** Es más incremental, no rompe lo que ya funciona, y da control fino al LLM. Además se mapea directamente al modelo mental que el usuario describió: "cosas visibles solo desde un chat concreto y otras pueden ser globales".

### Implementación de Estrategia B

1. **Extender `tool_memory_store`** en `crates/librefang-runtime/src/tool_runner.rs:2398-2408`: aceptar parámetro opcional `scope: "global" | "peer" | "chat"` (default `"peer"` para mantener el comportamiento actual PR #2015). Cuando `"global"`, saltar cualquier prefijo. Cuando `"chat"`, usar `chat:{chat_id}:{key}` en vez de `peer:...`.

2. **Propagar `chat_id`** desde el `ExecCtx` del runtime hasta el tool runner — ya debería estar disponible porque Fase 1A lo añade al `SessionId`. Subtarea: verificar que `ExecCtx` tiene (o recibe) `chat_id` tras Fase 1A. Si no, añadirlo ahí.

3. **Capa de memoria — reusar o extender el helper `peer_scoped_key`** que PR #2015 introdujo en `semantic.rs` y `proactive.rs`. Añadir un helper hermano `chat_scoped_key(chat_id, key) -> String` con formato `chat:{chat_id}:{key}`. La elección entre prefijos la hace la capa de llamada (`tool_memory_store`) — la capa de memoria solo aplica lo que se le pasa.

4. **`KernelHandle::memory_store` signature:** en vez de cambiar la firma existente (rompe el trait), añadir una variante nueva `memory_store_with_scope(key, value, peer_id, chat_id, scope)` que es la que usa el tool runner. La vieja sigue funcionando para callers que no necesitan scope-chat.

5. **Guidance en `prompt_builder.rs`:** añadir un fragmento que explique al LLM la diferencia entre `global`, `peer`, `chat` con ejemplos. Condicional al flag `memory.chat_scoping_enabled`.

6. **Feature flag:** `kernel_config.memory.chat_scoping_enabled: bool`. Default `false` upstream, `true` en rodela. Con flag off, el parámetro `scope="chat"` se degrada a `scope="peer"` (comportamiento existente), así que no rompe nada.

### Nota sobre `memory_list`

PR #2015 hizo `memory_list(peer_id)` para listar solo las claves del peer. Para Fase 2, añadir un nuevo endpoint `memory_list_by_scope(scope, peer_id, chat_id)` que filtra por el prefijo correspondiente. No modificar la signature existente.

### Tests Fase 2

- `scope="chat"` en chat A → recall desde chat A con scope `chat` → hit.
- `scope="chat"` en chat A → recall desde chat B → miss (chat distinto).
- `scope="global"` en chat A → recall desde chat B (scope global) → hit.
- `scope="peer"` en chat A, peer Pakman → recall desde chat B, peer Pakman → hit (peer match).
- `scope="peer"` en chat A, peer Pakman → recall desde chat B, peer María → miss (peer mismatch).
- Flag `chat_scoping_enabled=false` → `scope="chat"` se comporta como `scope="peer"`. Sin romper nada.

### Verificación en el Pi (Fase 2)

1. DM con Pakman: "recuerda que mi color favorito es verde, solo en este chat" → el LLM guarda con `scope="chat"`.
2. Grupo con Pakman y otros: "cuál es mi color favorito?" → responde "no lo sé" (memoria scoped al DM, no visible en el grupo).
3. DM con Pakman: "cuál es mi color favorito?" → responde "verde".
4. DM: "recuerda globalmente que Finlandia tiene 150k lagos" → guarda con `scope="global"`.
5. Grupo: "cuántos lagos tiene Finlandia?" → responde "150k".
6. DM con Pakman: "recuerda que tengo 35 años, solo yo" → guarda con `scope="peer"`.
7. Grupo con Pakman: "cuántos años tengo?" → responde "35" (scope peer, mismo user).
8. Grupo con María (distinto user): "cuántos años tiene Pakman?" → responde "no lo sé" (peer mismatch).

### Upstream

Estrategia B es un **follow-up limpio a #1978/#2015**. La plumbing ya existe, solo se añade un scope más. Podría ir upstream con el flag default-off. **Recomiendo abrir upstream con título "feat(memory): chat-scoped memory as third option alongside peer-scoped (#1978)"** y referenciar explícitamente #1978 como base. houko probablemente lo acepta si está bien presentado — es una extensión natural.

---

## Fase 3 — Cross-chat access bajo demanda (R&D, no implementable aún)

**Esta fase no puede empezar hasta resolver prerrequisitos de exploración.** Lo que sigue es análisis, no plan ejecutable.

### Por qué no es trivial (9 dimensiones)

#### A — Propiedad del chat
Un DM tiene un dueño claro (el usuario). Un grupo tiene N miembros, admins, el bot. No existe un único "owner" que pueda autorizar liberar el historial del grupo sin atropellar la privacidad de los demás. Cualquier diseño tipo "pide permiso al dueño" es inadecuado para grupos.

**Modelo correcto:** la tool sólo devuelve mensajes escritos por **el mismo usuario humano** que está solicitando la lectura. El usuario se autoriza a sí mismo para ver su propio contenido cruzando chats. Nunca lee lo que otros dijeron en el grupo.

#### B — Identidad humana persistente
Requiere que el sistema tenga un `user_id` estable que cruce chats. Telegram tiene `from.id` por mensaje. Librefang probablemente lo propaga en `SenderContext` pero **falta confirmar cómo y dónde**. Sin eso, no se puede implementar el modelo de Dimensión A. **Prerrequisito 1.**

#### C — Forma del dato
Nunca devolver mensajes crudos (`tool_use`, `system`, sub-agent interactions — ruido y riesgo). Devolver extractos ranked por una query: `session_history_search(query, limit=3) -> Vec<Extract>` donde cada extracto tiene `{source_chat_label, timestamp, snippet_<=200w, score}`.

#### D — Ephemeral vs persistente
Los extractos cargados en un turno **no deben entrar en `session.messages`** del chat destino. Viven solo durante la construcción del prompt de ese turno. El LLM los ve, responde, y al guardar la sesión del chat destino no se replica el contenido. Si el LLM quiere persistir algo relevante, lo hace explícitamente con `memory_store(scope: chat)` — acto consciente.

#### E — Provenance tag
Cada extracto cargado lleva metadata visible al LLM: `{origen_chat, origen_timestamp, origen_user}`. El system prompt instruye al LLM a no citar el origen literal si no es apropiado. Sirve también para auditoría.

#### F — Transitividad
Chat A carga extracto de chat B → extracto es ephemeral en A → si chat C luego consulta chats del mismo usuario, sólo ve mensajes en A y en C, no el extracto cargado. La barrera se mantiene por construcción (D).

#### G — Audit trail
Cada invocación se loguea: `{user_id, from_chat, to_chat, query, timestamp, num_hits}`. Sin el contenido. Telemetría ya existente en `librefang-telemetry` es suficiente; se añade un event type.

#### H — Forget cascade
Fuera de scope de Fase 3. Si el usuario dice "olvida X", no se cascadea a extractos persistidos. Como son ephemerales por default (D), el problema es pequeño. Si en Fase 2 el LLM guardó algo con `scope=chat`, el "forget" tiene que borrarlo de la memoria scoped, que es una operación ya soportada por la tool `memory_delete` (si existe) — a verificar.

#### I — Consentimiento por invocación
Por defecto, antes de leer otro chat, el bot pregunta en el chat actual: *"¿puedo consultar tus mensajes anteriores en el grupo rodela para responder a esto?"* y espera confirmación humana. Sin respuesta en 60s → cancela. Allowlist opcional en config para pares `(user, from, to)` de confianza total.

### Prerrequisitos de exploración (bloquean Fase 3)

1. **User_id persistente.** ¿Dónde vive `user_id` en `SenderContext`? ¿Se guarda en la `Session`? ¿Se puede indexar por `(user_id, chat_id)`? Grep objetivo: `user_id`, `sender.user`, `ChannelUser`, `from_user` en `crates/librefang-channels` y `crates/librefang-api`.

2. **Índice de búsqueda transversal.** ¿La capa `librefang-memory::session` indexa mensajes o es un append-log por `SessionId`? Un search cruzado requiere poder filtrar por `user_id` across sessions. Puede requerir añadir un índice secundario.

3. **Consentimiento in-chat.** ¿El approval system (`crates/librefang-api/src/routes/approvals.rs`) soporta enviar una pregunta por Telegram y esperar una respuesta del usuario en el mismo chat? Si no, hay que añadir ese flujo o usar un mecanismo distinto (e.g. mensaje con botones inline de Telegram).

Hasta resolver los tres, Fase 3 es R&D puro. Una exploración dedicada de 2-3h puede cerrarlos. **No empezar a implementar** hasta entonces.

### Upstream
Probablemente **no**. Este es territorio de fork local — es comportamiento específico del despliegue rodela. El modelo de privacidad depende de asunciones sobre Telegram y sobre el rol del usuario admin. No aplica genéricamente a otros canales (email, Slack, etc.).

---

## Fase 4 — Per-chat hand activation

Menor riesgo pero requiere cambiar una estructura de datos del kernel.

**Hoy:** `hand_activate` / `hand_deactivate` mantienen un `HashMap<AgentId, HashSet<HandId>>` (hipótesis, no verificado al 100%). El estado es global por agente.

**Cambio:** convertir a `HashMap<(AgentId, ChatId), HashSet<HandId>>`. Cuando `chat_id` no está presente (CLI, cron, webhooks), usar un `ChatId::default()` o `"__system__"` para invocaciones non-channel.

**Feature flag:** `kernel_config.hands.per_chat_activation: bool`. Default `false` upstream, `true` rodela.

**Tests:**
- Activar hand en chat A no lo activa en chat B.
- `hand_status` en A no lista hands activas en B.
- Con flag off: comportamiento global (compat).

**Prerrequisito de exploración:** localizar dónde vive exactamente la lista de hands activas en kernel (grep `active_hands`, `hand_state`, `activate_hand`).

**Upstream:** posible como feature opt-in. Decisión posterior.

---

## Decisiones abiertas (a tomar después de Fase 1 en el Pi)

1. **Migración de historial:** opción A (wipe, limpio) o B (attribute-al-grupo, preserva la mayoría). Recomendado A.
2. **Fase 2 tool:** un parámetro `scope` o dos tools separadas (`memory_store_chat` / `memory_store_global`)? Parámetro es más limpio; dos tools es más explícito y el LLM rara vez las confunde.
3. **Fase 3 timing:** ¿hacer la exploración de prerrequisitos en la siguiente sesión, o ignorar Fase 3 hasta que el usuario lo pida explícitamente?
4. **Fase 4 timing:** ¿cuánto duele hoy la activación global de hands? Si nadie lo siente, esperamos; si es molesto, se prioriza.
5. **Upstream de Fase 1:** ¿abrimos PR upstream el mismo día que desplegamos al Pi, o esperamos 24-48h de observación?
6. **Upstream de Fases 2-4:** por defecto se quedan en rodela según tu regla. Confirmar que eso sigue vigente.

---

## Upstream vs rodela fork

- **Fase 1 (bugs 1+2):** upstream PR.
- **Fase 2 (memory scoping):** probablemente rodela first; candidato upstream con flag si se prueba bien.
- **Fase 3 (cross-chat):** rodela only. Demasiado específico de la configuración del despliegue.
- **Fase 4 (per-chat hands):** rodela first; candidato upstream con flag.

---

## Recomendación

**Arrancar por Fase 1 sola.** Es:

- Pequeña (dos files efectivos + tests)
- Aislada (no depende de decisiones no tomadas)
- Testeable en el Pi en 5 minutos
- Cierra estructuralmente el bug 1 (el más doloroso observado)
- Cierra del todo el bug 2

**No** mezclar Fase 1 con Fases 2-4 en el mismo push. Fase 1 es la que debe verificarse en aislamiento para poder responsabilizarla de forma limpia si algo va mal. Después del éxito de Fase 1, se toman decisiones sobre Fases siguientes.

Antes de tocar código: ejecutar la subtarea de 30 min de probe de estado mutable compartido (ver Fase 1A). Si encuentro algo, pivotar; si no, seguir como planeado.
