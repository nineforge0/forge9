# forge9

General-purpose toolkit for RPF9 archives. Read, decrypt, inspect, extract and analyze.

RPF9 is a container format used by a well-known retail game engine. This tool handles decryption, TOC parsing, entry listing, content extraction, and format detection — a foundation for research, interoperability work and asset tooling.

## Features

- **Decrypt** archive table-of-contents using a 9-key T-table cipher with CBC mode
- **Inspect** headers, entry counts, key IDs, version info and flags
- **List** all entries with offset, size, format detection (zlib, XML, PSIN, nested RPF9, etc.)
- **Extract** entries to disk with automatic zlib chain inflation
- **Hexdump** individual entries for analysis
- **JSON output** for integration with other tools

## Usage

```bash
# Show archive info
forge9 info archive.rpf

# List entries
forge9 list archive.rpf
forge9 list archive.rpf --json

# Extract all entries
forge9 extract archive.rpf -o output_dir

# Extract specific entries
forge9 extract archive.rpf --select 0,1,5

# Hex dump entry #0
forge9 hexdump archive.rpf 0 --limit 256
```

## Crypto data

The decryption tables and key schedules must be provided externally in a `data/` directory (or set via `--data-dir` / `FORGE9_DATA` env var):

- `tables.bin` — 20,480 bytes: 16 permuted S-boxes + 16 T-tables
- `keys.bin` — 39,060 bytes: 9 expanded key schedules

These files are **not included** in this repository. They must be extracted from the target runtime by the user from their own legally obtained copy.

## Legal

This tool does not include, distribute or link to any game files, encryption keys, executables or copyrighted content. All crypto material and archive files must come from the user's own legally purchased copy of the game. This project exists for interoperability research and modding purposes, in the same spirit as tools like Wine, DXVK and Proton.

## Building

```bash
cargo build --release
```

## Roadmap

- [ ] Archive repacking (encrypt + rebuild TOC)
- [ ] Name recovery from hash dictionaries
- [ ] Nested RPF9 recursive inspection
- [ ] PSIN/PSO data structure browser
- [ ] Payload format decoders (textures, models, audio)
- [ ] Streaming world analysis tools

## License

MIT
