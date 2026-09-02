The plugin install and install-deps paths now share one Python interpreter and pip-argument resolver: `python3` is tried first, `python` second, and `--user` / `--break-system-packages` are omitted inside virtualenv or Conda environments, where pip rejects `--user` installs. (#7306) (@DaBlitzStein)

Interpreter availability probing runs on Tokio instead of a synchronous `std::process::Command` call, so the async install paths never block on a process spawn. (#7306) (@DaBlitzStein)
