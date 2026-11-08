//! Custom HLS object AVIO. FFmpeg 9 copies the parent's callbacks and opaque
//! into its MPEG-TS child. Its HLS close/trailer paths discard some I/O errors,
//! so a sticky error is authoritative, not the muxer's return value.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    ffi::{c_char, c_int, c_void, CStr},
    io,
    ops::{Deref, DerefMut},
    panic::{catch_unwind, AssertUnwindSafe},
    ptr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use ffmpeg_next::{ffi, Dictionary, Packet};

use crate::output::NativeOutput;

// An unknown protocol is intentional: hlsenc must not select file-protocol
// rename/.tmp handling or HTTP persistent-connection internals. No default I/O
// callback is ever invoked for these URLs.
const PREFIX: &str = "hls-object://recording/";
pub(super) const PLAYLIST_URL: &str = "hls-object://recording/playlist.m3u8";
pub(super) const SEGMENT_URL: &str = "hls-object://recording/%d.ts";
const AVIO_BUFFER_SIZE: usize = 32 * 1024;

/// Synchronous, atomic object operations used exclusively on a native blocking
/// worker, never a Tokio executor thread. Keys are `playlist.m3u8` and canonical
/// nonnegative numbered `.ts` names. Implementations must not publish partial
/// writes, and must poll `cancelled` while waiting for backend operations.
///
/// A missing playlist must return `ErrorKind::NotFound`; all other errors stop
/// recording. Error messages from implementations are deliberately not exposed
/// (they may contain credentials). A recording prefix requires a single writer.
pub trait HlsObjectStore: Send + Sync {
    /// Implementations must enforce `max_object_bytes` while downloading, not
    /// only after returning the allocated vector.
    fn read(&self, key: &str, cancelled: &AtomicBool) -> io::Result<Vec<u8>>;
    fn write(&self, key: &str, bytes: &[u8], cancelled: &AtomicBool) -> io::Result<()>;

    /// Hard limit for each playlist or TS object; default 64 MiB. The event
    /// manifest therefore has a finite maximum recording length. Native HLS
    /// also buffers one TS internally; we conservatively charge packet payload
    /// expansion plus 4 KiB per packet against this limit before muxing. Very
    /// large GOPs, unusually high bitrates, or very small configured limits can
    /// stop recording before an object reaches the byte limit. No archive-wide
    /// media buffer or background upload queue is used.
    fn max_object_bytes(&self) -> usize {
        64 * 1024 * 1024
    }
}

pub(super) struct RecordingOutput {
    native: NativeOutput,
    // Declared after native: callbacks and opaque must outlive native teardown.
    objects: Option<Box<ObjectIo>>,
    header_written: bool,
    trailer_written: bool,
}

impl Deref for RecordingOutput {
    type Target = NativeOutput;
    fn deref(&self) -> &Self::Target {
        &self.native
    }
}

impl DerefMut for RecordingOutput {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.native
    }
}

impl RecordingOutput {
    pub(super) fn new(
        mut native: NativeOutput,
        store: Option<Arc<dyn HlsObjectStore>>,
        cancelled: Arc<AtomicBool>,
    ) -> Self {
        let objects = store.map(|store| {
            let mut objects = Box::new(ObjectIo {
                limit: store.max_object_bytes(),
                store,
                cancelled,
                error: RefCell::new(None),
                discard: Cell::new(false),
                next_segment: Cell::new(0),
                packet_budget: Cell::new(0),
                open: RefCell::new(HashMap::new()),
            });
            unsafe {
                let context = native.output.as_mut_ptr();
                (*context).opaque = (&mut *objects as *mut ObjectIo).cast();
                (*context).io_open = Some(io_open);
                (*context).io_close2 = Some(io_close);
            }
            objects
        });
        Self {
            native,
            objects,
            header_written: false,
            trailer_written: false,
        }
    }

    fn check(&self) -> Result<(), String> {
        if let Some(objects) = &self.objects {
            objects.check()?;
        }
        Ok(())
    }

    pub(super) fn write_header(&mut self, options: Dictionary<'_>) -> Result<(), String> {
        let result = self.native.output.write_header_with(options).map(|_| ());
        self.header_written = result.is_ok();
        self.check()?;
        result.map_err(|error| format!("写入 HLS 文件头失败: {error}"))
    }

