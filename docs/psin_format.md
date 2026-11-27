# PSIN container format (PSO v122)

The reference for the PSIN binary-XML container as forge9 reads it. Every
statement here is backed by forge9's code (cited by file), by a test, or by
a finding recorded in another forge9 document. Each section separates what
is **KNOWN** (implemented, checked on real data) from what is **UNKNOWN**
(not parsed, not explained, or inferred).

PSIN containers hold compiled game configuration: vehicle handling, weapons,
AI behavior, spawn rules, ped metadata and other data-driven definitions.
They are found as payloads of RPF9 entries, primarily in `script.rpf` and
its nested archives.

All integers are little-endian unless stated otherwise. Section length fields
in the IFF chain are big-endian (the only BE values in the format).

Code: `src/rpf9/psin.rs`.

## Contents

1. [Overview](#overview)
2. [IFF-chunk chain](#iff-chunk-chain)
3. [LSCH — struct schema](#lsch--struct-schema)
4. [LENM — enum definitions](#lenm--enum-definitions)
5. [PSIN body — instance blocks](#psin-body--instance-blocks)
6. [Block layout](#block-layout)
7. [DOM tree](#dom-tree)
8. [Pool references](#pool-references)
9. [Attribute types](#attribute-types)
10. [Member type codes](#member-type-codes)
11. [Other sections](#other-sections)
12. [What is still unknown](#what-is-still-unknown)

## Overview

A PSIN file is an **IFF-chunk chain**: a sequence of tagged sections, each
with a 4-byte ASCII tag and a u32 big-endian length (which includes the
8-byte header itself). The first section is always PSIN and carries the
instance data (binary-XML DOM). Subsequent sections carry schema, string
tables and checksums.

The chain observed in retail data:

```
PSIN → LSCH → LENM → LSNS → LGID → LRSC → CHKS
```

Not every section is present in every container. The parser walks the chain
sequentially and tolerates absent sections.

## IFF-chunk chain

KNOWN, implemented in `PsinContainer::parse` (`psin.rs:168`):

| Offset | Size | Field | Endian | Meaning |
|--------|------|-------|--------|---------|
| `+0x00` | 4 | tag | — | ASCII section identifier |
| `+0x04` | u32 | length | **BE** | Total section length including this 8-byte header |

The body of each section is `data[offset + 8 .. offset + length]`. Sections
are concatenated with no padding; the next section starts at
`offset + length`.

The parser reads the first 4 bytes as magic and rejects anything that does
not start with `PSIN`.

KNOWN: the PSIN section at offset 0 is special — it carries the instance
block data, not schema. The parser collects the PSIN body but defers parsing
it until LSCH has populated the struct table, so the DOM decoder can check
for the `root_document` struct hash (`psin.rs:192–197`).

## LSCH — struct schema

Code: `parse_lsch` in `psin.rs:218`.

LSCH (Layout SCHema) defines the struct and member types used by the
instance blocks. The section body has two regions:

### Struct headers

A contiguous table of 32-byte records. The end of the table is detected by
the 8-byte sentinel `0xAA × 8` at bytes `+0x10..+0x18` of each record.
Parsing stops at the first record that lacks the sentinel.

| Offset | Size | Field | Status | Meaning |
|--------|------|-------|--------|---------|
| `+0x00` | u32 | `name_hash` | KNOWN | joaat hash of the struct name |
| `+0x04` | u32 | `signature` | KNOWN (read) | A 32-bit value that changes when the struct layout changes; not a joaat |
| `+0x08` | 8 | (padding) | UNKNOWN | Bytes before the sentinel; not read |
| `+0x10` | 8 | sentinel | KNOWN | `0xAA × 8`; marks a valid struct header |
| `+0x18` | u32 | `size` | KNOWN | Byte size of a serialised instance of this struct |
| `+0x1C` | 2 | (padding) | UNKNOWN | Not read |
| `+0x1E` | u16 | `member_count` | KNOWN | Number of members that follow in the member region |

### Member records

Immediately after the last struct header, all members are stored **back to
back in struct-header order**: struct 0's members first, then struct 1's,
etc. Each member is 16 bytes:

| Offset | Size | Field | Status | Meaning |
|--------|------|-------|--------|---------|
| `+0x00` | u32 | `name_hash` | KNOWN | joaat hash of the member name. `0x100` marks an anonymous array-element descriptor. |
| `+0x04` | u32 | `offset` | KNOWN | Byte offset of this member within a serialised struct instance |
| `+0x08` | u8 | `type_code` | KNOWN | Encodes the member's data type (see [Member type codes](#member-type-codes)) |
| `+0x09` | u8 | `sub_type` | PARTLY KNOWN | Secondary type qualifier; meaning varies by `type_code` |
| `+0x0A` | u16 | `reference` | PARTLY KNOWN | Index or reference; usage varies |
| `+0x0C` | u32 | `extra` | PARTLY KNOWN | For struct-typed members: the joaat hash of the target struct. For enum-typed members: the joaat hash of the enum definition. Zero otherwise. |

KNOWN (measured): `schema-db` harvests unique struct/enum definitions
across all PSIN entries in an archive set. Across the retail data:
**6,710 unique structs** and **1,132 unique enums** (by signature), of
which **206 structs** and **24 enums** have names resolved by the hash
dictionary.

Unit test: `lsch_headers_precede_members`.

## LENM — enum definitions

Code: `parse_lenm` in `psin.rs:251`.

LENM (Layout ENuM) defines enumerations. The section body has two regions:

### Enum headers

A contiguous table of 24-byte records. The end is detected by the sentinel
`0xEE × 8` at bytes `+0x08..+0x10`:

| Offset | Size | Field | Status | Meaning |
|--------|------|-------|--------|---------|
| `+0x00` | u32 | `name_hash` | KNOWN | joaat hash of the enum name |
| `+0x04` | 4 | (padding) | UNKNOWN | Not read |
| `+0x08` | 8 | sentinel | KNOWN | `0xEE × 8` |
| `+0x10` | u32 | `value_count` | KNOWN | Number of values that follow |
| `+0x14` | 4 | (padding) | UNKNOWN | Not read |

### Enum values

After the last header, all values are stored back to back in header order.
Each value is 16 bytes:

| Offset | Size | Field | Status | Meaning |
|--------|------|-------|--------|---------|
| `+0x00` | i64 | `value` | KNOWN | The enum's integer value (signed) |
| `+0x08` | u64 | `name` | KNOWN | A name identifier; appears to be a joaat or similar hash of the value name |

## PSIN body — instance blocks

Code: `parse_psin_instances` in `psin.rs:283`.

The body of the first (offset-0) PSIN section carries the **instance
blocks**: serialised struct instances that hold the actual game data. The
body is structured as a block header table followed by a data region.

The DOM tree (binary-XML) is one specific block type within this region.
Parsing proceeds only when the LSCH section defines a struct whose
`name_hash` is `0xde396fe2` (`root_document`).

## Block layout

KNOWN, fixed by the DOM decoder rewrite:

The body starts with a **contiguous header table** of 16-byte block
descriptors. Each descriptor:

| Offset | Size | Field | Meaning |
|--------|------|-------|---------|
| `+0x00` | u32 | `type_hash` | joaat hash identifying the block's struct type |
| `+0x04` | u32 | `size` | Byte size of this block's data |
| `+0x08` | 8 | sentinel | `0xAA × 8`; marks a valid block header |

The table ends at the first 16 bytes that do not have the sentinel at
`+0x08`. Let **N** be the number of headers found.

The **data region** starts at the first 16-byte-aligned file offset after
the header table. Because the PSIN section itself starts at file offset 0,
and its body starts at file offset 8:

```
data_start = align16(8 + N * 16)
body_offset = data_start - 8
```

Blocks are laid out **sequentially** in the data region, each at the next
16-byte-aligned offset after the previous block's data:

```
block[0] starts at data_start
block[i+1] starts at align16(block[i].start + block[i].size)
```

KNOWN (critical fix): an earlier decoder assumed blocks immediately
followed their headers. The correct layout has the contiguous header table
first, then padding to file alignment, then all data blocks in sequence.
This was verified on all 1,012 PSIN entries in `script.rpf/1`.

## DOM tree

Code: `parse_psin_instances`, `Dom` struct and methods in `psin.rs`.

The DOM tree lives in the block whose `type_hash` is `0xde396fe2`
(`root_document`). This block is a serialised `root_document` struct
instance with five **atArray** references at fixed offsets within it:

| Struct offset | Array | Element size | Content |
|---------------|-------|-------------|---------|
| `+0x00` | attributes | 16 bytes | `DomAttr` records |
| `+0x10` | strings | — | String pool (not decoded) |
| `+0x20` | vectors | 12 bytes | `[f32; 3]` vec3 values |
| `+0x30` | u64_hashes | 8 bytes | `u64` hash values |
| `+0x40` | nodes | 16 bytes | `DomNode` records |

Each atArray reference occupies 16 bytes at the given offset:

| Offset | Size | Field | Meaning |
|--------|------|-------|---------|
| `+0x00` | u32 | `count` | Number of elements |
| `+0x04` | 4 | (padding) | Not read |
| `+0x08` | u64 | `pool_ref` | Pool reference (see [Pool references](#pool-references)) |

If `count` is 0 the array is empty and `pool_ref` is ignored.

### DomNode (16 bytes)

| Offset | Size | Field | Status | Meaning |
|--------|------|-------|--------|---------|
| `+0x00` | u32 | `name_hash` | KNOWN | joaat hash of the element name |
| `+0x04` | u16 | `parent` | KNOWN | Index of the parent node, or `0xFFFF` for the root |
| `+0x06` | u16 | `child` | KNOWN | Index of the first child node, or `0xFFFF` if leaf |
| `+0x08` | u16 | `sibling` | KNOWN | Index of the next sibling, or `0xFFFF` if last |
| `+0x0A` | u16 | `prev_sibling` | KNOWN | Index of the previous sibling |
| `+0x0C` | u16 | `attr_start` | KNOWN | Index into the attributes array |
| `+0x0E` | u8 | `attr_count` | KNOWN | Number of attributes on this node |
| `+0x0F` | 1 | (padding) | UNKNOWN | Not read |

The tree is walked by following `child` from a node, then `sibling` from
each child. The root node is at index 0. A depth cap of 64 guards against
cyclic links (`psin.rs:130`).

### DomAttr (16 bytes)

| Offset | Size | Field | Status | Meaning |
|--------|------|-------|--------|---------|
| `+0x00` | u32 | `name_hash` | KNOWN | joaat hash of the attribute name |
| `+0x04` | u32 | `value_hash` | KNOWN (read) | Hash of the attribute value; usage not fully established |
| `+0x08` | u32 | `raw_value` | KNOWN | Interpretation depends on `attr_type` (see [Attribute types](#attribute-types)) |
| `+0x0C` | u8 | `attr_type` | KNOWN | Type discriminant (0–5) |
| `+0x0D` | 3 | (padding) | UNKNOWN | Not read |

KNOWN (measured on `script.rpf/1`): the DOM tree decoder produces
**20,379 nodes** across 1,012 PSIN entries, rendering to **34,238 lines
of XML**.

## Pool references

Code: `read_pool` closure in `parse_psin_instances` (`psin.rs:325`).

An atArray's data may live in a different block than the struct that
references it. The 64-bit `pool_ref` encodes a cross-block pointer:

| Bits | Field | Meaning |
|------|-------|---------|
| `0..11` | block index | **1-based** index into the block header table. 0 means null. |
| `12..19` | (unused) | Not extracted; implicitly zero in observed data |
| `20..63` | byte offset | Byte offset within the target block's data |

Resolved as:
```
block = blocks[pool_ref & 0xFFF - 1]
data  = block.data[pool_ref >> 20 ..]
```

KNOWN: this encoding is verified by successfully decoding all five atArray
pools across 1,012 entries.

## Attribute types

Code: `DomAttr::type_name`, `Dom::format_attr` in `psin.rs`.

| `attr_type` | Name | `raw_value` interpretation | Status |
|-------------|------|---------------------------|--------|
| 0 | `string` | joaat hash; resolved via dictionary | KNOWN |
| 1 | `vec3` | Index into the vectors array | KNOWN |
| 2 | `float` | IEEE 754 `f32` bits | KNOWN |
| 3 | `int` | Signed 32-bit integer | KNOWN |
| 4 | `bool` | 0 = false, nonzero = true | KNOWN |
| 5 | `hash64` | Index into the u64_hashes array | KNOWN |
| 6+ | `unknown` | Printed as hex | UNKNOWN |

KNOWN: type 0 (`string`) stores a **joaat hash**, not a string index. The
DOM contains no string-index attributes (type count = 0 across all retail
entries). Attribute name resolution depends entirely on the hash dictionary.

KNOWN (measured): of the 257 unique attribute name hashes across
`script.rpf/1`, **48 (19%)** are resolved by the forge9 dictionary, covering
**43%** of total attribute occurrences.

## Member type codes

Code: `member_type_name` in `psin.rs:405`.

Type codes used in LSCH member definitions:

| Code | Name | Notes |
|------|------|-------|
| `0x01` | `bool` | |
| `0x05` | `struct` | `extra` = target struct name hash |
| `0x07` | `ptr` | Pointer to struct |
| `0x10`, `0x11` | `u8` | |
| `0x12`, `0x13` | `u16` | |
| `0x14` | `i32/f32` | 32-bit, signed int or float |
| `0x15` | `u32` | |
| `0x16`, `0x17`, `0x18` | `u64` | |
| `0x20`–`0x22` | `vec3` | 3-component float vector |
| `0x23`, `0x24` | `vec4` | 4-component float vector |
| `0x44` | `string` | String reference |
| `0x48` | `hash_string` | Hashed string |
| `0x4A` | `joaat` | 32-bit joaat hash |
| `0x50` | `array_fixed` | Fixed-size array |
| `0x52` | `array_ptr` | Pointer array |
| `0x5C` | `array_counted` | Counted array (atArray) |
| `0x60` | `enum_u8` | 8-bit enum; `extra` = enum name hash |
| `0x65` | `bitset` | Bitfield |

This table is not exhaustive; unrecognised codes print as `?`.

## Other sections

### LSNS — string table

KNOWN (observed): present in most containers. The body has high entropy
(~7.9 bits/byte), indicating encryption or compression.

UNKNOWN: the encryption/compression scheme, and therefore the string
content. Because the DOM uses zero string-index attributes (type 0 stores
hashes, not indices), LSNS decryption would not resolve DOM attribute names.
It may contain display strings, localisation keys, or other runtime text.

### LGID

UNKNOWN. Present in retail containers. Body not parsed.

### LRSC

UNKNOWN. Present in retail containers. Body not parsed.

### CHKS — checksum

KNOWN (observed): typically the last section. Likely a checksum or integrity
value over the preceding sections.

UNKNOWN: the checksum algorithm and what it covers.

## What is still unknown

**IFF chain**

1. Whether sections can appear in a different order than observed.
2. Whether multiple LSCH or LENM sections can appear (the parser takes the
   last one encountered).

**Schema**

3. The exact semantics of `StructDef::signature` — it tracks layout changes
   but the hash/versioning scheme is not identified.
4. The `sub_type` and `reference` fields of member definitions: their role
   varies by `type_code` and is not fully mapped.
5. ~6,500 struct name hashes and ~1,100 enum name hashes unresolved.

**DOM**

6. The `value_hash` field of `DomAttr`: read but not used for
   reconstruction. May be a hash of the serialised attribute value for
   integrity or lookup.
7. The strings atArray at `+0x10` in the root_document block: not decoded.
   Its pool_ref encoding should match the others, but no entries with
   nonzero string count have been observed.
8. The `prev_sibling` field of `DomNode`: read but not used for tree
   traversal (child/sibling walk is sufficient). Assumed to be a back-link
   for O(1) insertion.
9. 209 of 257 unique DOM attribute name hashes unresolved. These require
   hash cracking (dictionary attack on joaat), not LSNS decryption.

**Instance blocks**

10. Blocks other than `root_document`: each block is a serialised struct
    instance, but only the root_document block is decoded into a DOM. Other
    blocks (vehicles, weapons, etc.) can be interpreted using the LSCH
    schema once member semantics are mapped.
11. The relationship between block `type_hash` and LSCH `name_hash` — they
    appear to be the same joaat hash, but this is confirmed only for
    `root_document` (`0xde396fe2`).

**Sections**

12. LSNS encryption/compression.
13. LGID content and purpose.
14. LRSC content and purpose.
15. CHKS algorithm.

**Writing**

16. No PSIN writer exists. forge9 can read and render PSIN data but cannot
    modify and re-serialise it. A writer would need to produce valid block
    layouts, correct pool references, update LSCH if struct shapes change,
    and regenerate CHKS.
