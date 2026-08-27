"""Selling dimension: transfer-listing a player attracts offers, and accepting
one completes a sale — the squad shrinks and cash comes in.

SellingManager transfer-lists its most valuable backups (Action::ListPlayer),
accepts offers at/above 0.9× market value, and does not buy. Under the
target-range squad scoring (the design notes §1: healthy [22, 26]), selling too many
players pushes the squad *below* the healthy range and is penalised — the
anti-fire-sale rule. The stable cross-seed signal is a positive squad_size
distance vs the (healthy) reference."""

from conftest import clubbench, parse_blocks


def _selling_block(stdout):
    for b in parse_blocks(stdout):
        if b["name"].startswith("Selling"):
            return b
    return None


def test_selling_trims_toward_range():
    r = clubbench("score", "--seeds", "42,43,44", "--days", "350", "--world", "compact", "--club", "15")
    assert r.returncode == 0, r.stderr
    block = _selling_block(r.stdout)
    assert block is not None, "Selling block not found"
    cand_dist = block["rows"]["squad_size"][0]
    ref_dist = block["rows"]["squad_size"][1]
    # Selling sells backups → its squad sits further from the healthy range
    # than the reference's (fire-selling is penalised, not rewarded).
    assert cand_dist >= ref_dist


def test_selling_dimension_reported():
    r = clubbench("score", "--seeds", "42", "--days", "200", "--world", "compact", "--club", "15")
    assert r.returncode == 0, r.stderr
    block = _selling_block(r.stdout)
    assert block is not None
    assert "squad_size" in block["rows"]
