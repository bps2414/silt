//! `MftScanner`: reads the Master File Table of an NTFS volume with `ntfs-ng` (admin only).
//!
//! Sizes are allocated bytes, summed over every non-resident attribute of the file: all `$DATA`
//! streams (a WOF-compressed file keeps its data in `WofCompressedData`), the folder index and
//! the rest. Compressed and sparse streams count what they really allocate; cloud placeholders
//! count 0. A file with several hard links counts once, on its first name. Files whose
//! attributes spill into extension records (`$ATTRIBUTE_LIST`) are completed from them.

use std::collections::HashMap;
use std::io;
use std::ops::ControlFlow;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

use ntfs_ng::BootSector;
use ntfs_ng::mft::Mft;
use ntfs_ng::scan::{Chunk, Entry, Projection, Scan};
use ntfs_ng::source::{Device, DeviceOptions, read_at};
use ntfs_ng::types;
use ntfs_ng::value::{FileName, ReparsePoint, StandardInformation};

use super::{Progress, ScanMsg, flush};
use crate::tree::{NO_ID, RawEntry, flags};
use crate::volume::Volume;
use crate::win;

/// The root folder's record number.
pub const ROOT: u64 = 5;

pub struct Opened {
    mft: Mft<Device>,
    cluster: u64,
}

pub fn open(volume: &Volume) -> io::Result<Opened> {
    let dev = Device::open(format!(r"\\.\{}:", volume.letter), DeviceOptions::default())?;
    let mut sector = vec![0u8; 4096];
    read_at(&dev, 0, &mut sector)?;
    let boot = BootSector::parse(&sector[..512]).map_err(|e| io::Error::other(e.to_string()))?;
    if !boot.is_ntfs() {
        return Err(io::Error::other("not NTFS"));
    }
    let mft = Mft::from_volume(dev, &boot).map_err(|e| io::Error::other(e.to_string()))?;
    Ok(Opened {
        mft,
        cluster: volume.cluster,
    })
}

/// What one record says about its file.
#[derive(Default)]
struct Piece {
    names: Vec<(u64, String, bool)>, // (parent record, name, dos only)
    alloc: u64,
    logical: Option<u64>,
    mtime: Option<u32>,
    attrs: u32,
    tag: u32,
    wof_stream: bool,
    unnamed_compressed: bool,
    unnamed_sparse: bool,
}

impl Piece {
    fn merge(&mut self, o: Piece) {
        self.names.extend(o.names);
        self.alloc += o.alloc;
        self.logical = self.logical.or(o.logical);
        self.mtime = self.mtime.or(o.mtime);
        self.attrs |= o.attrs;
        if self.tag == 0 {
            self.tag = o.tag;
        }
        self.wof_stream |= o.wof_stream;
        self.unnamed_compressed |= o.unnamed_compressed;
        self.unnamed_sparse |= o.unnamed_sparse;
    }
}

struct Base {
    number: u64,
    dir: bool,
    piece: Piece,
}

#[derive(Default)]
struct Out {
    ready: Vec<RawEntry>,
    /// Base records with an attribute list: completed after the scan.
    listed: Vec<Base>,
    /// Extension records: (base record, piece).
    ext: Vec<(u64, Piece)>,
}

struct Proj {
    cluster: u64,
}

impl Projection for Proj {
    type Output = Out;
    type Scratch = ();

    fn scratch(&self) {}

    fn begin(&self, _: &mut (), _: Chunk) -> Out {
        Out::default()
    }

