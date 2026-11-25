# `.7z` fixtures of the game database import

Small 7-Zip archives that the import tests read (`gamedb/tests.rs`, the core's
`facade_gamedb` test and the app's `e2e::database`). They are files rather than archives the
tests write, so no build of KChess turns on `sevenz-rust2`'s encoders or its AES: the workspace
hack would otherwise build them into the app too.

They were written with `sevenz-rust2` 0.25.0's `ArchiveWriter`, from the `GAMES` text of
`gamedb/tests.rs` (three usable games, two skipped):

| File | What it holds |
| --- | --- |
| `mixed-lzma2.7z` | A folder `ccrl/`; `ccrl/notes.txt` (a game that is not a PGN file); `ccrl/first.pgn` (one game, with no line ending after it); `__MACOSX/ccrl/._first.pgn` (macOS's attributes); `ccrl/More.PGN` (`GAMES`). Each file compressed on its own with LZMA2, 7-Zip's default. |
| `mixed-solid-ppmd.7z` | The same files, in one solid PPMd stream. |
| `no-pgn.7z` | `lumbra.si5` and `readme.txt`: no PGN file, as Lumbra's Scid downloads. |
| `many.7z` | `games.pgn`: `GAMES` forty times, LZMA2. |
| `locked.7z` | `games.pgn` (`GAMES`), AES-256 encrypted with the password `secret`. |
| `locked-names.7z` | The same, with its file names encrypted too. |

To write them again, use `ArchiveWriter` with `set_content_methods` (PPMd; AES-256 then LZMA2
for the locked ones), `push_archive_entries` for the solid one and `set_encrypt_header(true)` for
`locked-names.7z`, outside this workspace.
