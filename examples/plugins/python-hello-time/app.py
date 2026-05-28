"""
python-hello-time — Phase-6 plugin example exercising the `time`
host capability through the librefang:plugin world.

Calls `time.now()` to obtain the current Unix epoch (seconds), then
returns successfully. The integration test (Phase-6 C-007) verifies
that the component loads, `run` returns Ok, and the host time import
was reached.

Build:  cargo xtask plugins-rebuild python-hello-time
"""
from wit_world.imports import time as time_host


class WitWorld:
    def run(self) -> None:
        """
        Fetch the current wall-clock time via the host `time` import.
        Returns None (Ok) on success; raises Err(PluginError) on failure.
        The componentize-py runtime maps a plain `return` to Ok(()).
        """
        _ts: int = time_host.now()
        # Success — no side-effects required. The test asserts run() → Ok.
