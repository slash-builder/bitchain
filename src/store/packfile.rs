//! Packfile on-disk layout — format v2, locked by data-architect (CLUS-20).
//!
//! Two files per pack: `<pack_id>.pack` (data) and `<pack_id>.idx` (index).
//! The index is git-pack style: derived and rebuildable from the `.pack`
//! file alone, and is never treated as a source of truth — losing an `.idx`
//! is a `verify`/rebuild-away, not data loss.
//!
//! `.pack` layout:
//! ```text
//! header:  magic "BCPK" (4B) | format_ver u32 LE (=2) | partition_id (16B)
//! entry*:  hash (32B) | codec (1B) | flags (1B) | uncompressed_len u64 LE
//!          | stored_len u64 LE | payload (stored_len bytes)
//! trailer: entry_count u64 LE | pack_content_hash (32B)
//! ```
//! `pack_content_hash` is the BLAKE3 hash of every byte in the file *before*
//! the trailer (header + all entries) — it is also the pack's own content
//! address: sealed packs are named `<pack_content_hash_hex>.pack`.
//!
//! The trailer is only ever written when a pack is **sealed**; the
//! currently-open active pack has no trailer and is rebuilt by sequential
//! scan on reopen (crash-safe by construction — a scan simply stops at the
//! last complete entry).

use crate::addressing::{Hash, PartitionId, HASH_LEN, PARTITION_ID_LEN};
use crate::error::{BitchainError, Result};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub const PACK_MAGIC: [u8; 4] = *b"BCPK";
pub const IDX_MAGIC: [u8; 4] = *b"BCIX";
pub const FORMAT_VERSION: u32 = 2;
pub const HEADER_LEN: usize = 4 + 4 + PARTITION_ID_LEN;
pub const TRAILER_LEN: usize = 8 + HASH_LEN;
pub const ENTRY_META_LEN: usize = HASH_LEN + 1 + 1 + 8 + 8;

/// Storage codec applied to an entry's payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    /// Payload is stored uncompressed (used when Zstd does not help, e.g.
    /// already-compressed or very small inputs).
    Raw = 0,
    /// Payload is Zstd-compressed.
    Zstd = 1,
}

impl Codec {
    fn from_u8(b: u8) -> Result<Self> {
        match b {
            0 => Ok(Codec::Raw),
            1 => Ok(Codec::Zstd),
            other => Err(BitchainError::CorruptPack(format!(
                "unknown codec byte {other}"
            ))),
        }
    }
}

/// Metadata for one stored entry (payload excluded).
#[derive(Debug, Clone, Copy)]
pub struct EntryMeta {
    pub hash: Hash,
    pub codec: Codec,
    pub flags: u8,
    pub uncompressed_len: u64,
    pub stored_len: u64,
}

/// Location of a sealed entry: which pack, and byte offset of its entry
/// header within that pack.
#[derive(Debug, Clone, Copy)]
pub struct SealedLocation {
    pub pack_id: Hash,
    pub offset: u64,
}

const ZSTD_LEVEL: i32 = 3;

/// Compress `data`, falling back to raw storage if compression doesn't
/// actually shrink the payload (small or already-dense inputs).
pub fn encode(data: &[u8]) -> Result<(Codec, Vec<u8>)> {
    if data.is_empty() {
        return Ok((Codec::Raw, Vec::new()));
    }
    let compressed = zstd::stream::encode_all(data, ZSTD_LEVEL)
        .map_err(|e| BitchainError::Compression(e.to_string()))?;
    if compressed.len() < data.len() {
        Ok((Codec::Zstd, compressed))
    } else {
        Ok((Codec::Raw, data.to_vec()))
    }
}

pub fn decode(codec: Codec, stored: &[u8]) -> Result<Vec<u8>> {
    match codec {
        Codec::Raw => Ok(stored.to_vec()),
        Codec::Zstd => {
            zstd::stream::decode_all(stored).map_err(|e| BitchainError::Compression(e.to_string()))
        }
    }
}

fn write_header<W: Write>(w: &mut W, partition_id: &PartitionId) -> Result<()> {
    w.write_all(&PACK_MAGIC)?;
    w.write_all(&FORMAT_VERSION.to_le_bytes())?;
    w.write_all(partition_id.as_bytes())?;
    Ok(())
}

