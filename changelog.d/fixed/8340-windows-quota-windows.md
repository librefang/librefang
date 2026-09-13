Rolling resource windows no longer empty themselves on a host whose uptime is shorter than the window.
  The underflow guard in the scheduler fell back to "now", which drained each window on every read rather than keeping it, so `max_network_bytes_per_hour` never accumulated a spend to refuse on for the first hour after a boot — and the tool-call and token burst gates were equally blind for a daemon's first minute.
  (#8340) (@houko)
