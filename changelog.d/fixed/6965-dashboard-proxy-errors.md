Restore Vite's default dev-server proxy error logging, which a custom logger and a set of no-op `error` handlers on the `/api` proxy, its outgoing request, and its incoming response were silently swallowing.
A backend that was down or unreachable during `npm run dev` produced no diagnostic output at all, making the failure look like a hang instead of a connection error.
The WebSocket (`ws: true`) and five-minute proxy timeout behavior are unchanged (#6965) (@houko)
