# Key families in the executable

The executable contains a table of 22 key schedules at a fixed offset in
`.rdata`. Each schedule is a 0x10F4-byte blob whose first four bytes are
the family magic. The blobs are padded to 0x1100 alignment in the table
(12 bytes of zeros between consecutive blobs, except after key 0 which
has an additional 32-byte trailer).

## Family distribution

| Family     | Count | Indices | Notes |
|------------|-------|---------|-------|
| 8E9B3B4E   | 9     | 0, 2-9  | primary; previously the only known set |
| 7E533141   | 1     | 1       | interleaved between primary 0 and 2 |
| 5DCE1827   | 8     | 10-17   | secondary batch |
| 504F865E   | 1     | 18      | |
| 44AB246F   | 1     | 19      | |
| FB4BF43F   | 1     | 20      | |
| AA38AA57   | 1     | 21      | |

All 7 families share the same T-tables and cipher algorithm (13 rounds,
CBC with the fixed IV). They differ only in round keys and output S-boxes.

## Key selection

`Archive::open` tries every key and picks the one that recovers the most
table records. The header `key_id` field (offset 0x28) does NOT index
into the key table and does NOT map deterministically to a key index;
archives with the same `key_id` can use different keys. The brute-force
approach is therefore necessary and correct.

## Header layout (updated)

| Offset | Size | Field | Notes |
|--------|------|-------|-------|
| 0x00   | 4    | magic | `9FPR` (0x52504639 LE) |
| 0x04   | 4    | payload_start | byte offset where encrypted TOC begins |
| 0x08   | 4    | split | |
| 0x0C   | 4    | trailer_size | bytes at the end of the archive |
| 0x10   | 8    | archive_id | u64 |
| 0x18   | 4    | gen_word | generation in upper 16 bits |
| 0x1C   | 4    | gen_const | 0x150 in all observed archives |
| 0x20   | 2    | version_low | |
| 0x22   | 2    | version_high | |
| 0x24   | 4    | entry_count | declared number of entries |
| 0x28   | 4    | key_id | opaque tag, NOT a key index |
| 0x2C   | 4    | flags | low byte used for entry stride |
| 0x30   | 4    | alignment | observed 0x10 |
| 0x34   | 12   | reserved | zeros |

## Verification

- 11 existing tests pass with the 22-key `keys.bin`.
- All 12 top-level archives in the corpus open.
- All nested archives in `data.rpf` and `packs.rpf` open.
- The 9 primary-family blobs are byte-identical to the previous `keys.bin`.
- No archive in the corpus was observed using keys 10-21 (secondary
  families). These keys may protect content not in the available build,
  or nested archives in the larger files not yet fully scanned.
