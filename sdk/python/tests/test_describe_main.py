"""End-to-end test: `python -m <adapter> --describe` writes JSON to stdout."""
import io
import json
import sys
from contextlib import redirect_stdout

from librefang.sidecar import Field, Schema, describe_main


class _AdapterWithSchema:
    SCHEMA = Schema(
        name="dummy",
        display_name="Dummy",
        description="Test adapter",
        fields=[Field("DUMMY_KEY", "Key", "text", required=True)],
    )


def test_describe_main_prints_json_and_exits_zero():
    buf = io.StringIO()
    rc = 99
    with redirect_stdout(buf):
        rc = describe_main(_AdapterWithSchema())
    assert rc == 0
    payload = json.loads(buf.getvalue())
    assert payload["name"] == "dummy"
    assert payload["fields"][0]["key"] == "DUMMY_KEY"


def test_describe_main_missing_schema_exits_two():
    class NoSchema:
        pass
    buf = io.StringIO()
    rc = 99
    with redirect_stdout(buf):
        rc = describe_main(NoSchema())
    assert rc == 2
    # Empty stdout on failure — daemon parses stdout, must not feed it junk
    assert buf.getvalue() == ""


def test_run_stdio_main_describes_class_without_instantiation():
    """run_stdio_main accepts a class and runs --describe without ever
    calling __init__ — so adapters whose __init__ requires env vars
    can still be described."""
    import subprocess, sys, json
    # NoSchema__init__-raises is a class whose __init__ would crash, so if
    # run_stdio_main correctly avoids instantiation it must still describe
    # successfully when SCHEMA is class-level.
    script = (
        "from librefang.sidecar import Schema, Field, run_stdio_main\n"
        "class Crashy:\n"
        "    SCHEMA = Schema('crashy','C','test',[Field('K','K','text')])\n"
        "    def __init__(self):\n"
        "        raise SystemExit(99)\n"
        "import sys; sys.argv = ['x', '--describe']\n"
        "run_stdio_main(Crashy)\n"
    )
    out = subprocess.check_output([sys.executable, "-c", script])
    payload = json.loads(out)
    assert payload["name"] == "crashy"