    fn entry(&self, out: &mut Out, _: &mut (), number: u64, entry: Entry<'_>) {
        let Entry::Record(rec) = entry else { return };
        if !rec.in_use() {
            return;
        }
        let mut piece = Piece::default();
        let mut listed = false;
        for attr in rec.attributes() {
            let Ok(attr) = attr else { break };
            let t = attr.type_code();
            if t == types::ATTRIBUTE_LIST {
                listed = true;
            }
            if !attr.is_resident() {
                let Ok(nr) = attr.non_resident() else {
                    continue;
                };
                // Allocated = the clusters the runs point at, in every extent. The header's
                // sizes mislead for a sparse stream without the sparse flag: `$BadClus:$Bad`
                // claims the whole volume.
                let clusters: u64 = nr
                    .runs()
                    .map_while(Result::ok)
                    .filter(|r| r.lcn.is_some())
                    .map(|r| r.length)
                    .sum();
                piece.alloc += clusters * self.cluster;
                if nr.lowest_vcn == 0 && t == types::DATA {
                    if attr.is_unnamed() {
                        piece.logical = Some(nr.data_size);
                        piece.unnamed_compressed = attr.is_compressed();
                        piece.unnamed_sparse = attr.is_sparse();
                    } else if attr.name().is_ok_and(|n| n.eq_str("WofCompressedData")) {
                        piece.wof_stream = true;
                    }
                }
                continue;
            }
            let Ok(value) = attr.value() else { continue };
            match t {
                types::STANDARD_INFORMATION => {
                    if let Ok(si) = StandardInformation::parse(value) {
                        piece.mtime = Some(win::ticks_unix(si.times.modified));
                        piece.attrs = si.attributes;
                    }
                }
                types::FILE_NAME => {
                    if let Ok(f) = FileName::parse(value) {
                        if piece.tag == 0 && f.reparse != 0 && f.attributes & 0x400 != 0 {
                            piece.tag = f.reparse;
                        }
                        piece.names.push((
                            f.parent.record,
                            f.name.to_string_lossy(),
                            f.namespace.is_dos_only(),
                        ));
                    }
                }
                types::DATA if attr.is_unnamed() => piece.logical = Some(value.len() as u64),
                types::REPARSE_POINT => {
                    if let Ok(r) = ReparsePoint::parse(value) {
                        piece.tag = r.tag;
                    }
                }
                _ => {}
            }
        }
        if !rec.is_base() {
            out.ext.push((rec.header().base_record.record, piece));
        } else if listed {
            out.listed.push(Base {
                number,
                dir: rec.is_directory(),
                piece,
            });
        } else {
            emit(&mut out.ready, number, rec.is_directory(), piece);
        }
    }
}

/// Turns a file's merged record into one entry per name.
fn emit(out: &mut Vec<RawEntry>, number: u64, dir: bool, p: Piece) {
    let mut f = if dir { flags::DIR } else { 0 };
    let a = p.attrs;
    if a & 0x2 != 0 {
        f |= flags::HIDDEN;
    }
    if a & 0x4 != 0 {
        f |= flags::SYSTEM_ATTR;
    }
    if a & 0x4000 != 0 {
        f |= flags::ENCRYPTED;
    }
    if p.unnamed_compressed || a & 0x800 != 0 {
        f |= flags::NTFS_COMPRESSED;
    }
    if p.unnamed_sparse || a & 0x200 != 0 {
        f |= flags::SPARSE;
    }
    if p.wof_stream || p.tag == win::TAG_WOF {
        f |= flags::WOF;
        f &= !flags::SPARSE;
    }
    if win::is_cloud_tag(p.tag) || a & (0x1000 | 0x40_0000) != 0 {
        f |= flags::CLOUD;
    }
    if win::is_name_surrogate(p.tag) {
        f |= flags::LINK;
    }
    let has_long = p.names.iter().any(|n| !n.2);
    let mut first = true;
    for (parent, name, dos) in p.names {
        if dos && has_long {
            continue; // 8.3 alias of a long name
        }
        out.push(RawEntry {
            id: if first { number } else { NO_ID },
            parent,
            name,
            alloc: if first { p.alloc } else { 0 },
            // Every name shows the file's size, as Explorer does; only the first one owns its clusters.
            logical: p.logical.unwrap_or(0),
            mtime: p.mtime.unwrap_or(0),
            flags: if first { f } else { f | flags::HARDLINK },
        });
        first = false;
    }
}

pub fn run(opened: Opened, tx: Sender<ScanMsg>, progress: Arc<Progress>, cancel: Arc<AtomicBool>) {
    let mft = opened.mft;
    let mut listed = Vec::new();
    let mut ext: HashMap<u64, Vec<Piece>> = HashMap::new();
    let scan = Scan::new(&mft).ordered(false).cancel(&cancel);
    let result = scan.run(
        &Proj {
            cluster: opened.cluster,
        },
        |_, out: Out| {
            let mut ready = out.ready;
            listed.extend(out.listed);
            for (base, piece) in out.ext {
                ext.entry(base).or_default().push(piece);
            }
            if flush(&tx, &mut ready, &progress) {
                ControlFlow::Continue(())
            } else {
                ControlFlow::Break(())
            }
        },
    );
    if let Err(e) = result {
        let _ = tx.send(if cancel.load(Ordering::Relaxed) {
            ScanMsg::Done
        } else {
            ScanMsg::Error(e.to_string())
        });
        return;
    }
    let mut batch = Vec::with_capacity(4096);
    for mut b in listed {
        for piece in ext.remove(&b.number).unwrap_or_default() {
            b.piece.merge(piece);
        }
        emit(&mut batch, b.number, b.dir, b.piece);
        if batch.len() >= 4096 && !flush(&tx, &mut batch, &progress) {
            return;
        }
    }
    flush(&tx, &mut batch, &progress);
    let _ = tx.send(ScanMsg::Done);
}
