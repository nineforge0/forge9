# Key families in the executable

The executable contains a table of 22 key schedules in `.rdata`. This page
records where the table is, how the keys are grouped into families, and why
the archive header `key_id` cannot be used to pick a key. All statements were
checked against the files named below; see [Verification](#verification).

All integers are little-endian. Offsets are file offsets unless stated.

## Contents

1. [Key table in the executable](#key-table-in-the-executable)
2. [Family distribution](#family-distribution)
3. [Key selection](#key-selection)
4. [Header layout (corrected)](#header-layout-corrected)
5. [key_id is not a key index](#key_id-is-not-a-key-index)
6. [Multi-part PSIN entries](#multi-part-psin-entries)
7. [Corpus structure (packs.rpf)](#corpus-structure-packsrpf)
8. [Verification](#verification)

## Key table in the executable

| Property | Value |
|---|---|
| File | Xbox v1.0.2.1 executable (115 MB, PE32+ x64) |
| Table start | file offset `0x066e4940`, in `.rdata` |
| Entry count | 22 key schedule blobs |
| Blob size | `0x10F4` bytes (`KEY_BLOB_SIZE`); first 4 bytes are the family magic |
| Stride | `0x1100` (12 zero bytes between consecutive blobs) |

Exception: after key 0 there are 44 bytes instead of 12, namely the usual
12 zero bytes followed by 32 bytes of data:

```
ebefa94f0627b8ea0a69d82aa5e23ef155fa8e9ede97e002a43f4295b9767d40
```

The meaning of these 32 bytes is not established.

## Family distribution

| Family | Count | Indices | Notes |
|---|---|---|---|
| `8E9B3B4E` | 9 | 0, 2-9 | primary; previously the only known set |
| `7E533141` | 1 | 1 | sits between primary keys 0 and 2 |
| `5DCE1827` | 8 | 10-17 | secondary batch |
| `504F865E` | 1 | 18 | |
| `44AB246F` | 1 | 19 | |
| `FB4BF43F` | 1 | 20 | |
| `AA38AA57` | 1 | 21 | |

All 7 families share the same T-tables and cipher algorithm (13 rounds,
T-table CBC). They differ in the key schedule content.

The 9 primary-family blobs are byte-identical to the previous `keys.bin`.
No archive in the available corpus uses keys 10-21.

## Key selection

`Archive::open` tries every key and picks the one that recovers the most
table records. Brute force is the only correct approach, because the header
`key_id` does not identify a key (next sections).

## Header layout (corrected)

Offsets from the start of the file, per `src/rpf9/header.rs`.

| Offset | Size | Field | Observed values |
|---|---|---|---|
| `0x00` | 4 | magic | `9FPR` (`0x52504639`) |
| `0x04` | 4 | payload_start | varies |
| `0x08` | 4 | split | varies |
| `0x0C` | 4 | trailer_size | 64 to 22640 bytes observed |
| `0x10` | 8 | archive_id | u64 |
| `0x18` | 4 | gen_word | `0x47a00` in all top-level archives |
| `0x1C` | 4 | gen_const | `0x150` in all top-level archives |
| `0x20` | 2 | version_low | 3 to 7 observed |
| `0x22` | 2 | version_high | always 1 |
| `0x24` | 4 | entry_count | declared number of entries |
| `0x28` | 4 | key_id | opaque; NOT a key index |
| `0x2C` | 4 | flags | low byte = entry stride offset |
| `0x30` | 4 | alignment | `0x10` |
| `0x34` | 12 | reserved | zeros |

## key_id is not a key index

All 12 top-level archives were tested. The same `key_id` maps to different
brute-forced key indices:

| key_id | Brute-forced key (archive) |
|---|---|
| `0xda72573e` | #5 (appdata0, movies), #9 (common), #6 (shaders) |
| `0xd98004d5` | #9 (anim), #4 (navmeshes), #5 (packs) |
| `0xc0a92754` | #5 (script), failed with timeout (audio) |
| `0xd5c19f6b` | #2 (data), #7 (textures) |

## Multi-part PSIN entries

Large PSIN entries in `packs.rpf` declare a `decoded_size` larger than the
sum of their inflated zlib parts. Example: entry 5 declares `0xae0188` but
inflates to `0x960000`. Rejecting only when declared < actual (instead of
requiring equality) unlocks 7227 texture variation records from the ped
metadata PSIN.

## Corpus structure (packs.rpf)

- 172 top-level entries, many of them nested RPFs.
- Pack #97: 3724 entries; the main ped/character drawable pack (214 MB
  nested RPF).
- Entry 5 (top-level, 2.6 MB PSIN, zlib): 7227 texture variation records
  with diffuse/normal TTPF store keys.
- Entries 7-17 (smaller PSINs): block tables without part tables or
  variations; likely non-ped metadata.
- All nested RPFs use keys 0-9 (primary family); none needed keys 10-21.

## Verification

- 11 existing tests pass with the 22-key `keys.bin`.
- All 12 top-level archives in the corpus open.
- All nested archives in `data.rpf` and `packs.rpf` open.
- Keys 10-21 may protect content absent from the available build, or nested
  archives not yet fully scanned.
