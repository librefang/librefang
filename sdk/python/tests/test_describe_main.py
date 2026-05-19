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
