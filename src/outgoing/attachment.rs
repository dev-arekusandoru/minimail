//! Managed attachments: files are copied into a per-draft store once, and the
//! copy — not the original — is what a draft sends.
//!
//! Objects are content-addressed and never overwritten, so two drafts can share
//! one file and moving or editing the original afterwards cannot break either
//! draft. Nothing is deleted while another draft still references it.

use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Identity of one MIME part: the stored object's content hash, plus a hash of
/// everything that makes *this* part distinct from another — its file name,
/// its declared media type and its inline id. Every field is length-prefixed
/// into the hash, so no file name, type or id can be crafted to collide with
/// another part's (a name containing the separator used to truncate one).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AttachmentId(pub String);

impl AttachmentId {
    /// The content hash, which decides which stored object is used.
    pub fn digest(&self) -> &str {
        self.0.split_once(':').map_or(&self.0, |(digest, _)| digest)
    }
}

impl fmt::Display for AttachmentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where a managed copy came from: attached from disk, carried over by a
/// forward, or produced by the app.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttachmentSource {
    Added { origin: PathBuf },
    Forwarded { message: String, part: String },
    Generated,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedAttachment {
    pub id: AttachmentId,
    pub file_name: String,
    /// Path of the copy relative to the store root.
    pub rel: String,
    /// Absolute path of the copy; readable without the store.
    pub stored: PathBuf,
    pub size: u64,
    pub content_type: String,
    pub sha256: String,
    pub source: AttachmentSource,
    /// `Content-ID` of a part the body references as `cid:`, for a part that
    /// came from (or is destined for) an inline image. `None` for an ordinary
    /// attachment the body never points at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_id: Option<String>,
}

impl ManagedAttachment {
    pub fn read(&self) -> io::Result<Vec<u8>> {
        fs::read(&self.stored)
    }

    /// Mark this copy as an inline part under `content_id`, which may carry the
    /// surrounding angle brackets but nothing else that could break out of the
    /// header. A refused id is an error rather than a quiet correction, so the
    /// body's `cid:` reference can never end up pointing somewhere else.
    ///
    /// Two inline parts of one imported body can share bytes and file name and
    /// still be different parts, so the identity carries the id while the
    /// stored object stays shared.
    pub fn with_content_id(self, content_id: Option<String>) -> io::Result<Self> {
        let Some(raw) = content_id else {
            return Ok(self.clear_content_id());
        };
        let trimmed = raw.trim();
        let candidate = trimmed
            .strip_prefix('<')
            .and_then(|rest| rest.strip_suffix('>'))
            .unwrap_or(trimmed);
        if !valid_content_id(candidate) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{raw:?} is not a usable Content-ID"),
            ));
        }
        let cid = candidate.to_owned();
        let mut this = self;
        this.content_id = Some(cid);
        this.id = part_identity(&this);
        Ok(this)
    }

    /// The same part as an ordinary attachment.
    pub fn clear_content_id(mut self) -> Self {
        self.content_id = None;
        self.id = part_identity(&self);
        self
    }
}

/// The identity of a part: its object's hash, then a hash of the fields that
/// tell this part from another. Two parts of one message may share bytes and a
/// file name and still be different parts — a different declared media type or
/// a different `Content-ID` makes them so — so both differences are part of
/// the identity rather than metadata that one of them silently overwrites.
fn part_identity(attachment: &ManagedAttachment) -> AttachmentId {
    let mut hasher = Sha256::new();
    for field in [
        attachment.file_name.as_str(),
        attachment.content_type.as_str(),
        attachment.content_id.as_deref().unwrap_or(""),
    ] {
        hasher.update((field.len() as u64).to_be_bytes());
        hasher.update(field.as_bytes());
    }
    AttachmentId(format!("{}:{}", attachment.sha256, hex(&hasher.finalize())))
}

/// A `Content-ID` lands in a header between angle brackets: it must be one
/// non-empty id, with no whitespace, control character or bracket of its own.
pub fn valid_content_id(cid: &str) -> bool {
    !cid.is_empty()
        && cid
            .chars()
            .all(|c| !c.is_whitespace() && !c.is_control() && c != '<' && c != '>')
}

/// Hashes to keep alive: every attachment any draft still references.
pub fn keep_hashes<'a>(
    attachments: impl IntoIterator<Item = &'a ManagedAttachment>,
) -> HashSet<String> {
    attachments.into_iter().map(|a| a.sha256.clone()).collect()
}

