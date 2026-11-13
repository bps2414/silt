# r/rust (draft, not posted)

**Title:** Silt: reading the NTFS MFT in Rust to scan a drive in ~1 s, plus transparent WOF compression to free space without deleting files

**Flair:** Project

**Body:**

I built a Windows storage analyzer in Rust: https://github.com/bps2414/silt

Some parts that might interest people here:

- **MFT scan.** With admin, it opens the volume and parses the Master File Table (on top of the
  `ntfs-ng` crate), merging `$ATTRIBUTE_LIST` extension records and counting hard links once. Size
  that counts is *allocated*, summing every stream, so WOF-compressed files, sparse files and
  OneDrive placeholders come out right. Total found vs. `GetDiskFreeSpaceExW` used: 99.98 % on my C:,
  the rest shows up as "Filesystem metadata". 790 k entries in 1.2 s, 151 MB peak.
- **Tree.** An arena `Vec<Node>` with `u32` ids and one string buffer for names. A made-up drive
  with 2 million files peaks at 280 MB in the app.
- **Shrink estimate.** It samples blocks per extension, compresses them with the Windows
  Compression API (`XPRESS_HUFF`, raw) and projects the result cluster by cluster as a range.
  Extensions learned in one folder aren't sampled again in the next (except opaque ones like
  `.dat`), which took the estimate for 72 folders from 21 s to 1.3 s.
- **Compression** is `FSCTL_SET_EXTERNAL_BACKING` with the WOF file provider; restore is
  `FSCTL_DELETE_EXTERNAL_BACKING`. Tests round-trip real files and compare hashes.
- **Clean rules** are TOML embedded with `include_str!`; deletion guards live in code so a bad
  rule can't remove a profile or `Program Files`.
- **UI** is [Rune](https://github.com/anomalyco/rune), pinned by rev. The dithered bar is a CPU
  RGBA image (Bayer 8×8) so headless snapshots match the window pixel for pixel. Long scans and
  actions run on threads and wake the window through a `Waker`; an idle window draws nothing.

Workspace: `silt-core` (no UI), `silt-cli` (everything as JSON) and `silt-app`.
MIT or Apache-2.0. Feedback on the unsafe Win32 bits is very welcome.
