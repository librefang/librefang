A Telegram sidecar could stay alive but stop receiving messages after a transient error.
The poll round-trip had no overall deadline: `urllib`'s timeout bounds one socket operation, not the whole response, so a 502 whose body never terminated could park the worker thread forever, with no log line and no process exit for the supervisor to notice.
`produce()` now wraps the poll in a 50 s deadline, distinguishes a normal long-poll timeout from a wedged thread, and escalates three consecutive wedges to a producer crash that exits the process with status 1 so the supervisor restarts the sidecar.
Each poll publishes its batch only if it is still the current one, so a poll abandoned at its deadline can no longer double-emit or move the offset backwards.
Repeated timeouts and long gaps without a successful poll are logged at WARN, rate-limited. (#8662) (@DaBlitzStein)