pub struct AttachmentStore {
    root: PathBuf,
}

impl AttachmentStore {
    /// Open (creating if needed) the store at `root`.
    pub fn open(root: &Path) -> io::Result<Self> {
        fs::create_dir_all(root.join("objects"))?;
        Ok(Self {
            root: root.to_path_buf(),
        })
    }

    /// `~/Library/Application Support/mail-classifier/drafts`, or whatever
    /// `$MAIL_CLASSIFIER_MAIL_DB` sits next to.
    pub fn default_root() -> PathBuf {
        crate::sync::cache::default_cache_path()
            .parent()
            .map_or_else(|| PathBuf::from("drafts"), |p| p.join("drafts"))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Copy a file into the store. The original is left exactly where it is:
    /// moving or editing it afterwards cannot change the draft.
    pub fn import_file(&self, source: &Path) -> io::Result<ManagedAttachment> {
        let metadata = fs::metadata(source)?;
        if !metadata.is_file() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a file"));
        }
        let bytes = fs::read(source)?;
        self.import_bytes(
            &file_name_of(source),
            &bytes,
            AttachmentSource::Added {
                origin: source.to_path_buf(),
            },
            None,
        )
    }

    /// Store bytes (a forwarded attachment, a file written by the app) as an
    /// immutable object. Storing the same content twice reuses one object.
    pub fn import_bytes(
        &self,
        file_name: &str,
        bytes: &[u8],
        source: AttachmentSource,
        content_type: Option<&str>,
    ) -> io::Result<ManagedAttachment> {
        let file_name = sanitize_file_name(file_name);
        let sha256 = hex(&Sha256::digest(bytes));
        let rel = format!("objects/{}/{}", &sha256[..2], sha256);
        let stored = self.root.join(&rel);
        if let Some(parent) = stored.parent() {
            fs::create_dir_all(parent)?;
        }
        // Publish only a complete object: a partial write (a full disk) must
        // not leave a file that a later import would mistake for the real one.
        match fs::metadata(&stored) {
            // An object of the right size for this content is the object.
            Ok(meta) if meta.len() as usize == bytes.len() => {}
            _ => publish(&stored, bytes)?,
        }
        let content_type = declared_content_type(content_type, &file_name);
        let attachment = ManagedAttachment {
            id: AttachmentId(String::new()),
            file_name,
            rel,
            stored,
            size: bytes.len() as u64,
            content_type,
            sha256,
            source,
            content_id: None,
        };
        let id = part_identity(&attachment);
        Ok(ManagedAttachment { id, ..attachment })
    }

    /// Read a managed copy, refusing anything outside this store.
    pub fn read(&self, attachment: &ManagedAttachment) -> io::Result<Vec<u8>> {
        let path = self.root.join(&attachment.rel);
        if !path.starts_with(&self.root) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "attachment outside store",
            ));
        }
        fs::read(path)
    }

    /// Delete every stored object no live draft references. Returns how many
    /// objects went away.
    pub fn prune(&self, keep: &HashSet<String>) -> io::Result<usize> {
        let objects = self.root.join("objects");
        let mut removed = 0;
        let Ok(shards) = fs::read_dir(&objects) else {
            return Ok(0);
        };
        for shard in shards.flatten() {
            let Ok(files) = fs::read_dir(shard.path()) else {
                continue;
            };
            for file in files.flatten() {
                let name = file.file_name();
                let Some(name) = name.to_str() else {
                    continue;
                };
                if keep.contains(name) || name.starts_with(".incoming-") {
                    continue;
                }
                if fs::remove_file(file.path()).is_ok() {
                    removed += 1;
                }
            }
            // A shard with nothing left in it is removed too.
            let _ = fs::remove_dir(shard.path());
        }
        Ok(removed)
    }
}

/// Write `bytes` to a temporary file in the same directory and move it into
/// place, so the object only ever appears complete.
fn publish(stored: &Path, bytes: &[u8]) -> io::Result<()> {
    use io::Write as _;
    let dir = stored.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "attachment object has no directory",
        )
    })?;
    let temp = dir.join(format!(
        ".incoming-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    let write = || -> io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        let result = file.write_all(bytes).and_then(|()| file.sync_all());
        drop(file);
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
    };
    write()?;
    if let Err(e) = fs::rename(&temp, stored) {
        let _ = fs::remove_file(&temp);
        return Err(e);
    }
    Ok(())
}

