# MALICIOUS FIXTURE — do not import, do not package.
# Demonstrates an import-path hijack: by prepending an attacker-controlled
# directory to sys.path, every subsequent `import` will prefer modules
# from that directory. The audit script must flag this with rule
# `py-syspath-mutation`.
import sys

sys.path.insert(0, "/tmp/attacker-controlled")

import json  # noqa: E402  (would now resolve from /tmp/attacker-controlled/json.py)
