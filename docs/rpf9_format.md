# RPF9 container format

The reference for the RPF9 archive container as forge9 reads and writes it.
Every statement here is backed by forge9's code (cited by file), by a test that
builds its bytes in memory, or by a finding recorded in another forge9 document
or commit. Each section separates what is **KNOWN** (implemented, and checked on
real archives or by a byte-identical round trip) from what is **UNKNOWN** (not
parsed, not explained, or only inferred). Where forge9 relies on a heuristic,
this document says so.

This page covers the container only: header, encryption, table of contents and
entry storage. Payload formats (drawables, textures, shaders, mods) have their
own documents, listed in the [Format index](#format-index).

All integers are little-endian unless stated otherwise. "Unit" means 16 bytes:
offsets and sizes in the TOC are stored as counts of 16-byte units.

## Contents

1. [Overview](#overview)
2. [Header](#header)
3. [Encryption](#encryption)
4. [The TOC column system](#the-toc-column-system)
5. [Record layouts](#record-layouts)
6. [Entry names and recovery](#entry-names-and-recovery)
7. [Compression and nesting](#compression-and-nesting)
8. [Writing archives](#writing-archives)
9. [Integrity and signatures](#integrity-and-signatures)
10. [What is still unknown](#what-is-still-unknown)
11. [Format index](#format-index)

## Overview

An RPF9 file is four regions, in file order:

| Region | File range | Content |
|---|---|---|
| Header | `0x00 .. 0x40` | 64 bytes, plaintext ([Header](#header)) |
| TOC | `0x40 .. payload_start` | Encrypted with a block cipher in CBC mode ([Encryption](#encryption)); decrypts to a column directory and its columns ([TOC column system](#the-toc-column-system)) |
| Data | `payload_start .. file_len - trailer_size` | Entry payloads at 16-byte-unit offsets, raw or as zlib chains ([Compression and nesting](#compression-and-nesting)) |
| Trailer | last `trailer_size` bytes | Not parsed; copied verbatim on repack |

Entries are addressed by **record index**, the position of their record in
the TOC. An entry has no stored path or string name; its name exists only as a
32-bit hash in a separate column ([Entry names](#entry-names-and-recovery)).
An entry can itself be an RPF9, so archives nest; forge9 addresses a nested
entry as `a/b` (entry `b` of the archive stored as entry `a`).

Code: `src/rpf9/header.rs`, `crypto.rs`, `entry.rs`, `reader.rs`, `writer.rs`,
`hashes.rs`.

## Header

Fixed 64 bytes (`HEADER_SIZE = 0x40`), parsed by `Header::parse` and written
back by `Header::to_bytes` (`src/rpf9/header.rs`). `forge9 info` prints every
field.

| Offset | Size | Field (forge9 name) | Status | Meaning |
|---|---|---|---|---|
| `0x00` | u32 | `magic` | KNOWN | `0x52504639`, the bytes `9FPR` on disk. The only check the parser makes; anything else fails with "bad magic". |
| `0x04` | u32 | `payload_start` | KNOWN | End of the encrypted TOC and start of the data region. The TOC ciphertext is `0x40 .. payload_start`. The reader rejects a value past the file end or `<= 0x40`. |
| `0x08` | u32 | `split` | UNKNOWN | Printed by `info`, used nowhere. Meaning not identified. |
| `0x0C` | u32 | `trailer_size` | KNOWN (layout) | Byte length of the trailer at the end of the file. The archive end used for entry bounds is `file_len - trailer_size`. The trailer's content is UNKNOWN; repack copies it unchanged. |
| `0x10` | u64 | `archive_id` | UNKNOWN | Read and preserved; semantics not identified (the name is descriptive only). |
| `0x18` | u32 | `gen_word` | PARTLY KNOWN | forge9 reports `gen_word >> 16` as the "generation". What the generation means, and what the low 16 bits hold, is UNKNOWN. |
| `0x1C` | u32 | `gen_const` | UNKNOWN | Read and preserved, used nowhere. |
| `0x20` | u16 | `version_low` | PARTLY KNOWN | Printed as the minor part of `version high.low`. Not checked. |
| `0x22` | u16 | `version_high` | PARTLY KNOWN | Printed as the major part. Not checked. |
| `0x24` | u32 | `entry_count` | KNOWN | Declared number of entries. Used as the target for record detection: a run of exactly `entry_count + 1` records means the first record is skipped, and `0` means "trust the detected run" (`reader.rs`). |
| `0x28` | u32 | `key_id` | UNKNOWN (mapping) | Printed, but **not** used to pick a key. forge9 finds the key by trial ([Key selection](#key-selection)). How `key_id` maps to a key slot, if it does, is not established. |
| `0x2C` | u32 | `flags` | UNKNOWN | `Header::entry_stride` reports `8 + (flags & 0xFF)` and `info` prints it as "stride", but no parser uses that value and it is not verified against the real record strides. The other bits are not identified. |
| `0x30` | u32 | `alignment` | UNKNOWN (use) | Read and preserved. forge9's writer does not use it: payload alignment on repack is inferred from each original offset ([Writing archives](#writing-archives)). |
| `0x34 .. 0x40` | 12 bytes | (none) | UNKNOWN | Not parsed. `Header::to_bytes` emits zeros here; repack does not use `to_bytes` and copies the original 64 bytes verbatim, so whatever this region holds is preserved. |

KNOWN about the header as a whole: it is plaintext, it is not covered by the
TOC encryption, and an unchanged copy of it is accepted by the independent
reader used in [repack_validation.md](repack_validation.md). The header is
signed ([Integrity and signatures](#integrity-and-signatures)).

## Encryption

Code: `src/rpf9/crypto.rs`.

### Cipher

KNOWN, implemented in both directions and proven by byte-identical round
trips:

- A 128-bit block cipher built from **lookup tables**: 16 input S-boxes, 16
  T-tables of 256 u32 words, and 16 per-key output S-boxes.
- Decryption (`KeySchedule::decrypt_block`): the 16 input bytes go through the
  16 input S-boxes, are packed big-endian into four words and XORed with the
  first round-key words; then 13 T-table rounds (each output word is the XOR of
  four T-table lookups, one byte from each input word, per `ROUND_TAPS`, plus a
  round-key word); then each output byte goes through its own per-key S-box.
  The schedule holds 60 round-key words (14 round keys including the initial
  whitening, as the module doc puts it).
- Encryption (`KeySchedule::encrypt_block`) is forge9's own inverse: each
  T-table is an affine map over GF(2) of a byte bijection, and the four tables
  feeding one output word span all 32 bits, so one round inverts by solving a
  32x32 linear system, precomputed as byte lookup tables (`InverseRound`). The
  tests check `decrypt_block(encrypt_block(p)) == p` for every key.
- **CBC mode** over the whole TOC region `0x40 .. payload_start`, with the fixed
  IV `TAG_IV` defined in `crypto.rs`. Every archive and every nested archive
  uses the same IV. The region is processed in whole 16-byte blocks.

UNKNOWN: whether this is a known cipher with whitened tables or a bespoke one.
It only matters for naming; forge9 needs only the tables.

### Keys and provenance

| File | Size | Layout | Status |
|---|---|---|---|
| `tables.bin` | `0x5000` (20,480) bytes | 16 input S-boxes (`0x100` each, at `0x0000`), then 16 T-tables (`0x400` each, at `0x1000`) | KNOWN |
| `keys.bin` | `n * 0x10F4` bytes | One blob per key: 4-byte family magic `8E 9B 3B 4E`, 60 round-key words (`0xF0` bytes), 16 output S-boxes (`0x1000` bytes) | KNOWN |

- The retail data uses **9 keys** (`keys.bin` = 39,060 bytes). The commit that
  added content-archive support records that the executable holds exactly 9
  blobs of the `8E9B3B4E` family, byte-identical to the ones forge9 uses; other
  key-shaped blobs in it decrypt nothing.
- `CryptoContext::from_bytes` accepts any number of blobs, as long as each one
  starts with the family magic, so a wider keyring needs no code change.
- **Provenance.** forge9 ships neither file and has no key material compiled
  in. The user extracts both from their own legally obtained copy and points
  forge9 at them (`data/`, `--data-dir` or `FORGE9_DATA`). See the README's
  "Crypto data" and "Scope and provenance" sections.

### Key selection

KNOWN behaviour, heuristic method (`Archive::open`, `reader.rs`): forge9
decrypts the TOC with **every** key and keeps the key whose plaintext yields
the longest run of valid records ([Record detection](#record-detection)). The
header's `key_id` is not consulted. Observed slots (from
[repack_validation.md](repack_validation.md)): slot 8 for `common.rpf`, slot 4
for `appdata0.rpf` and `script.rpf`, slot 5 for `shaders.rpf`. Nested content
archives decrypt with the same 9 keys.

`entry.rs` also defines a 16-byte marker, `GRAPH_MAGIC`: u64 `0x40` immediately
followed by the records column id `0x33f70dfaf2c50989`. The repack validation
harness uses its presence in the plaintext as evidence that the right key was
found. Why `0x40` precedes the id is UNKNOWN.

## The TOC column system

Code: `column`, `name_hashes`, `detect_compact` in `src/rpf9/entry.rs`.

KNOWN: the decrypted TOC is a **column store**. A directory of fixed 24-byte
descriptors names each column by a 64-bit id and says where it is:

| Descriptor offset | Size | Field |
|---|---|---|
| `+0x00` | u64 | column id |
| `+0x08` | u32 | count (number of elements) |
| `+0x0C` | u32 | offset of the column's first element, from a base shared by all columns |
| `+0x10` | u64 | stride (bytes per element) |

All offsets in one directory are relative to **the same base**. forge9 does
not know where the base is; it does not need to, because it locates one column
independently (the records, by scanning) and derives every other column from
the difference of the two descriptor offsets:
`other_at = records_at + other.offset - records.offset` (`name_hashes`).

Element `i` of every per-record column belongs to record `i`. A column may be
**shorter** than the record column: the records past its end have no value.

forge9 finds a descriptor by searching the plaintext for the 8-byte id. This
is a heuristic: an id could in principle also occur inside data.

### Known column ids

| Column id | Stride | Element | Status | Used by forge9 |
|---|---|---|---|---|
| `0x33f70dfaf2c50989` | 24 | Entry record, indexed or stored layout | KNOWN | Records are found by scanning, not via this descriptor; its offset anchors the name column. Also part of `GRAPH_MAGIC`. |
| `0xba180afb232d1b4b` | 20 | Entry record, compact layout (nested content archives) | KNOWN | `detect_compact` reads its count; records are then found by scanning. Anchors the name column for compact archives. |
| `0xb7076b6245d35fb5` | 4 | u32 joaat of the entry's name, one per record index | KNOWN | `name_hashes` reads it (only when the descriptor's stride is 4). |
| `0x73bcb72dce74f11f` | 8 | u64 per-record value, apparently a 64-bit hash | UNKNOWN (function) | Not read. The hash function that produces it is not identified. |
| `0xe0c12d7a5d4884c5` | 8 | 8-byte per-record value | UNKNOWN | Not read; seen in column directories, not parsed or checked by forge9 code. |
| `0xb6f5bdb2c66da2e9` | not recorded | not identified | UNKNOWN | Not read; seen in column directories, not parsed or checked by forge9 code. |

`rpf9-explorer`'s notes describe "six record tables" in the index
(see [repack_validation.md](repack_validation.md), "Limits"). Six column ids
are listed above, which is consistent with that count; the correspondence is
not verified.

## Record layouts

Three record layouts exist. Indexed and stored share the 24-byte stride of
column `0x33f70dfaf2c50989`; compact is the 20-byte column
`0xba180afb232d1b4b`. No layout carries the name hash: that lives in its own
column.

### Indexed record (24 bytes)

Used by most archives (`common.rpf`, `appdata0.rpf`, `shaders.rpf`). Decoded
by `decode` in `entry.rs`.

| Offset | Size | Field | Status | Meaning |
|---|---|---|---|---|
| `+0x00` | u32 | `size_hint` | PARTLY KNOWN | Kept as `Entry::size_hint`; not used to size the entry. Its exact meaning (declared decoded length, codec info or other) is UNKNOWN. |
| `+0x04` | u32 | (zero) | KNOWN | Must be 0 for the record to decode as indexed. |
| `+0x08` | u32 | offset / 16 | KNOWN | Payload byte offset in 16-byte units. Must be nonzero. The only word repack rewrites. |
| `+0x0C` | u32 | `flags` | UNKNOWN | Kept as `Entry::flags` and printed by `list --json`. Bit meanings not identified. |
| `+0x10` | u32 | `aux` | UNKNOWN | Kept as `Entry::aux`. |
| `+0x14` | u32 | (zero) | KNOWN | Must be 0. |

KNOWN: the indexed record stores **no explicit size**. forge9 sizes an entry as
the span to the next entry in physical (offset) order, and the last one up to
`file_len - trailer_size` (`assign_sizes`). A raw payload therefore reads back
with its zero padding to the next entry; zlib payloads end where the zlib chain
ends.

### Stored record (24 bytes)

Used where entries are nested RPF9s stored raw (for example `script.rpf`).

| Offset | Size | Field | Status | Meaning |
|---|---|---|---|---|
| `+0x00` | u64 | offset / 16 | KNOWN | Payload byte offset in 16-byte units. Nonzero. |
| `+0x08` | u64 | ceil(size / 16) | KNOWN | Size in units, rounded up. |
| `+0x10` | u64 | size | KNOWN | Exact payload size in bytes. Nonzero. |

KNOWN: the middle field is a built-in check. A record is read as stored when
`units != 0`, `size != 0` and `blocks == ceil(size / 16)`; otherwise it is
tried as indexed. Once an indexed record has flags set, it cannot satisfy this
check, which is how the two 24-byte layouts are told apart. Unit test:
`stored_layout_uses_explicit_offset_and_size`.

### Compact record (20 bytes)

Used by the nested RPF9s inside story and level content packs. Parsed by
`parse_compact`.

| Offset | Size | Field | Status | Meaning |
|---|---|---|---|---|
| `+0x00` | u32 | `size_hint` | PARTLY KNOWN | As in the indexed record. |
| `+0x04` | u32 | `flags` | UNKNOWN | Kept as `Entry::flags`. |
| `+0x08` | u32 | size / 16 | KNOWN | Stored span in 16-byte units, **including** the 16-byte prefix. |
| `+0x0C` | u32 | offset / 16 | KNOWN | Byte offset of the prefix, in units. |
| `+0x10` | u32 | `aux` | UNKNOWN | Kept as `Entry::aux`. |

KNOWN: **each compact payload starts with 16 zero bytes** that are not part of
the entry. The entry content is `offset * 16 + 16 .. (offset + size) * 16`, so
`Entry::offset = units * 16 + 16` and `Entry::size = blocks * 16 - 16`. What
the prefix is for is UNKNOWN. Unit test: `compact_layout_from_directory`.

### Record detection

forge9 does not locate the records through the column directory. It scans
(`detect`, `detect_compact`):

- **24-byte layouts:** at every 4-byte step of the plaintext, count the run of
  consecutive records that decode (stored or indexed) with an offset inside the
  file, up to `entry_count + 2`; keep the longest run. A run of exactly
  `entry_count + 1` means one leading record precedes the entries and is
  skipped. What that leading record is, is UNKNOWN.
- **Compact layout**, tried only when the 24-byte run falls short of
  `entry_count`: take `count` from the `0xba180afb232d1b4b` descriptor, then
  find the first 8-aligned start after the descriptor where `count` records in
  a row are valid: span of more than one unit, prefix at or after
  `payload_start`, inside the file, and a zero 16-byte prefix.

This works on every archive forge9 has opened, but it is a heuristic. Reading
the record offset straight from the descriptor, relative to the shared base,
would replace it once the base is identified.

## Entry names and recovery

Code: `src/rpf9/hashes.rs`, `name_hashes` in `entry.rs`, the `catalog`
command in `main.rs`.

KNOWN:

- Entries have no stored name string. Their name is the 32-bit **joaat**
  (Jenkins one-at-a-time) hash in column `0xb7076b6245d35fb5`, at the entry's
  record index. forge9's `joaat` hashes the bytes as given, with no case
  folding.
- A hash of `0` (the joaat of the empty string) is a placeholder, not a name;
  `Archive::open` leaves `Entry::name_hash` empty for it.
- Some archives have no name column, or a shorter one than their records.
- Evidence (commit `f432fba`): in a story content pack, 48 entries across 5
  nested archives match the joaat of strings found in sibling archives at
  exactly their record index, and the matched names are in **alphabetical
  order** by record index.

Recovery is dictionary-based: a 32-bit hash cannot be reversed, only matched.
`forge9 --dict names.txt` loads `hash name` lines on top of the small built-in
table (`KNOWN` in `hashes.rs`). `forge9 catalog` lists every entry, nested ones
included, as `archive:path` (plus `@hash` when the column has one) and names it
from, in order: the dictionary; else the embedded label of a drawable or
drawable dictionary (header `+0xA8`, see [drawable_format.md](drawable_format.md));
else candidate strings from the archives whose joaat matches. A candidate is
only a 32-bit match and can be a collision; the JSON output carries an
`evidence` field for each name.

UNKNOWN: whether the hashed string is a bare name, a name with extension, or a
path; and what 64-bit hash fills column `0x73bcb72dce74f11f` (a likely second
name or path hash, unconfirmed).

## Compression and nesting

Code: `Archive::read_entry_blocks`, `inflate_zlib_chain`, `sniff_ext` in
`reader.rs`; `walk` in `main.rs`.

KNOWN:

- A payload is either **raw** or a **zlib chain**. It counts as zlib when its
  first two bytes form a valid zlib header (`zlib_head`).
- A zlib chain is a sequence of independent zlib streams. forge9 inflates one
  stream, advances by the bytes it consumed, rounds up to the next 16-byte
  boundary, and continues while the next byte is `0x78`. forge9's writer
  produces the same shape (`zlib_chain`): 64 KiB input chunks, each compressed
  separately and padded to 16 bytes.
- Paged resources keep their system and graphics pages on stream boundaries,
  so `read_entry_blocks` returns the streams separately. Page layout:
  [drawable_format.md](drawable_format.md).
- **Page stitching.** Some multi-page resources continue across physically
  adjacent entries. The independent reader used for validation stitches 64 KiB
  pages across those neighbours; forge9's reader does not stitch and reads
  each entry on its own span. forge9's writer keeps the original physical
  order so the neighbours stay adjacent (see [Writing archives](#writing-archives)).
  How the next page is located, and what marks a continued entry, is UNKNOWN.
- **Nesting.** An entry whose (inflated) content starts with `9FPR` is an RPF9
  and opens with the same reader, the same keys and the same IV. `list
  --recursive`, `tree`, `stats`, `diff` and `catalog` descend into them; every
  entry-taking command accepts `a/b` paths. Observed nesting: `script.rpf`
  holds 2 nested RPF9s (stored records, 4 KiB aligned); a story content pack
  holds 18 nested archives (compact records, 465 entries in all).
- Payload kinds are detected by magic only (`sniff_ext`): `9FPR`, `PSIN`,
  `RBF0`, `ADAT`, `SGD5`, `RDS5`, `GFX\0`/`CFX\0`, XML, `0DRP`, `KB2`, `DDS `,
  PNG. Entry flags are not used for typing.

UNKNOWN: whether any record field declares the codec or the decoded length
(`size_hint` is a candidate), and any codec other than zlib in the container.

## Writing archives

Code: `src/rpf9/writer.rs`. Validation: [repack_validation.md](repack_validation.md).

KNOWN:

- **In-place replacement** (`replace_inplace`, `replace_zlib_inplace`) swaps a
  payload of the same stored size and leaves the TOC ciphertext untouched; a
  zlib replacement is recompressed and zero-padded to the original size.
- **Repack** (`repack_bytes`): decrypt the TOC with the archive's key, lay
  payloads out in their **original physical order**, each at the alignment of
  its original offset (capped at 4 KiB) and padded to 16 bytes, rewrite each
  record's location with `encode_location`, re-encrypt the TOC with the same
  key and `TAG_IV`, and write the original header bytes, the new TOC, the data
  and the original trailer.
- `encode_location` keeps the layout: an indexed record has only its offset
  word (`+0x08`) rewritten; a stored record has all three u64 fields rewritten.
- An **identity repack is byte-identical** on `common.rpf` (418 entries),
  `appdata0.rpf`, `script.rpf` and `shaders.rpf`, which proves the encryptor
  exactly inverts the decryptor for those keys.
- Archives with **compact** records are refused by repack ("repack
  unsupported") rather than written with stale offsets.
- `repack --verify` and `mod apply --verify` reopen the output and compare
  every entry with the source.

UNKNOWN: the real alignment rule (the `alignment` header field is not used;
the writer's rule is an inference marked in the code); and whether `size_hint`,
`flags` or the unread columns must change when a payload's size or codec
changes.

## Integrity and signatures

Summary of [repack_validation.md](repack_validation.md), "Game loader":

- KNOWN (reported by `rpf9-explorer`): RPF9 headers carry **RSA-2048 PKCS#1
  v1.5 signatures** over selected header ranges, verified offline across 22
  standalone and 4,351 child headers. forge9 does not parse, check or produce
  them; repack keeps the original header bytes.
- KNOWN (static analysis of the retail executable): the RPF9 load path checks
  only the 4-byte magic, then reads the header and TOC. **No signature or hash
  check was found** on that path. The only gate observed is the TOC cipher,
  which forge9 reproduces exactly.
- UNKNOWN: this is a static negative on a loader with unresolved virtual calls,
  not a proof. No repacked archive has been shown to load in the running game.
  Where the signature bytes are stored is not modeled by forge9 (the trailer is
  one region forge9 copies without reading).

## What is still unknown

Every item below is open. Each is either not parsed by forge9, or parsed with
a meaning that is not established.

**Header**

1. `split` (`0x08`): meaning.
2. `archive_id` (`0x10`): what it identifies and whether anything checks it.
3. `gen_word` (`0x18`): what the "generation" (high 16 bits) means; the low 16
   bits.
4. `gen_const` (`0x1C`): meaning.
5. `key_id` (`0x28`): how, or whether, it maps to a key slot. forge9 tries every
   key instead.
6. `flags` (`0x2C`): every bit; whether `8 + (flags & 0xFF)` really is a stride.
7. `alignment` (`0x30`): how it relates to payload alignment.
8. `0x34 .. 0x40`: whether it is always zero or holds data.
9. The trailer: its content, and whether it holds the signatures.

**TOC**

10. Where the shared base of the column offsets is, so the records can be read
    from their descriptor instead of by scanning.
11. The leading record skipped when the run is `entry_count + 1`.
12. Column `0x73bcb72dce74f11f` (stride 8): the 64-bit hash function and the
    string it hashes.
13. Column `0xe0c12d7a5d4884c5` (stride 8): content.
14. Column `0xb6f5bdb2c66da2e9`: stride and content.
15. Short or missing name-hash columns: why some archives declare fewer name
    hashes than records, or none.
16. Whether the six columns are the "six record tables" of `rpf9-explorer`'s
    notes, and whether more exist.

**Records**

17. `size_hint`: exact meaning (declared decoded length is the leading guess).
18. Record `flags` and `aux` in the indexed and compact layouts.
19. The 16 zero bytes before every compact payload.

**Names**

20. What string the u32 joaat covers (bare name, with extension, or path); case
    rules.

**Payloads and writing**

21. How a multi-page resource continues into the physically next entry, and
    what marks it.
22. Any declared codec or decoded length in the index; any codec besides zlib.
23. The real payload alignment rule.
24. Which index fields the game requires to change when a payload's size or
    codec changes.

**Integrity**

25. Whether the running game rejects an archive whose header signature no
    longer matches (no runtime test yet).
26. The cipher's lineage (naming only).

## Format index

| Document | Covers |
|---|---|
| [rpf9_format.md](rpf9_format.md) | This page: the RPF9 container |
| [repack_validation.md](repack_validation.md) | Repack checks against an independent reader, byte-identical identity repacks, header signatures and the loader finding |
| [mod_format.md](mod_format.md) | Mod packs: `mod.toml`, patches against entry paths, base hashes |
| [drawable_format.md](drawable_format.md) | Paged resources and drawables inside content archives |
| [geometry_check.md](geometry_check.md) | Drawable buffer dumps and captured-position checks |
| [bcpack_bc5_bc7_structure.md](bcpack_bc5_bc7_structure.md) | BCPack BC5/BC7 texture stream structure |
| [mesh_shader_binding_model.md](mesh_shader_binding_model.md) | Mesh shader root parameter binding |
| [wave64_instruction_usage.md](wave64_instruction_usage.md) | Wave64-sensitive instructions in the shader database |
| [rdna2_opcode_coverage.md](rdna2_opcode_coverage.md) | RDNA2 opcode coverage of the shader database |
| [exec_uniformity.md](exec_uniformity.md) | EXEC uniformity of shader programs |
| [shader_compile_cost.md](shader_compile_cost.md) | Heaviest shader programs |
| [rt_traversal_census.md](rt_traversal_census.md) | Ray-traversal intersect sites |