fn unique_suffix() -> u128 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let clock = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    clock.wrapping_add(u128::from(n))
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn file_name_of(path: &Path) -> String {
    path.file_name().map_or_else(
        || "attachment".to_owned(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// A name safe to put in a header and on disk: no directories, no controls.
pub fn sanitize_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let cleaned: String = base
        .chars()
        .map(|c| if c.is_control() || c == '"' { '_' } else { c })
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.').trim();
    if cleaned.is_empty() {
        "attachment".to_owned()
    } else {
        cleaned.to_owned()
    }
}

/// The media type a part declares, made usable: the bare `type/subtype` a
/// client reads, with any parameters dropped. A declaration is authoritative
/// over the file name — an extensionless or mislabeled file still goes out as
/// what the provider said it is — but one that is not a media type at all is
/// not trusted, and the name is asked instead.
pub fn declared_content_type(declared: Option<&str>, file_name: &str) -> String {
    let guessed = || guess_content_type(file_name).to_owned();
    let Some(declared) = declared.map(str::trim).filter(|d| !d.is_empty()) else {
        return guessed();
    };
    let bare = declared.split(';').next().unwrap_or(declared).trim();
    let (kind, subtype) = match bare.split_once('/') {
        Some(parts) => parts,
        None => return guessed(),
    };
    let is_token = |token: &str| {
        !token.is_empty()
            && token
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "!#$&^_.+-".contains(c))
    };
    if is_token(kind) && is_token(subtype) {
        bare.to_ascii_lowercase()
    } else {
        guessed()
    }
}

/// A content type good enough for a mail client, defaulting to binary.
pub fn guess_content_type(name: &str) -> &'static str {
    let ext = name
        .rsplit_once('.')
        .map_or("", |(_, e)| e)
        .to_ascii_lowercase();
    match ext.as_str() {
        "txt" | "log" | "md" | "csv" | "tsv" => "text/plain",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "json" => "application/json",
        "xml" => "application/xml",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "zip" => "application/zip",
        "gz" => "application/gzip",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "ics" => "text/calendar",
        "vcf" => "text/vcard",
        "eml" => "message/rfc822",
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "patch" | "diff" => "text/x-diff",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mail-classifier-attach-{tag}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn imported(tag: &str, name: &str, bytes: &[u8]) -> ManagedAttachment {
        AttachmentStore::open(&temp_dir(tag))
            .expect("store")
            .import_bytes(name, bytes, AttachmentSource::Generated, None)
            .expect("import")
    }

    #[test]
    fn importing_copies_the_file_and_leaves_the_original_alone() {
        let dir = temp_dir("copy");
        let store = AttachmentStore::open(&dir.join("store")).expect("store");
        let original = dir.join("original.txt");
        fs::write(&original, b"hello").expect("write");

        let attachment = store.import_file(&original).expect("import");
        fs::write(&original, b"changed after the fact").expect("rewrite");

        assert_eq!(attachment.read().expect("read"), b"hello");
        assert_eq!(store.read(&attachment).expect("read"), b"hello");
        assert_eq!(
            fs::read(&original).expect("read"),
            b"changed after the fact"
        );
        assert!(attachment.stored != original);
    }

    #[test]
    fn identical_content_is_stored_once_and_shared() {
        let dir = temp_dir("dedupe");
        let store = AttachmentStore::open(&dir).expect("store");
        let a = store
            .import_bytes("a.txt", b"same", AttachmentSource::Generated, None)
            .expect("a");
        let b = store
            .import_bytes("b.txt", b"same", AttachmentSource::Generated, None)
            .expect("b");
        assert_eq!(a.stored, b.stored);
        assert_ne!(a.id, b.id, "the name is part of the id");
        assert_eq!(a.sha256, b.sha256);
    }

    #[test]
    fn forwarded_bytes_keep_their_provenance() {
        let dir = temp_dir("forward");
        let store = AttachmentStore::open(&dir).expect("store");
        let a = store
            .import_bytes(
                "report.pdf",
                b"%PDF",
                AttachmentSource::Forwarded {
                    message: "m1".to_owned(),
                    part: "1.2".to_owned(),
                },
                None,
            )
            .expect("import");
        assert_eq!(a.content_type, "application/pdf");
        assert!(matches!(a.source, AttachmentSource::Forwarded { .. }));
    }

    #[test]
    fn file_names_are_reduced_to_something_safe() {
        assert_eq!(sanitize_file_name("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_file_name("  "), "attachment");
        assert_eq!(sanitize_file_name("a\"b.txt"), "a_b.txt");
    }

    #[test]
    fn prune_keeps_anything_a_draft_still_references() {
        let dir = temp_dir("prune");
        let store = AttachmentStore::open(&dir).expect("store");
        let kept = store
            .import_bytes("keep.txt", b"keep", AttachmentSource::Generated, None)
            .expect("keep");
        let orphan = store
            .import_bytes("drop.txt", b"drop", AttachmentSource::Generated, None)
            .expect("drop");

        assert_eq!(store.prune(&keep_hashes([&kept])).expect("prune"), 1);
        assert!(kept.stored.exists());
        assert!(!orphan.stored.exists());
    }

    #[test]
    fn reading_outside_the_store_is_refused() {
        let dir = temp_dir("escape");
        let store = AttachmentStore::open(&dir).expect("store");
        let mut attachment = store
            .import_bytes("ok.txt", b"ok", AttachmentSource::Generated, None)
            .expect("import");
        attachment.rel = "../../../etc/passwd".to_owned();
        assert!(store.read(&attachment).is_err());
    }

    #[test]
    fn a_truncated_object_is_replaced_not_trusted() {
        let dir = temp_dir("truncated");
        let store = AttachmentStore::open(&dir).expect("store");
        let good = store
            .import_bytes("a.txt", b"complete", AttachmentSource::Generated, None)
            .expect("import");
        // A partial write from an older run left the object short.
        fs::write(&good.stored, b"comp").expect("truncate");

        let again = store
            .import_bytes("a.txt", b"complete", AttachmentSource::Generated, None)
            .expect("import");
        assert_eq!(again.stored, good.stored);
        assert_eq!(again.read().expect("read"), b"complete");
        let leftovers: Vec<_> = fs::read_dir(again.stored.parent().expect("dir"))
            .expect("dir")
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(".incoming-"))
            .collect();
        assert!(leftovers.is_empty(), "no half-written file is left behind");
    }

    #[test]
    fn a_content_id_is_accepted_or_refused_outright() {
        let plain = imported("cid", "a.png", b"x");
        let inline = plain
            .clone()
            .with_content_id(Some("<id@x.test>".to_owned()))
            .expect("a plain id is accepted");
        assert_eq!(inline.content_id.as_deref(), Some("id@x.test"));
        assert_eq!(
            inline.id.digest(),
            plain.sha256,
            "the object is still shared"
        );

        for bad in [
            "a\r\nX-Evil: 1@x.test",
            "two ids@x.test",
            "brackets<inside>@x.test",
            "",
            "   ",
        ] {
            assert!(
                plain.clone().with_content_id(Some(bad.to_owned())).is_err(),
                "{bad:?} must not become a Content-ID"
            );
        }
        assert!(
            plain
                .clone()
                .with_content_id(None)
                .expect("clear")
                .content_id
                .is_none()
        );
    }

    #[test]
    fn two_inline_parts_of_the_same_file_stay_distinct() {
        // One store: the parts share an object because their bytes match, and
        // are distinct parts because their Content-IDs differ.
        let store = AttachmentStore::open(&temp_dir("dual-cid")).expect("store");
        let first = store
            .import_bytes("logo.png", b"same bytes", AttachmentSource::Generated, None)
            .expect("one")
            .with_content_id(Some("one@x.test".to_owned()))
            .expect("one is inline");
        let second = store
            .import_bytes("logo.png", b"same bytes", AttachmentSource::Generated, None)
            .expect("two")
            .with_content_id(Some("two@x.test".to_owned()))
            .expect("two is inline");

        assert_ne!(first.id, second.id, "each part is its own attachment");
        assert_eq!(
            first.stored, second.stored,
            "the bytes are still stored once"
        );
        assert_eq!(keep_hashes([&first, &second]).len(), 1);

        // Dropping one part leaves the other referenced and its bytes alive.
        let mut record = crate::outgoing::draft::DraftRecord::new_email("me@x.test", 1);
        assert!(record.attach(first.clone()));
        assert!(record.attach(second.clone()));
        assert!(record.detach(&second.id));
        assert_eq!(record.attachments.len(), 1);
        assert_eq!(record.attachments[0].id, first.id);
        assert_eq!(keep_hashes([&record.attachments[0]]).len(), 1);
    }

    #[test]
    fn a_declared_media_type_beats_a_mislabeled_or_extensionless_name() {
        let store = AttachmentStore::open(&temp_dir("declared")).expect("store");
        // Gmail knows what these bytes are; the file name says nothing useful
        // and in one case actively disagrees.
        let image = store
            .import_bytes(
                "notes",
                b"\x89PNG",
                AttachmentSource::Forwarded {
                    message: "m1".to_owned(),
                    part: "0.1".to_owned(),
                },
                Some("image/png"),
            )
            .expect("import");
        assert_eq!(image.content_type, "image/png");

        let mislabeled = store
            .import_bytes(
                "photo.txt",
                b"%PDF",
                AttachmentSource::Forwarded {
                    message: "m1".to_owned(),
                    part: "0.2".to_owned(),
                },
                Some("application/pdf"),
            )
            .expect("import");
        assert_eq!(
            mislabeled.content_type, "application/pdf",
            "the name must not win"
        );

        // Parameters are not the media type and are dropped, but the type is
        // kept; a declaration that is not a media type at all falls back to
        // the name rather than being trusted into a header.
        let declared = store
            .import_bytes(
                "a.bin",
                b"x",
                AttachmentSource::Generated,
                Some("text/plain; charset=utf-8"),
            )
            .expect("import");
        assert_eq!(declared.content_type, "text/plain");
        let junk = store
            .import_bytes(
                "b.txt",
                b"x",
                AttachmentSource::Generated,
                Some("not a media type"),
            )
            .expect("import");
        assert_eq!(junk.content_type, "text/plain");
        // Nothing declared is the file name's to answer, not a fixed default.
        let unnamed = store
            .import_bytes("c.jpg", b"x", AttachmentSource::Generated, Some(""))
            .expect("import");
        assert_eq!(unnamed.content_type, "image/jpeg");
    }

    #[test]
    fn same_bytes_and_name_stay_distinct_parts_when_the_type_differs() {
        // A draft that carries one image twice, declared under two types,
        // must keep both: collapsing them would send the wrong type for one of
        // them and silently drop the other's declaration.
        let store = AttachmentStore::open(&temp_dir("dual-type")).expect("store");
        let png = store
            .import_bytes(
                "logo",
                b"same bytes",
                AttachmentSource::Generated,
                Some("image/png"),
            )
            .expect("png");
        let jpeg = store
            .import_bytes(
                "logo",
                b"same bytes",
                AttachmentSource::Generated,
                Some("image/jpeg"),
            )
            .expect("jpeg");

        assert_ne!(
            png.id, jpeg.id,
            "a different declared type is a different part"
        );
        assert_eq!(png.stored, jpeg.stored, "the bytes are still stored once");

        let mut record = crate::outgoing::draft::DraftRecord::new_email("me@x.test", 1);
        assert!(record.attach(png.clone()));
        assert!(
            record.attach(jpeg.clone()),
            "the second declaration was silently dropped"
        );
        assert_eq!(record.attachments.len(), 2);
        assert!(record.detach(&png.id));
        assert_eq!(record.attachments[0].content_type, "image/jpeg");
        // Asking for the same part twice is still the same part.
        assert!(!record.attach(jpeg.clone()));
    }

    #[test]
    fn a_name_containing_the_identity_separator_cannot_collide() {
        let store = AttachmentStore::open(&temp_dir("separator")).expect("store");
        // Under the old `sha:name#cid` shape, this name carried the separator
        // and truncation made these two parts the same identity.
        let plain = store
            .import_bytes("a#b.txt", b"x", AttachmentSource::Generated, None)
            .expect("a");
        let inline = plain
            .clone()
            .with_content_id(Some("cid@x.test".to_owned()))
            .expect("inline");
        assert_ne!(plain.id, inline.id);
        // Clearing the id gives back exactly the ordinary attachment's
        // identity, rather than a truncated prefix of its name.
        assert_eq!(inline.clone().clear_content_id().id, plain.id);
        // Two names that differ only past a separator stay distinct too.
        let other = store
            .import_bytes("a", b"x", AttachmentSource::Generated, None)
            .expect("b");
        assert_ne!(plain.id, other.id);
    }
}