    pub(super) fn write_packet(&mut self, packet: &Packet) -> Result<(), String> {
        // hlsenc uses an internal dynamic buffer until each segment boundary,
        // before our io_open is called. Bound that growth as well as custom
        // AVIO. 4x payload covers AVC/HEVC length-prefix expansion and TS
        // framing; the per-packet allowance covers PES/PAT/PMT/SDT overhead.
        let extradata = self
            .native
            .output
            .stream(packet.stream())
            .map_or(0, |stream| unsafe {
                (*stream.parameters().as_ptr()).extradata_size.max(0) as usize
            });
        let cost = packet
            .size()
            .saturating_add(extradata)
            .saturating_mul(4)
            .saturating_add(4096);
        if let Some(objects) = &self.objects {
            objects.reserve_packet(cost)?;
        }
        let result = packet.write_interleaved(&mut self.native.output);
        if let Some(objects) = &self.objects {
            // A boundary closes the previous segment before muxing this packet.
            objects
                .packet_budget
                .set(objects.packet_budget.get().max(cost));
        }
        self.check()?;
        result.map_err(|error| format!("写入录制分片失败: {error}"))
    }

    pub(super) fn write_trailer(&mut self) -> Result<(), String> {
        self.trailer_written = true;
        let result = self.native.output.write_trailer();
        self.check()?;
        result.map_err(|error| format!("结束 HLS 录制失败: {error}"))
    }
}

impl Drop for RecordingOutput {
    fn drop(&mut self) {
        if let Some(objects) = &self.objects {
            objects.discard.set(true);
            // HLS frees its internal segment dynamic buffer in write_trailer,
            // not in deinit. On failure/cancellation, drain only to discard I/O,
            // never to the backend. A failed header cannot safely be trailed.
            if self.header_written && !self.trailer_written {
                let _ = self.native.output.write_trailer();
            }
        }
    }
}

struct ObjectIo {
    store: Arc<dyn HlsObjectStore>,
    cancelled: Arc<AtomicBool>,
    limit: usize,
    error: RefCell<Option<String>>,
    discard: Cell<bool>,
    next_segment: Cell<u64>,
    packet_budget: Cell<usize>,
    open: RefCell<HashMap<usize, Box<Object>>>,
}

impl ObjectIo {
    fn fail(&self, message: &str) -> c_int {
        // This is also called by catch_unwind handlers: never panic on a
        // reentrant borrow while recording the original callback failure.
        if let Ok(mut error) = self.error.try_borrow_mut() {
            if error.is_none() {
                *error = Some(message.to_owned());
            }
        }
        -ffi::EIO
    }

    fn check(&self) -> Result<(), String> {
        if let Some(error) = self.error.borrow().as_ref() {
            return Err(error.clone());
        }
        if self.cancelled.load(Ordering::Acquire) {
            return Err("HLS object I/O cancelled".into());
        }
        if self.discard.get() {
            return Err("HLS object I/O discarded".into());
        }
        Ok(())
    }

    fn reserve_packet(&self, cost: usize) -> Result<(), String> {
        self.check()?;
        let budget = self.packet_budget.get().saturating_add(cost);
        if budget > self.limit {
            self.fail("HLS segment buffer limit exceeded");
            return self.check();
        }
        self.packet_budget.set(budget);
        Ok(())
    }
}

struct Object {
    owner: *const ObjectIo,
    key: String,
    bytes: Vec<u8>,
    position: usize,
    write: bool,
}

fn segment_number(key: &str) -> Option<u64> {
    let number = key.strip_suffix(".ts")?;
    let value: u64 = number.parse().ok()?;
    (value.to_string() == number && value < i64::MAX as u64).then_some(value)
}

