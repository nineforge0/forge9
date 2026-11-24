# xml-tweak

A minimal v2 mod: it sets one attribute value in one XML entry of `common.rpf`.
It contains no game files, only the entry path, the SHA-256 of the original
payload, and the patch op. See [docs/mod_format.md](../../docs/mod_format.md).

```bash
forge9 mod verify mod.toml --archive /path/to/common.rpf
forge9 mod apply mod.toml --archive /path/to/common.rpf -o modded.rpf
forge9 diff /path/to/common.rpf modded.rpf   # ~ 9  591 -> 591 bytes (+0) .xml
```

If your copy is a different version, or the entry is already modded, `verify`
and `apply` stop with `base_sha256 mismatch`.

Made with `forge9 mod make --path 9 original.xml edited.xml`, with `expect` removed.
