#!/usr/bin/env bash
# Import the community hash database (~1.36M JOAAT hashes with names)
# into forge9's dictionary format.
#
# Source: https://github.com/NotHGM/gta6-hash-database
# Output: data/extended.dict (not committed; use with --dict data/extended.dict)
#
# Usage: ./examples/import_hashes.sh

set -euo pipefail
cd "$(dirname "$0")/.."

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT
BASE_URL="https://raw.githubusercontent.com/NotHGM/gta6-hash-database/main/data/community"

echo "Downloading 16 community hash shards..."
for x in 0 1 2 3 4 5 6 7 8 9 a b c d e f; do
  curl -sL "$BASE_URL/hashes_$x.tsv" -o "$TMPDIR/hashes_$x.tsv" &
done
wait
echo "Downloaded $(cat "$TMPDIR"/hashes_*.tsv | wc -l | tr -d ' ') lines"

echo "Downloading game_data.tsv..."
curl -sL "https://raw.githubusercontent.com/NotHGM/gta6-hash-database/main/data/game/game_data.tsv" \
  -o "$TMPDIR/game_data.tsv"

echo "Converting to forge9 dict format..."
python3 -I -c "
import sys, os

out = {}

# Load existing dict
base = 'data/codewalker.dict'
if os.path.exists(base):
    with open(base) as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith('#'):
                continue
            parts = line.split(None, 1)
            if len(parts) == 2:
                h = parts[0].lower().replace('0x', '')
                out[h] = parts[1]
    print(f'  Base dict: {len(out)} entries')

# Community shards: hash\tnames\tsources
tmpdir = sys.argv[1]
added = 0
for x in '0123456789abcdef':
    path = os.path.join(tmpdir, f'hashes_{x}.tsv')
    with open(path) as f:
        next(f)  # skip header
        for line in f:
            parts = line.strip().split('\t')
            if len(parts) >= 2 and parts[1]:
                h = parts[0].lower()
                name = parts[1].split(',')[0].strip()
                if h not in out and name:
                    out[h] = name
                    added += 1
print(f'  Community: +{added} new hashes')

# Game data: hash\tkind\tname\tother_names
gd = 0
path = os.path.join(tmpdir, 'game_data.tsv')
with open(path) as f:
    next(f)
    for line in f:
        parts = line.strip().split('\t')
        if len(parts) >= 3 and parts[2]:
            h = parts[0].lower()
            if h not in out:
                out[h] = parts[2].strip()
                gd += 1
print(f'  Game data: +{gd} new hashes')

# Write sorted
with open('data/extended.dict', 'w') as f:
    for h in sorted(out.keys()):
        f.write(f'0x{h} {out[h]}\n')
print(f'  Total: {len(out)} entries -> data/extended.dict')
" "$TMPDIR"

echo "Done. Use with: forge9 --dict data/extended.dict ..."
