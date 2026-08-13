Command lane read/write lock recovery from a poisoned state is now logged with the affected lane, instead of recovering silently.
The lock's poison flag is cleared once the recovered state has been read out, so a single panic produces one diagnostic log line rather than a permanent per-access warning for the rest of the process (#7013) (@houko)
