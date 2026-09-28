"""Multi-competition worlds: medium/standard worlds run with real leagues, and
the managed club actually plays matches (not a fake single mega-league)."""

from conftest import frompitch2board, parse_blocks


def test_medium_world_runs():
    r = frompitch2board("score", "--seeds", "42", "--days", "60", "--world", "medium", "--club", "15")
    assert r.returncode == 0, r.stderr
    blocks = parse_blocks(r.stdout)
    assert blocks, "no output blocks"
    # The club actually played matches: the reference points > 0.
    assert blocks[0]["rows"]["points"][1] > 0.0


def test_compact_world_runs():
    r = frompitch2board("score", "--seeds", "42", "--days", "100", "--world", "compact", "--club", "0")
    assert r.returncode == 0, r.stderr
    assert parse_blocks(r.stdout)