fn read_header<R: Read>(r: &mut R) -> Result<PartitionId> {
    let mut magic = [0u8; 4];
    r.read_exact(&mut magic)?;
    if magic != PACK_MAGIC {
        return Err(BitchainError::CorruptPack("bad magic".into()));
    }
    let mut ver = [0u8; 4];
    r.read_exact(&mut ver)?;
    let ver = u32::from_le_bytes(ver);
    if ver != FORMAT_VERSION {
        return Err(BitchainError::CorruptPack(format!(
            "unsupported pack format_ver {ver} (expected {FORMAT_VERSION})"
        )));
    }
    let mut pid = [0u8; PARTITION_ID_LEN];
    r.read_exact(&mut pid)?;
    Ok(PartitionId::from_bytes(pid))
}

/// An open, append-only pack. Entries are appended one at a time; the
/// trailer is written only at `seal()` time.
pub struct PackWriter {
    path: PathBuf,
    partition_id: PartitionId,
    file: File,
    /// Byte length of the file so far (header + entries written).
    pub len: u64,
    pub entry_count: u64,
    /// In-memory offset index for entries written this process — avoids a
    /// re-scan to serve a `get()` against data not yet sealed.
    pub offsets: HashMap<Hash, u64>,
}

impl PackWriter {
    /// Open (creating if absent) the active pack at `path` for `partition_id`,
    /// rebuilding in-memory state by scanning any bytes already there
    /// (resuming after a crash or a prior process exit).
    pub fn open_active(path: &Path, partition_id: PartitionId) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let is_new = !path.exists();
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(path)?;

        if is_new {
            write_header(&mut file, &partition_id)?;
            file.flush()?;
            return Ok(PackWriter {
                path: path.to_path_buf(),
                partition_id,
                file,
                len: HEADER_LEN as u64,
                entry_count: 0,
                offsets: HashMap::new(),
            });
        }

        // Resume: sequential scan for whatever complete entries exist.
        let mut reader = File::open(path)?;
        let found_partition = read_header(&mut reader)?;
        if found_partition != partition_id {
            return Err(BitchainError::PartitionBoundary(format!(
                "active pack at {} belongs to partition {}, not {}",
                path.display(),
                found_partition.to_hex(),
                partition_id.to_hex()
            )));
        }
        let mut offsets = HashMap::new();
        let mut pos = HEADER_LEN as u64;
        let mut count = 0u64;
        loop {
            let entry_start = pos;
            match read_entry_meta_at(&mut reader, entry_start) {
                Ok(Some(meta)) => {
                    offsets.insert(meta.hash, entry_start);
                    pos = entry_start + ENTRY_META_LEN as u64 + meta.stored_len;
                    count += 1;
                }
                Ok(None) => break, // clean EOF at an entry boundary
                Err(_) => break,   // truncated/partial trailing entry — stop before it
            }
        }
        Ok(PackWriter {
            path: path.to_path_buf(),
            partition_id,
            file,
            len: pos,
            entry_count: count,
            offsets,
        })
    }

    /// Append one entry (dedup is the caller's responsibility — this always
    /// writes). Returns the byte offset the entry starts at.
    pub fn append(
        &mut self,
        hash: Hash,
        codec: Codec,
        uncompressed_len: u64,
        payload: &[u8],
    ) -> Result<u64> {
        let offset = self.len;
        let mut buf = Vec::with_capacity(ENTRY_META_LEN + payload.len());
        buf.extend_from_slice(hash.as_bytes());
        buf.push(codec as u8);
        buf.push(0u8); // flags, reserved
        buf.extend_from_slice(&uncompressed_len.to_le_bytes());
        buf.extend_from_slice(&(payload.len() as u64).to_le_bytes());
        buf.extend_from_slice(payload);
        self.file.write_all(&buf)?;
        self.file.flush()?;
        self.len += buf.len() as u64;
        self.entry_count += 1;
        self.offsets.insert(hash, offset);
        Ok(offset)
    }

    pub fn read_payload_at(&self, offset: u64) -> Result<(EntryMeta, Vec<u8>)> {
        let mut f = File::open(&self.path)?;
        read_entry_full_at(&mut f, offset)
    }

    /// Seal this pack: write the trailer, rename to its content-addressed
    /// final name (`<pack_content_hash>.pack`), and return the new id +
    /// path. The writer is consumed — callers open a fresh `PackWriter` for
    /// the next active pack.
    pub fn seal(mut self, packs_dir: &Path) -> Result<(Hash, PathBuf)> {
        self.file.flush()?;
        // Hash everything written so far (header + entries), excluding the
        // trailer we're about to append.
        let mut hasher = blake3::Hasher::new();
        let mut reader = File::open(&self.path)?;
        std::io::copy(&mut reader, &mut hasher)?;
        let pack_content_hash = Hash::from_bytes(*hasher.finalize().as_bytes());

        self.file.write_all(&self.entry_count.to_le_bytes())?;
        self.file.write_all(pack_content_hash.as_bytes())?;
        self.file.flush()?;

        let pack_id_hex = pack_content_hash.to_hex();
        let shard = PartitionId::shard_prefix(&pack_id_hex).to_string();
        let dest_dir = packs_dir.join(&shard);
        fs::create_dir_all(&dest_dir)?;
        let dest_path = dest_dir.join(format!("{pack_id_hex}.pack"));
        fs::rename(&self.path, &dest_path)?;

        // Build + persist the derived index alongside the sealed pack.
        let entries = rebuild_index_entries(&dest_path)?;
        write_index(
            &dest_path.with_extension("idx"),
            &self.partition_id,
            &entries,
        )?;

        Ok((pack_content_hash, dest_path))
    }

    pub fn partition_id(&self) -> PartitionId {
        self.partition_id
    }
}

