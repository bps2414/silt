# r/pcgaming (draft, not posted; check the sub's self-promotion rules first)

**Title:** I made a free tool that shrinks game folders with Windows' built-in compression: same files, less space, undo any time

**Body:**

SSD full, and the only thing taking space is the games you actually play? Windows 10 and 11 can
store files compressed and decompress them on the fly when a game reads them (the same thing
`compact /exe` and CompactGUI do). Reading costs a little CPU; most games should load about the same [measure load times on two
or three games before posting].

Silt (https://github.com/bps2414/silt, free, open source) does that inside a disk analyzer:

- scans your drive in a couple of seconds and shows what fills it (games, apps, caches, ...);
- estimates how much each game folder would save **before** touching anything;
- compresses the ones you tick, and **Restore** puts them back exactly as they were;
- skips files that don't compress (videos, `.pak` archives) and anti-cheat folders
  (EasyAntiCheat, BattlEye, Vanguard, ...), and warns on games using DirectStorage.

Real result: an Xbox PC game went from 5.1 GB to 2.2 GB on disk (saved 2.9 GB). Results vary a lot
by game: ones that ship already-compressed packages save little, and Silt tells you so up front.

A game update rewrites the files it changes uncompressed; the next scan shows "updated" on that
folder so you can shrink it again.

It also cleans shader caches (NVIDIA, AMD, DirectX), unticked by default since games rebuild them
on the next launch.

**Image:** `docs/media/shrink.png` (fixed demo drive, no real paths)
