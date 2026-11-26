#!/usr/bin/env bash
# forge9 discovery pipeline -- run on Mac mini with game files
# Generates first-publication findings that nobody else can produce
set -euo pipefail

FORGE9="${FORGE9:-cargo run --release --}"
DATA="${GAME_DIR:?set GAME_DIR to the directory containing the rpf files}"
OUT="${OUT_DIR:-./discovery-$(date +%Y%m%d)}"
mkdir -p "$OUT"

echo "=== forge9 discovery pipeline ==="
echo "output: $OUT"
echo ""

# 0. Build release first
cargo build --release

# 1. levels.rpf -- the big one (stride fix test)
echo "--- 1. levels.rpf header (stride fix test) ---"
$FORGE9 info "$DATA/levels.rpf" > "$OUT/levels-info.txt" 2>&1 || true
cat "$OUT/levels-info.txt"
echo ""

# 2. Full catalog of accessible RPFs
echo "--- 2. Full catalog ---"
for rpf in "$DATA"/*.rpf; do
    name=$(basename "$rpf" .rpf)
    echo "  cataloging $name..."
    $FORGE9 catalog --json "$rpf" > "$OUT/catalog-${name}.json" 2>/dev/null || echo "  SKIP $name"
done
echo ""

# 3. Hash harvest across all RPFs
echo "--- 3. Hash harvest ---"
$FORGE9 harvest "$DATA"/*.rpf > "$OUT/harvest.txt" 2>"$OUT/harvest-stats.txt" || true
wc -l "$OUT/harvest.txt"
cat "$OUT/harvest-stats.txt"
echo ""

# 4. PSIN XML dump from script.rpf (all 1012 DOM trees)
echo "--- 4. PSIN DOM trees from script.rpf ---"
# script.rpf has nested RPFs; entry 0 of the inner archive has the DOM trees
# Try each potential index
for idx in 0 1 2; do
    $FORGE9 psin "$DATA/script.rpf" "$idx" --xml > "$OUT/script-psin-${idx}.xml" 2>/dev/null || true
done
wc -l "$OUT"/script-psin-*.xml 2>/dev/null || true
echo ""

# 5. Object graph from script.rpf
echo "--- 5. Object graph analysis ---"
for idx in 0 1 2; do
    $FORGE9 object-graph "$DATA/script.rpf" "$idx" --census --json > "$OUT/objgraph-${idx}.json" 2>/dev/null || true
done
echo ""

# 6. Network/online/session string search
echo "--- 6. Network string search ---"
for term in session lobby network matchmak replicat sync player_list GamerTag squad online multiplayer; do
    echo "  searching: $term"
    $FORGE9 search --grep "$term" "$DATA/script.rpf" >> "$OUT/net-search.txt" 2>/dev/null || true
    $FORGE9 search --grep "$term" "$DATA/data.rpf" >> "$OUT/net-search.txt" 2>/dev/null || true
done
hits=$(wc -l < "$OUT/net-search.txt" || echo 0)
echo "  total hits: $hits"
echo ""

# 7. Ped variation database
echo "--- 7. Ped variation records ---"
$FORGE9 ped-meta "$DATA/packs.rpf" 5 > "$OUT/ped-variations.txt" 2>/dev/null || true
wc -l "$OUT/ped-variations.txt" 2>/dev/null || true
echo ""

# 8. Shader root parameter census
echo "--- 8. Shader census ---"
# Find SGD5 entries and run fssm on them
$FORGE9 search --magic SGD5 "$DATA/data.rpf" > "$OUT/sgd5-entries.txt" 2>/dev/null || true
echo "  SGD5 entries found: $(wc -l < "$OUT/sgd5-entries.txt")"
echo ""

# 9. packs.rpf structure map
echo "--- 9. packs.rpf structure ---"
$FORGE9 tree "$DATA/packs.rpf" > "$OUT/packs-tree.txt" 2>/dev/null || true
wc -l "$OUT/packs-tree.txt" 2>/dev/null || true
echo ""

echo "=== done ==="
echo "results in $OUT/"
ls -lh "$OUT/"