/// Validate the restricted event format we produce before FFmpeg's permissive
/// append parser sees it. In particular, do not silently restart at zero after
/// malformed input or accept external/relative-traversal segment references.
fn playlist_next(bytes: &[u8]) -> Result<u64, &'static str> {
    let text = std::str::from_utf8(bytes).map_err(|_| "Invalid HLS object playlist")?;
    let mut lines = text.lines();
    if lines.next() != Some("#EXTM3U") {
        return Err("Invalid HLS object playlist");
    }
    let mut next = None;
    let mut duration = false;
    for line in lines {
        if line.len() > 1023 || line.contains('\0') {
            return Err("Invalid HLS object playlist line");
        }
        if let Some(sequence) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            if next.is_some() {
                return Err("Invalid HLS object playlist sequence");
            }
            next = Some(
                sequence
                    .parse::<u64>()
                    .map_err(|_| "Invalid HLS object playlist sequence")?,
            );
        } else if let Some(value) = line.strip_prefix("#EXTINF:") {
            let value = value
                .split(',')
                .next()
                .unwrap_or("")
                .parse::<f64>()
                .map_err(|_| "Invalid HLS object playlist duration")?;
            if duration || !value.is_finite() || value < 0.0 {
                return Err("Invalid HLS object playlist duration");
            }
            duration = true;
        } else if line.is_empty()
            || line == "#EXT-X-DISCONTINUITY"
            || line == "#EXT-X-ENDLIST"
            || line == "#EXT-X-PLAYLIST-TYPE:EVENT"
            || line == "#EXT-X-PLAYLIST-TYPE:VOD"
            || line.starts_with("#EXT-X-VERSION:")
            || line.starts_with("#EXT-X-TARGETDURATION:")
            || line.starts_with("#EXT-X-ALLOW-CACHE:")
        {
            continue;
        } else {
            let number = segment_number(line).ok_or("Unsupported HLS object playlist entry")?;
            // MEDIA-SEQUENCE defaults to zero in HLS; m3u8-rs omits it
            // when serializing both new and closed zero-based archives.
            if !duration || next.unwrap_or(0) != number {
                return Err("Invalid HLS object playlist numbering");
            }
            next = number.checked_add(1);
            duration = false;
        }
    }
    if duration || next.is_some_and(|n| n >= i64::MAX as u64) {
        return Err("Invalid HLS object playlist ending");
    }
    Ok(next.unwrap_or(0))
}

unsafe extern "C" fn io_open(
    context: *mut ffi::AVFormatContext,
    pb: *mut *mut ffi::AVIOContext,
    url: *const c_char,
    flags: c_int,
    _options: *mut *mut ffi::AVDictionary,
) -> c_int {
    let owner = &*((*context).opaque.cast::<ObjectIo>());
    *pb = ptr::null_mut();
    catch_unwind(AssertUnwindSafe(|| open_object(owner, pb, url, flags)))
        .unwrap_or_else(|_| owner.fail("HLS object open callback panicked"))
}

