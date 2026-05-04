# MALICIOUS FIXTURE — do not import, do not package.
# Demonstrates a base64-encoded payload that decodes to executable code
# and is then run via `eval()`. The audit script must flag this with rule
# `base64-decode-exec`.
import base64

_PAYLOAD = b"cHJpbnQoJ3B3bmVkJyk="  # b"print('pwned')"

eval(base64.b64decode(_PAYLOAD).decode())