fn read_entry_meta_at<R: Read + Seek>(r: &mut R, offset: u64) -> Result<Option<EntryMeta>> {
    r.seek(SeekFrom::Start(offset))?;
    let mut meta_buf = [0u8; ENTRY_META_LEN];
    match r.read_exact(&mut meta_buf) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let hash = Hash::from_bytes(meta_buf[0..32].try_into().unwrap());
    let codec = Codec::from_u8(meta_buf[32])?;
    let flags = meta_buf[33];
    let uncompressed_len = u64::from_le_bytes(meta_buf[34..42].try_into().unwrap());
    let stored_len = u64::from_le_bytes(meta_buf[42..50].try_into().unwrap());
    // Confirm the payload is actually present (not a torn write).
    let mut probe = vec![0u8; stored_len as usize];
    match r.read_exact(&mut probe) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    Ok(Some(EntryMeta {
        hash,
        codec,
        flags,
        uncompressed_len,
        stored_len,
    }))
}

fn read_entry_full_at<R: Read + Seek>(r: &mut R, offset: u64) -> Result<(EntryMeta, Vec<u8>)> {
    r.seek(SeekFrom::Start(offset))?;
    let mut meta_buf = [0u8; ENTRY_META_LEN];
    r.read_exact(&mut meta_buf)?;
    let hash = Hash::from_bytes(meta_buf[0..32].try_into().unwrap());
    let codec = Codec::from_u8(meta_buf[32])?;
    let flags = meta_buf[33];
    let uncompressed_len = u64::from_le_bytes(meta_buf[34..42].try_into().unwrap());
    let stored_len = u64::from_le_bytes(meta_buf[42..50].try_into().unwrap());
    let mut payload = vec![0u8; stored_len as usize];
    r.read_exact(&mut payload)?;
    Ok((
        EntryMeta {
            hash,
            codec,
            flags,
            uncompressed_len,
            stored_len,
        },
        payload,
    ))
}

/// Rebuild the full `(offset, meta)` list for a **sealed** pack by
/// sequential scan, ignoring/validating against the trailer if present.
pub fn rebuild_index_entries(pack_path: &Path) -> Result<Vec<(u64, EntryMeta)>> {
    let mut f = File::open(pack_path)?;
    let _partition_id = read_header(&mut f)?;
    let file_len = f.metadata()?.len();
    let scan_end = file_len.saturating_sub(TRAILER_LEN as u64);

    let mut out = Vec::new();
    let mut pos = HEADER_LEN as u64;
    while pos < scan_end {
        let entry_start = pos;
        let meta = read_entry_meta_at(&mut f, entry_start)?.ok_or_else(|| {
            BitchainError::CorruptPack(format!(
                "truncated entry at offset {entry_start} in {}",
                pack_path.display()
            ))
        })?;
        pos = entry_start + ENTRY_META_LEN as u64 + meta.stored_len;
        out.push((entry_start, meta));
    }
    if pos != scan_end {
        return Err(BitchainError::CorruptPack(format!(
            "entry scan did not land on trailer boundary in {}",
            pack_path.display()
        )));
    }
    Ok(out)
}

