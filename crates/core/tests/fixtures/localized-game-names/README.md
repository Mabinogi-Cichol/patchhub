# Localized-game-name fixtures

These are independent, minimal binary fixtures for the Rust appinfo parser.
They contain only a public game AppID and its public name subtree. No accounts,
tokens, local configuration, ownership, timestamps or unrelated application data
from the Steam cache are retained. Entry metadata and hashes are zero-filled;
V29 keys are remapped to a six-item table.

## Source observation (2026-10-07, Windows)

The V29 subtree was reconstructed from a read-only observation of local Steam
`appcache/appinfo.vdf`; Steam.exe FileVersion was `11.08.81.85`.
The observed source file is a newer snapshot than the feature's original
899-entry investigation. This snapshot has 2,416,036 bytes, magic `0x07564429`,
universe 1, key table offset 2,377,315, 4,320 keys and 900 entries. The zero AppID
terminator is at 2,377,311. Full source SHA-256:
`12f39a5dde802c23cc9adc9ba880f21f840f73ceb8202d561265c04f80f7a61f`.

AppID 2303800 starts at 2,014,830, entry_size 2485. Its entire 2425-byte payload
was decoded independently using only tags 0/1/2; it consumed exactly 2425 bytes
and 186 nodes. Retained decoded paths:

- `appinfo.common.name`: `My Life as a Cult Leader`
- `appinfo.common.name_localized.schinese`: `妃神会秘史`
- `appinfo.common.name_localized.japanese`: `色々教団生活`

`v29-real-name-subtree.vdf` is 239 bytes; SHA-256:
`a36107f4ed0518247d7cbaa934a9c7876f9155abd7de7d7152c0469469461e11`.

`v28-synthetic-name-subtree.vdf` is a synthetic V28 encoding of those public
names; it is not a capture from an older Steam installation. It uses inline
keys and an 8-byte file header. It is 203 bytes; SHA-256:
`c415e7c3022708e622c469b72748ffdc0cb68fc3ae20d58ea2f95f2098149747`.

Both entries have 68-byte fixed headers, entry_size = payload bytes + 60,
and next entry offset = entry offset + entry_size + 8. The zero AppID follows
the entry; only V29 then has a key count and NUL-terminated UTF-8 key table.

## Format and license boundary

The Rust implementation is independently written from the recorded format
contract, with no third-party implementation copied. The feature's original
format references are Protontricks (GPL-3.0); they are references only.

UTF-16 tag 5 is deliberately rejected for the entire target entry in this
iteration. No real appinfo sample with this tag was available to establish its
framing. SteamKit's primary implementation also rejects WideString (observed
2026-10-07, `KeyValue.cs`, Type.WideString branch):
https://raw.githubusercontent.com/SteamRE/SteamKit/master/SteamKit2/SteamKit2/Types/KeyValue.cs
A future verified wide-string fixture can add support without guessing between
length-prefixed and NUL-terminated formats. Invalid/unknown tags and trailing or
incomplete payload data likewise discard every name from that entry.