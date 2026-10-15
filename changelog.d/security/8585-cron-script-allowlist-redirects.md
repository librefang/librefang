Cron `pre_check_script` now has to resolve under `<home>/scripts/`, the same allowlist `pre_script` already had, so an agent's `cron_create` can no longer schedule an arbitrary executable to run as the daemon.
The check runs at create, update and fire time; a job persisted before it, or edited by hand, is refused at fire time and wakes the agent instead of running the script.
Cron webhook deliveries no longer follow redirects, so an allowed endpoint cannot bounce the payload to an internal address, and a non-2xx legacy webhook response is now logged as a failed delivery. (#8585) (@houko)