/// Read a sealed pack's trailer and verify `pack_content_hash` against the
/// actual bytes preceding it. Returns `(entry_count, pack_content_hash)`.
pub fn verify_trailer(pack_path: &Path) -> Result<(u64, Hash)> {
    let mut f = File::open(pack_path)?;
    let file_len = f.metadata()?.len();
    if file_len < (HEADER_LEN + TRAILER_LEN) as u64 {
        return Err(BitchainError::CorruptPack(
            "pack too short for trailer".into(),
        ));
    }
    f.seek(SeekFrom::Start(file_len - TRAILER_LEN as u64))?;
    let mut trailer = [0u8; TRAILER_LEN];
    f.read_exact(&mut trailer)?;
    let entry_count = u64::from_le_bytes(trailer[0..8].try_into().unwrap());
    let recorded_hash = Hash::from_bytes(trailer[8..40].try_into().unwrap());

    let mut hasher = blake3::Hasher::new();
    let body = File::open(pack_path)?;
    let mut limited = Read::take(body, file_len - TRAILER_LEN as u64);
    std::io::copy(&mut limited, &mut hasher)?;
    let actual_hash = Hash::from_bytes(*hasher.finalize().as_bytes());

    if actual_hash != recorded_hash {
        return Err(BitchainError::CorruptPack(format!(
            "pack_content_hash mismatch in {}: trailer says {}, computed {}",
            pack_path.display(),
            recorded_hash,
            actual_hash
        )));
    }
    Ok((entry_count, actual_hash))
}

/// Fetch and decode one entry's payload from a sealed pack at `offset`,
/// verifying the stored bytes hash to the entry's own recorded hash.
pub fn get_at(pack_path: &Path, offset: u64) -> Result<Vec<u8>> {
    let mut f = File::open(pack_path)?;
    let (meta, payload) = read_entry_full_at(&mut f, offset)?;
    let decoded = decode(meta.codec, &payload)?;
    let actual = Hash::of(&decoded);
    if actual != meta.hash {
        return Err(BitchainError::HashMismatch {
            expected: meta.hash.to_hex(),
            actual: actual.to_hex(),
        });
    }
    if decoded.len() as u64 != meta.uncompressed_len {
        return Err(BitchainError::CorruptPack(format!(
            "uncompressed_len mismatch for {}",
            meta.hash
        )));
    }
    Ok(decoded)
}

// ---------------------------------------------------------------------
// .idx — git-pack style derived index. Never authoritative; always
// rebuildable from the corresponding `.pack` via `rebuild_index_entries`.
// ---------------------------------------------------------------------

const IDX_RECORD_LEN: usize = HASH_LEN + 8 + 8 + 8 + 1; // hash | offset | uncompressed_len | stored_len | codec

pub fn write_index(
    idx_path: &Path,
    partition_id: &PartitionId,
    entries: &[(u64, EntryMeta)],
) -> Result<()> {
    let mut sorted: Vec<&(u64, EntryMeta)> = entries.iter().collect();
    sorted.sort_by_key(|(_, m)| *m.hash.as_bytes());

    let mut buf = Vec::with_capacity(4 + 4 + PARTITION_ID_LEN + 8 + sorted.len() * IDX_RECORD_LEN);
    buf.extend_from_slice(&IDX_MAGIC);
    buf.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    buf.extend_from_slice(partition_id.as_bytes());
    buf.extend_from_slice(&(sorted.len() as u64).to_le_bytes());
    for (offset, meta) in sorted {
        buf.extend_from_slice(meta.hash.as_bytes());
        buf.extend_from_slice(&offset.to_le_bytes());
        buf.extend_from_slice(&meta.uncompressed_len.to_le_bytes());
        buf.extend_from_slice(&meta.stored_len.to_le_bytes());
        buf.push(meta.codec as u8);
    }
    fs::write(idx_path, buf)?;
    Ok(())
}

