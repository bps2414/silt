# Show HN (draft, not posted)

**Title:** Show HN: Silt – a Windows disk analyzer that frees space without deleting anything

**URL:** https://github.com/bps2414/silt

**Text:**

Every disk analyzer I've used ends at "this folder is big, delete it". That works for old
downloads, but most of what fills my drive is stuff I use: games, toolchains, apps,
`node_modules`.

Windows has had a good answer to that since Windows 10 and almost nobody uses it: the Windows
Overlay Filter can store any file compressed (XPRESS or LZX) and decompress it transparently on
read. It's what `compact /c /exe:xpress8k` does. Programs don't notice; reading costs a little CPU and there's less to read from disk [measure a
load time or two before posting].

Silt puts that next to a disk analyzer:

- It reads the NTFS Master File Table directly, so a 790k-file C: scans in about 1.2 s.
- Every byte goes into a category (apps, games, developer, caches, ...) on a dithered bar.
- It samples a few blocks of each file type with the Windows Compression API and estimates what
  each folder would save, as a range. The whole drive's estimate takes about a second.
- "Shrink" compresses the folders you pick; "Restore" undoes it. Clean empties caches that
  rebuild themselves; Trash goes to the Recycle Bin. Nothing is ever deleted permanently without
  going through the bin.

Measured on real folders (estimate vs. result, files checked byte for byte after compress and
restore): a Rust toolchain 627 MB → saved 352 MB, Git for Windows 396 MB → 196 MB, an Xbox PC game
5.1 GB → 2.9 GB. Video, images and archives are already compressed, so it leaves them alone.

It's Rust, MIT/Apache-2.0. The UI is built with Rune, a young GPU UI library, which also renders
every screen headless; the README's GIF comes from a scripted run, not a screen recording.

What I'd love feedback on: the safety rules (`rules/*.toml` plus hard-coded guards; the README
has an "Is it safe?" section listing exactly what can be deleted) and games where WOF
compression causes trouble (I already skip anti-cheat folders and warn on DirectStorage).

**Image:** `docs/media/social.png`
