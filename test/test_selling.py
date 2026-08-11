"""Selling dimension: transfer-listing a player attracts offers, and accepting
one completes a sale — the squad shrinks and cash comes in.

SellingManager transfer-lists its most valuable backups (Action::ListPlayer),
accepts offers at/above 0.9× market value, and does not buy. Its squad_size
must therefore be smaller than the reference's (sold players), which is the
stable cross-seed signal."""

from conftest import clubbench, parse_blocks


def _selling_block(stdout):
    for b in parse_blocks(stdout):
        if b["name"].startswith("Selling"):
            return b
    return None


def test_selling_trims_the_squad():
    r = clubbench("score", "--seeds", "42,43,44", "--days", "350", "--world", "compact", "--club", "15")
    assert r.returncode == 0, r.stderr
    block = _selling_block(r.stdout)
    assert block is not None, "Selling block not found"
    _, _, delta_size, _, _ = block["rows"]["squad_size"]
    # SellingManager sells backups → its squad is smaller than the reference.
    assert delta_size < 0


def test_selling_dimension_reported():
    r = clubbench("score", "--seeds", "42", "--days", "200", "--world", "compact", "--club", "15")
    assert r.returncode == 0, r.stderr
    block = _selling_block(r.stdout)
    assert block is not None
    assert "squad_size" in block["rows"]