/// Read a `.idx` file into a `hash -> offset` map. Callers should not treat
/// a missing/corrupt index as an error condition for the store as a whole —
/// it is always recoverable via `rebuild_index_entries`.
pub fn read_index(idx_path: &Path) -> Result<HashMap<Hash, u64>> {
    let bytes = fs::read(idx_path)?;
    if bytes.len() < 4 + 4 + PARTITION_ID_LEN + 8 {
        return Err(BitchainError::CorruptPack("idx too short".into()));
    }
    if bytes[0..4] != IDX_MAGIC {
        return Err(BitchainError::CorruptPack("bad idx magic".into()));
    }
    let count_off = 4 + 4 + PARTITION_ID_LEN;
    let count = u64::from_le_bytes(bytes[count_off..count_off + 8].try_into().unwrap()) as usize;
    let mut out = HashMap::with_capacity(count);
    let mut pos = count_off + 8;
    for _ in 0..count {
        if pos + IDX_RECORD_LEN > bytes.len() {
            return Err(BitchainError::CorruptPack("idx truncated".into()));
        }
        let hash = Hash::from_bytes(bytes[pos..pos + 32].try_into().unwrap());
        let offset = u64::from_le_bytes(bytes[pos + 32..pos + 40].try_into().unwrap());
        out.insert(hash, offset);
        pos += IDX_RECORD_LEN;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn append_seal_and_read_back() {
        let dir = tempdir().unwrap();
        let packs_dir = dir.path().join("packs");
        let partition_id = PartitionId::derive("test-partition");

        let mut writer =
            PackWriter::open_active(&dir.path().join("active/active.pack"), partition_id).unwrap();
        let data = b"hello, packfile";
        let hash = Hash::of(data);
        let (codec, payload) = encode(data).unwrap();
        let offset = writer
            .append(hash, codec, data.len() as u64, &payload)
            .unwrap();

        let (pack_id, pack_path) = writer.seal(&packs_dir).unwrap();
        assert!(pack_path.exists());

        let (entry_count, content_hash) = verify_trailer(&pack_path).unwrap();
        assert_eq!(entry_count, 1);
        assert_eq!(content_hash, pack_id);

        let round_tripped = get_at(&pack_path, offset).unwrap();
        assert_eq!(round_tripped, data);
    }

    #[test]
    fn active_pack_resumes_after_reopen() {
        let dir = tempdir().unwrap();
        let active_path = dir.path().join("active/active.pack");
        let partition_id = PartitionId::derive("resume-partition");

        {
            let mut writer = PackWriter::open_active(&active_path, partition_id).unwrap();
            let (codec, payload) = encode(b"first").unwrap();
            writer
                .append(Hash::of(b"first"), codec, 5, &payload)
                .unwrap();
        }
        let writer = PackWriter::open_active(&active_path, partition_id).unwrap();
        assert_eq!(writer.entry_count, 1);
        assert!(writer.offsets.contains_key(&Hash::of(b"first")));
    }

    #[test]
    fn index_roundtrips() {
        let dir = tempdir().unwrap();
        let packs_dir = dir.path().join("packs");
        let partition_id = PartitionId::derive("idx-partition");
        let mut writer =
            PackWriter::open_active(&dir.path().join("active/active.pack"), partition_id).unwrap();
        let (codec, payload) = encode(b"index me").unwrap();
        writer
            .append(Hash::of(b"index me"), codec, 8, &payload)
            .unwrap();
        let (_pack_id, pack_path) = writer.seal(&packs_dir).unwrap();

        let idx_path = pack_path.with_extension("idx");
        let idx = read_index(&idx_path).unwrap();
        assert_eq!(idx.len(), 1);
        assert!(idx.contains_key(&Hash::of(b"index me")));

        // Index is derived — rebuilding from the pack alone must agree.
        let rebuilt = rebuild_index_entries(&pack_path).unwrap();
        assert_eq!(rebuilt.len(), 1);
        assert_eq!(rebuilt[0].1.hash, Hash::of(b"index me"));
    }
}