unsafe fn open_object(
    owner: &ObjectIo,
    pb: *mut *mut ffi::AVIOContext,
    url: *const c_char,
    flags: c_int,
) -> c_int {
    if owner.check().is_err() {
        return -ffi::EIO;
    }
    let key = CStr::from_ptr(url)
        .to_str()
        .ok()
        .and_then(|url| url.strip_prefix(PREFIX));
    let Some(key) = key.filter(|key| *key == "playlist.m3u8" || segment_number(key).is_some())
    else {
        return owner.fail("Rejected HLS object URL");
    };
    let write = flags == ffi::AVIO_FLAG_WRITE;
    if !write && (flags != ffi::AVIO_FLAG_READ || key != "playlist.m3u8") {
        return owner.fail("Rejected HLS object I/O mode");
    }
    if write && key != "playlist.m3u8" && segment_number(key) != Some(owner.next_segment.get()) {
        return owner.fail("Rejected HLS segment overwrite or out-of-order write");
    }
    let bytes = if write {
        Vec::new()
    } else {
        match owner.store.read(key, &owner.cancelled) {
            Ok(bytes) => {
                if bytes.len() > owner.limit {
                    return owner.fail("HLS playlist object limit exceeded");
                }
                match playlist_next(&bytes) {
                    Ok(next) => owner.next_segment.set(next),
                    Err(error) => return owner.fail(error),
                }
                bytes
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => return -ffi::ENOENT,
            Err(_) => return owner.fail("HLS playlist object read failed"),
        }
    };
    let mut object = Box::new(Object {
        owner,
        key: key.to_owned(),
        bytes,
        position: 0,
        write,
    });
    let buffer = ffi::av_malloc(AVIO_BUFFER_SIZE).cast::<u8>();
    if buffer.is_null() {
        return owner.fail("HLS AVIO buffer allocation failed");
    }
    let io = ffi::avio_alloc_context(
        buffer,
        AVIO_BUFFER_SIZE as c_int,
        i32::from(write),
        (&mut *object as *mut Object).cast(),
        if write { None } else { Some(read_packet) },
        if write { Some(write_packet) } else { None },
        None,
    );
    if io.is_null() {
        ffi::av_free(buffer.cast());
        return owner.fail("HLS AVIO allocation failed");
    }
    owner.open.borrow_mut().insert(io as usize, object);
    *pb = io;
    0
}

unsafe extern "C" fn read_packet(opaque: *mut c_void, buf: *mut u8, size: c_int) -> c_int {
    let object = &mut *opaque.cast::<Object>();
    let owner = &*object.owner;
    catch_unwind(AssertUnwindSafe(|| {
        if owner.check().is_err() {
            return -ffi::EIO;
        }
        let count = (size as usize).min(object.bytes.len() - object.position);
        if count == 0 {
            return ffi::AVERROR_EOF;
        }
        ptr::copy_nonoverlapping(object.bytes.as_ptr().add(object.position), buf, count);
        object.position += count;
        count as c_int
    }))
    .unwrap_or_else(|_| owner.fail("HLS object read callback panicked"))
}

unsafe extern "C" fn write_packet(opaque: *mut c_void, buf: *const u8, size: c_int) -> c_int {
    let object = &mut *opaque.cast::<Object>();
    let owner = &*object.owner;
    catch_unwind(AssertUnwindSafe(|| {
        if owner.check().is_err() {
            return -ffi::EIO;
        }
        let count = size as usize;
        if object.bytes.len().saturating_add(count) > owner.limit {
            return owner.fail("HLS object buffer limit exceeded");
        }
        if object.bytes.try_reserve_exact(count).is_err() {
            return owner.fail("HLS object buffer allocation failed");
        }
        object
            .bytes
            .extend_from_slice(std::slice::from_raw_parts(buf, count));
        size
    }))
    .unwrap_or_else(|_| owner.fail("HLS object write callback panicked"))
}

unsafe extern "C" fn io_close(
    context: *mut ffi::AVFormatContext,
    io: *mut ffi::AVIOContext,
) -> c_int {
    let owner = &*((*context).opaque.cast::<ObjectIo>());
    catch_unwind(AssertUnwindSafe(|| {
        let object = owner.open.borrow_mut().remove(&(io as usize));
        let Some(object) = object else {
            return owner.fail("Unknown HLS AVIO close");
        };
        // A backend panic still frees both AVIO and opaque; keep the object
        // alive until after AVIO has been flushed and freed.
        let allocation = OpenAllocation { io, object };
        if !allocation.object.write {
            return 0;
        }
        ffi::avio_flush(io);
        if (*io).error < 0 {
            return owner.fail("HLS AVIO flush failed");
        }
        if owner.check().is_err() {
            return -ffi::EIO;
        }
        let object = &allocation.object;
        if object.key == "playlist.m3u8" {
            match playlist_next(&object.bytes) {
                Ok(next) if next == owner.next_segment.get() => (),
                _ => return owner.fail("HLS playlist references uncommitted segments"),
            }
        }
        if owner
            .store
            .write(&object.key, &object.bytes, &owner.cancelled)
            .is_err()
        {
            return owner.fail(if object.key == "playlist.m3u8" {
                "HLS playlist object write failed"
            } else {
                "HLS segment object write failed"
            });
        }
        if object.key != "playlist.m3u8" {
            owner.next_segment.set(owner.next_segment.get() + 1);
            owner.packet_budget.set(0);
        }
        0
    }))
    .unwrap_or_else(|_| owner.fail("HLS object close callback panicked"))
}

struct OpenAllocation {
    io: *mut ffi::AVIOContext,
    object: Box<Object>,
}

impl Drop for OpenAllocation {
    fn drop(&mut self) {
        unsafe { free_io(self.io) };
    }
}

unsafe fn free_io(mut io: *mut ffi::AVIOContext) {
    // avio_close[p] assumes opaque is a URLContext and calls ffurl_close:
    // it must NEVER be used on custom AVIO. avio_context_free alone does not
    // free the (possibly replaced) buffer.
    ffi::av_free((*io).buffer.cast());
    (*io).buffer = ptr::null_mut();
    ffi::avio_context_free(&mut io);
}

impl Drop for ObjectIo {
    fn drop(&mut self) {
        // hlsenc error/deinit paths do not close every VariantStream output.
        // Registry ownership lets us reclaim those contexts without flushing
        // or publishing incomplete data.
        for (io, _object) in self.open.get_mut().drain() {
            unsafe { free_io(io as *mut ffi::AVIOContext) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{ffi::CString, path::Path, sync::atomic::AtomicUsize};

    #[test]
    fn initialized_and_closed_playlists_allow_implicit_zero_sequence() {
        assert_eq!(
            playlist_next(b"#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXT-X-PLAYLIST-TYPE:EVENT\n"),
            Ok(0)
        );
        assert_eq!(
            playlist_next(
                b"#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXT-X-PLAYLIST-TYPE:VOD\n\
                  #EXTINF:4,\n0.ts\n#EXT-X-ENDLIST\n"
            ),
            Ok(1)
        );
        // Omitting the sequence does not authorize arbitrary numbering.
        assert!(playlist_next(b"#EXTM3U\n#EXTINF:4,\n9.ts\n").is_err());
    }

    #[derive(Default)]
    struct Store(AtomicUsize);
    impl HlsObjectStore for Store {
        fn read(&self, _: &str, _: &AtomicBool) -> io::Result<Vec<u8>> {
            Err(io::ErrorKind::NotFound.into())
        }
        fn write(&self, _: &str, _: &[u8], _: &AtomicBool) -> io::Result<()> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
        fn max_object_bytes(&self) -> usize {
            64
        }
    }

    fn output(store: Arc<Store>) -> RecordingOutput {
        crate::metadata::initialize().unwrap();
        let cancelled = Arc::new(AtomicBool::new(false));
        let native = NativeOutput::new(Path::new(PLAYLIST_URL), "hls", cancelled.clone()).unwrap();
        RecordingOutput::new(native, Some(store), cancelled)
    }

    #[test]
    fn unsafe_urls_and_modes_never_fall_back_to_protocol_io() {
        for (url, flags) in [
            ("file:/tmp/should-not-exist.ts", ffi::AVIO_FLAG_WRITE),
            ("hls-object://other/0.ts", ffi::AVIO_FLAG_WRITE),
            ("hls-object://recording/../0.ts", ffi::AVIO_FLAG_WRITE),
            ("hls-object://recording/0.ts.tmp", ffi::AVIO_FLAG_WRITE),
            ("hls-object://recording/00.ts", ffi::AVIO_FLAG_WRITE),
            ("hls-object://recording/0.ts", ffi::AVIO_FLAG_READ),
            (PLAYLIST_URL, ffi::AVIO_FLAG_READ_WRITE),
        ] {
            let store = Arc::new(Store::default());
            let mut output = output(store.clone());
            let mut io = ptr::null_mut();
            let url = CString::new(url).unwrap();
            assert!(
                unsafe {
                    io_open(
                        output.output.as_mut_ptr(),
                        &mut io,
                        url.as_ptr(),
                        flags,
                        ptr::null_mut(),
                    )
                } < 0
            );
            assert!(io.is_null());
            assert!(output.check().is_err());
            assert_eq!(store.0.load(Ordering::Relaxed), 0);
        }
    }

    #[test]
    fn close_checks_buffered_errors_and_drop_discards_open_objects() {
        for close in [true, false] {
            let store = Arc::new(Store::default());
            let mut output = output(store.clone());
            unsafe {
                let context = output.output.as_mut_ptr();
                let mut io = ptr::null_mut();
                let url = CString::new("hls-object://recording/0.ts").unwrap();
                assert_eq!(
                    io_open(
                        context,
                        &mut io,
                        url.as_ptr(),
                        ffi::AVIO_FLAG_WRITE,
                        ptr::null_mut()
                    ),
                    0
                );
                ffi::avio_write(io, [0_u8; 128].as_ptr(), 128);
                // Still buffered in AVIO, not yet delivered to write_packet.
                assert_eq!((*io).error, 0);
                if close {
                    assert!(io_close(context, io) < 0);
                    assert!(output.check().unwrap_err().contains("limit exceeded"));
                    assert!(output.objects.as_ref().unwrap().open.borrow().is_empty());
                }
            }
            drop(output);
            assert_eq!(store.0.load(Ordering::Relaxed), 0);
        }
    }
}
