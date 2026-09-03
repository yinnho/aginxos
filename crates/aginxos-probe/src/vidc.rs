//! M41 bring-up: Venus (msm-vidc) stateful M2M H264 → NV12 decode probe.
//!
//! Subcommands:
//!   vidc caps                          — QUERYCAP every /dev/videoN
//!   vidc sizes [node]                  — ioctl size-bit sweep (ABI archaeology)
//!   vidc ion                           — enumerate ION heaps
//!   vidc decode <in.h264> <out> [n] [heapmask]
//!                                      — decode n frames (default 3),
//!                                        dumps <out>.<i>.p0/.p1 planes
//!
//! Two ABI facts measured on this kernel (android-msm-redbull-4.19, see the
//! `sizes` sweep): sizeof(struct v4l2_format) is 208 (vendor +4 vs mainline)
//! and vidioc_querybuf is NOT wired — msm-vidc has no .mmap either, so the
//! MMAP memory model is unusable. The working allocation path (msm_vidc.c
//! vb2_bufq_init: io_modes = VB2_MMAP | VB2_USERPTR, get_userptr stub) is
//! ION + V4L2_MEMORY_USERPTR: userspace allocates dma-bufs on /dev/ion, mmaps
//! them itself, and passes the fd per plane in planes[].reserved[
//! MSM_VIDC_BUFFER_FD] — msm_vidc_qbuf copies reserved[0]→m.fd before vb2
//! sees it. Struct layouts below are transcribed from the kernel UAPI
//! (aarch64 LP64) — do not "fix" field order or padding.

use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;

// ---------------------------------------------------------------------------
// ioctl request encoding (asm-generic, dir<<30 | size<<16 | type<<8 | nr)
// ---------------------------------------------------------------------------

const fn ioc(dir: u32, ty: u8, nr: u8, size: u32) -> u32 {
    (dir << 30) | (size << 16) | ((ty as u32) << 8) | nr as u32
}

const VIDIOC_QUERYCAP: u32 = ioc(2, b'V', 0, 104);
const VIDIOC_G_FMT: u32 = ioc(3, b'V', 4, 208);
const VIDIOC_S_FMT: u32 = ioc(3, b'V', 5, 208);
const VIDIOC_REQBUFS: u32 = ioc(3, b'V', 8, 20);
const VIDIOC_QBUF: u32 = ioc(3, b'V', 15, 88);
const VIDIOC_DQBUF: u32 = ioc(3, b'V', 17, 88);
const VIDIOC_STREAMON: u32 = ioc(1, b'V', 18, 4);
const VIDIOC_STREAMOFF: u32 = ioc(1, b'V', 19, 4);
// cmd words verified by compiling the real android13 branch header:
// SUBSCRIBE_EVENT 0x4020565a (32B), DQEVENT 0x80885659 (136B)
const VIDIOC_SUBSCRIBE_EVENT: u32 = ioc(1, b'V', 90, 32);
const VIDIOC_DQEVENT: u32 = ioc(2, b'V', 91, 136);
const EVENT_SOURCE_CHANGE: u32 = 5;

// ION uapi per redbull drivers/staging/android/uapi/ion.h (Qualcomm
// downstream): ALLOC is nr *0* (not the AOSP 5), heap_query is 24 bytes,
// heap names 32 bytes.
const ION_IOC_ALLOC: u32 = ioc(3, b'I', 0, 24);
const ION_IOC_HEAP_QUERY: u32 = ioc(3, b'I', 8, 24);

const BUFCAP_MPLANE: u32 = 0x0000_1000; // V4L2_CAP_VIDEO_CAPTURE_MPLANE
const BUFOUT_MPLANE: u32 = 0x0000_2000; // V4L2_CAP_VIDEO_OUTPUT_MPLANE
const TYPE_CAPTURE_MPLANE: u32 = 9;
const TYPE_OUTPUT_MPLANE: u32 = 10;
const MEMORY_USERPTR: u32 = 2;
// msm_vidc.c qbuf/dqbuf: dma-buf fd rides in planes[].reserved[0] and the
// per-plane data offset in reserved[1] (enum msm_vidc_plane_aux — the enum
// lives in a uapi header absent from the LineageOS tree; values pinned by the
// driver's own dqbuf write-back reserved[FD]=m.fd / reserved[DO]=data_offset).
const MSM_VIDC_BUFFER_FD: usize = 0;
const MSM_VIDC_DATA_OFFSET: usize = 1;
const TS_COPY: u32 = 1; // V4L2_BUF_FLAG_TIMESTAMP_COPY

const fn fourcc(a: u8, b: u8, c: u8, d: u8) -> u32 {
    (a as u32) | ((b as u32) << 8) | ((c as u32) << 16) | ((d as u32) << 24)
}
const FMT_H264: u32 = fourcc(b'H', b'2', b'6', b'4');
/// vdec_output_formats[] lists NV12 (not NV12M!) for the CAPTURE queue; the
/// pix-format constraints table still gives it num_planes = 2.
const FMT_NV12: u32 = fourcc(b'N', b'V', b'1', b'2');

/// Default heap mask; overridden by `vidc ion` findings. Placeholder until
/// the heap sweep runs — see `ion()`.
const DEFAULT_ION_HEAP_MASK: u32 = 0x0200_0000; // system heap, id 25 (see `vidc ion`)

// ---------------------------------------------------------------------------
// ABI structs
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy)]
struct V4l2Capability {
    driver: [u8; 16],
    card: [u8; 32],
    bus_info: [u8; 32],
    version: u32,
    capabilities: u32,
    device_caps: u32,
    reserved: [u32; 3],
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct PlanePixFormat {
    sizeimage: u32,
    bytesperline: u32,
    reserved: [u16; 6],
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct PixFormatMplane {
    width: u32,
    height: u32,
    pixelformat: u32,
    field: u32,
    colorspace: u32,
    plane_fmt: [PlanePixFormat; 8], // VIDEO_MAX_PLANES=8 in this kernel
    num_planes: u8,
    flags: u8,
    ycbcr_enc: u8,
    quantization: u8,
    xfer_func: u8,
    reserved: [u8; 7],
}

/// type + pad + raw union. The union is 8-byte aligned (contains pointers),
/// so it starts at offset 8: 4 + 4 pad + 200 = 208 (verified by compiling the
/// real android-msm-redbull-4.19-android13 header with aarch64 LP64).
#[repr(C)]
struct V4l2Format {
    ty: u32,
    _pad: u32,
    fmt: [u8; 200],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct V4l2Plane {
    bytesused: u32,
    length: u32,
    m: u64, // union: low 32 bits = mem_offset or fd
    data_offset: u32,
    reserved: [u32; 11],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Timeval {
    tv_sec: i64,
    tv_usec: i64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct V4l2Timecode {
    ty: u32,
    flags: u32,
    frames: u8,
    seconds: u8,
    minutes: u8,
    hours: u8,
    userbits: [u8; 4],
}

#[repr(C)]
struct V4l2Buffer {
    index: u32,
    ty: u32,
    bytesused: u32,
    flags: u32,
    field: u32,
    timestamp: Timeval,
    timecode: V4l2Timecode,
    sequence: u32,
    memory: u32,
    m: u64, // planes pointer for MPLANE
    length: u32,
    reserved2: u32,
    request_fd: i32,
    _pad: u32,
}

#[repr(C)]
struct V4l2Requestbuffers {
    count: u32,
    ty: u32,
    memory: u32,
    capabilities: u32,
    flags: u8,
    reserved: [u8; 3],
}

/// vendor v4l2_event_subscription (32 B, cmd 0x4020565a)
#[repr(C)]
struct V4l2EventSubscription {
    ty: u32,
    id: u32,
    flags: u32,
    reserved: [u32; 5],
}

/// raw 136-byte v4l2_event (cmd 0x80885659): type @0,
/// union u.src_change.changes @4.
#[repr(C, align(8))]
struct V4l2EventRaw {
    data: [u8; 136],
}

/// downstream 4.19 ION allocation (returns a dma-buf fd)
#[repr(C)]
struct IonAllocData {
    len: u64,
    heap_id_mask: u32,
    flags: u32,
    fd: u32,
    unused: u32,
}

#[repr(C)]
struct IonHeapData {
    name: [u8; 32],
    ty: u32,
    heap_id: u32,
    reserved: [u32; 3],
}

/// Qualcomm-downstream struct ion_heap_query (24 B).
#[repr(C)]
struct IonHeapQuery {
    cnt: u32,
    reserved0: u32,
    heaps: u64, // struct ion_heap_data __user *
    reserved1: u32,
    reserved2: u32,
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn ioctl<T>(fd: i32, req: u32, arg: *mut T) -> i32 {
    // musl declares ioctl(int, int, ...); macOS (host test builds) uses
    // unsigned long — the request bit-pattern (u32) passes through unchanged.
    #[cfg(target_os = "linux")]
    let req = req as libc::c_int;
    #[cfg(not(target_os = "linux"))]
    let req = req as libc::c_ulong;
    unsafe { libc::ioctl(fd, req, arg as *mut libc::c_void) }
}

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

fn cstr(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

fn fourcc_str(v: u32) -> String {
    let b = v.to_le_bytes();
    b.iter().map(|&c| (c as char).to_string()).collect()
}

/// S/G_FMT wrapper around the raw-union v4l2_format.
fn fmt_ioctl(fd: i32, req: u32, ty: u32, pix: &mut PixFormatMplane) -> Result<(), String> {
    let mut f = V4l2Format {
        ty,
        _pad: 0,
        fmt: [0u8; 200],
    };
    if req == VIDIOC_S_FMT {
        unsafe {
            std::ptr::copy_nonoverlapping(
                pix as *const PixFormatMplane as *const u8,
                f.fmt.as_mut_ptr(),
                std::mem::size_of::<PixFormatMplane>(),
            );
        }
    }
    let rc = ioctl(fd, req, &mut f);
    if rc < 0 {
        return Err(format!(
            "fmt ioctl req={req:#010x} rc={rc} errno={} ({})",
            errno(),
            std::io::Error::from_raw_os_error(errno())
        ));
    }
    unsafe {
        // count is in ELEMENTS of the pointer type — keep it byte-typed or
        // this copies size*size bytes and smashes the stack.
        std::ptr::copy_nonoverlapping(
            f.fmt.as_ptr() as *const u8,
            pix as *mut PixFormatMplane as *mut u8,
            std::mem::size_of::<PixFormatMplane>(),
        );
    }
    Ok(())
}

/// Build a v4l2_buffer pointing at `planes`. The planes slice is leaked for
/// the lifetime of the ioctl (probe code — bounded count, acceptable).
fn mk_buffer(ty: u32, memory: u32, planes: Vec<V4l2Plane>) -> V4l2Buffer {
    let n = planes.len() as u32;
    let flags = if ty == TYPE_OUTPUT_MPLANE { TS_COPY } else { 0 };
    let b = V4l2Buffer {
        index: 0,
        ty,
        bytesused: 0,
        flags,
        field: 0,
        timestamp: Timeval { tv_sec: 0, tv_usec: 0 },
        timecode: V4l2Timecode {
            ty: 0,
            flags: 0,
            frames: 0,
            seconds: 0,
            minutes: 0,
            hours: 0,
            userbits: [0; 4],
        },
        sequence: 0,
        memory,
        m: planes.as_ptr() as u64,
        length: n,
        reserved2: 0,
        request_fd: 0,
        _pad: 0,
    };
    std::mem::forget(planes);
    b
}

fn align4k(n: usize) -> usize {
    (n + 0xfff) & !0xfff
}

/// One msm-vidc USERPTR plane: fd in reserved[MSM_VIDC_BUFFER_FD], data offset
/// in reserved[MSM_VIDC_DATA_OFFSET]. `m` carries the mmap address (vb2's
/// userptr slot — the driver's get_userptr is a stub, the value is never
/// dereferenced, but keep it truthful for stack traces).
fn qbuf_plane(fd: i32, ptr: *mut u8, len: usize, data_offset: usize) -> V4l2Plane {
    let mut p = V4l2Plane {
        bytesused: 0,
        length: len as u32,
        m: ptr as u64,
        data_offset: data_offset as u32,
        reserved: [0; 11],
    };
    p.reserved[MSM_VIDC_BUFFER_FD] = fd as u32;
    p.reserved[MSM_VIDC_DATA_OFFSET] = data_offset as u32;
    p
}

/// Feed free OUTPUT buffers with pending Annex-B access units. Shared by the
/// pre-STREAMON bulk feed and the in-loop recycle path. Mirrors what the
/// driver expects at msm_vidc_start_streaming:967 — buffers queued before
/// STREAMON are marked DEFERRED by msm_comm_qbuf and then flushed
/// synchronously by msm_comm_qbufs() inside the same ioctl that lands
/// START_DONE, instead of relying on a later async un-defer.
#[allow(clippy::too_many_arguments)]
fn feed_aus(
    fd: i32,
    free_out: &mut Vec<u32>,
    aus_iter: &mut Vec<&[u8]>,
    out_bufs: &[IonBuf],
    ts: &mut i64,
    fed: &mut usize,
    config_first: bool,
) -> Result<(), String> {
    while let (Some(bi), Some(chunk)) = (free_out.pop(), aus_iter.first().copied()) {
        aus_iter.remove(0);
        let b = &out_bufs[bi as usize];
        let n = chunk.len().min(b.len);
        unsafe {
            std::ptr::copy_nonoverlapping(chunk.as_ptr(), b.ptr, n);
        }
        let mut p = qbuf_plane(b.fd, b.ptr, b.len, 0);
        p.bytesused = n as u32;
        let mut vbuf = mk_buffer(TYPE_OUTPUT_MPLANE, MEMORY_USERPTR, vec![p]);
        vbuf.index = bi;
        // AU0 = leading parameter sets only (no slice): tag it codec-config.
        // msm_vidc_qbuf → vb2 __fill_vb2_buffer keeps user flags (minus the
        // vb2-managed mask), populate_frame_data maps it to
        // HAL_BUFFERFLAG_CODECCONFIG so the fw parses SPS/PPS without
        // expecting picture data in the buffer.
        if *fed == 0 && config_first {
            vbuf.flags = 0x0002_0000; // V4L2_BUF_FLAG_CODECCONFIG
        }
        vbuf.timestamp = Timeval {
            tv_sec: 0,
            tv_usec: *ts,
        };
        *ts += 33_333;
        if ioctl(fd, VIDIOC_QBUF, &mut vbuf) < 0 {
            return Err(format!(
                "QBUF(out {bi}, {n} B) errno={} ({})",
                errno(),
                std::io::Error::from_raw_os_error(errno())
            ));
        }
        *fed += 1;
        if free_out.is_empty() {
            break;
        }
    }
    Ok(())
}

/// ION-allocated dma-buf, mapped into our address space.
struct IonBuf {
    fd: i32,
    ptr: *mut u8,
    len: usize,
}

impl IonBuf {
    fn alloc(len: usize, heap_mask: u32) -> Result<IonBuf, String> {
        let f = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/ion")
            .map_err(|e| format!("open /dev/ion: {e}"))?;
        let mut data = IonAllocData {
            len: len as u64,
            heap_id_mask: heap_mask,
            flags: 0,
            fd: 0,
            unused: 0,
        };
        if ioctl(f.as_raw_fd(), ION_IOC_ALLOC, &mut data) < 0 {
            return Err(format!(
                "ION_IOC_ALLOC len={len} mask={heap_mask:#x} errno={} ({})",
                errno(),
                std::io::Error::from_raw_os_error(errno())
            ));
        }
        let dmabuf = data.fd as i32;
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                dmabuf,
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            let e = errno();
            unsafe { libc::close(dmabuf) };
            return Err(format!("mmap dmabuf errno={e}"));
        }
        Ok(IonBuf {
            fd: dmabuf,
            ptr: ptr as *mut u8,
            len,
        })
    }
}

// ---------------------------------------------------------------------------
// caps sweep
// ---------------------------------------------------------------------------

pub fn caps() {
    let mut entries: Vec<String> = fs::read_dir("/dev")
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.starts_with("video"))
                .filter(|n| n[5..].chars().all(|c| c.is_ascii_digit()))
                .collect()
        })
        .unwrap_or_default();
    entries.sort_by_key(|n| n[5..].parse::<u32>().unwrap_or(0));
    for name in entries {
        let path = format!("/dev/{name}");
        let f = match fs::File::open(&path) {
            Ok(f) => f,
            Err(e) => {
                println!("{path}: open <{e}>");
                continue;
            }
        };
        let mut cap = V4l2Capability {
            driver: [0; 16],
            card: [0; 32],
            bus_info: [0; 32],
            version: 0,
            capabilities: 0,
            device_caps: 0,
            reserved: [0; 3],
        };
        if ioctl(f.as_raw_fd(), VIDIOC_QUERYCAP, &mut cap) < 0 {
            println!("{path}: QUERYCAP errno={}", errno());
            continue;
        }
        let c = if cap.device_caps != 0 {
            cap.device_caps
        } else {
            cap.capabilities
        };
        let m2m = c & (BUFCAP_MPLANE | BUFOUT_MPLANE) == BUFCAP_MPLANE | BUFOUT_MPLANE;
        println!(
            "{path}: driver={} card={} caps={:08x}{} m2m_mplane={}",
            cstr(&cap.driver),
            cstr(&cap.card),
            c,
            if cap.device_caps != 0 { " (device)" } else { "" },
            m2m,
        );
    }
}

// ---------------------------------------------------------------------------
// size sweep
// ---------------------------------------------------------------------------

/// Sweep the size bits of struct-sized ioctls: the v4l2 core switches on the
/// FULL request word, so a userspace sizeof mismatch surfaces as ENOTTY.
/// ENOTTY => wrong size; anything else => handler ran (struct accepted).
fn sizes(node: &str) {
    let f = match fs::OpenOptions::new().read(true).write(true).open(node) {
        Ok(f) => f,
        Err(e) => {
            println!("{node}: open {e}");
            return;
        }
    };
    let fd = f.as_raw_fd();
    for (name, dir, nr, range) in [
        ("G_FMT", 3u32, 4u8, 188u32..=216),
        ("S_FMT", 3, 5, 188..=216),
        ("REQBUFS", 3, 8, 12..=32),
        ("QUERYBUF", 3, 9, 64..=144),
        ("QBUF", 3, 15, 64..=144),
        ("DQBUF", 3, 17, 64..=144),
    ] {
        for size in range.step_by(4) {
            let req = ioc(dir, b'V', nr, size);
            let mut buf = [0u8; 256];
            let rc = ioctl(fd, req, buf.as_mut_ptr());
            let e = if rc < 0 { errno() } else { 0 };
            if e != 25 {
                println!("{name} size={size}: rc={rc} errno={e}  <-- NOT ENOTTY");
            }
        }
        println!("{name}: all other sizes ENOTTY");
    }
}

// ---------------------------------------------------------------------------
// ION heap enumeration
// ---------------------------------------------------------------------------

fn ion() {
    let f = match fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/ion")
    {
        Ok(f) => f,
        Err(e) => {
            println!("open /dev/ion: {e}");
            return;
        }
    };
    // probe count
    let mut q = IonHeapQuery {
        cnt: 0,
        reserved0: 0,
        heaps: 0,
        reserved1: 0,
        reserved2: 0,
    };
    if ioctl(f.as_raw_fd(), ION_IOC_HEAP_QUERY, &mut q) < 0 {
        println!("ION_IOC_HEAP_QUERY(count probe) errno={}", errno());
        return;
    }
    let n = q.cnt.min(16) as usize;
    let mut heaps: Vec<IonHeapData> = (0..n)
        .map(|_| IonHeapData {
            name: [0; 32],
            ty: 0,
            heap_id: 0,
            reserved: [0; 3],
        })
        .collect();
    let mut q = IonHeapQuery {
        cnt: n as u32,
        reserved0: 0,
        heaps: heaps.as_mut_ptr() as u64,
        reserved1: 0,
        reserved2: 0,
    };
    if ioctl(f.as_raw_fd(), ION_IOC_HEAP_QUERY, &mut q) < 0 {
        println!("ION_IOC_HEAP_QUERY errno={}", errno());
        return;
    }
    for h in heaps.iter().take(n) {
        println!("heap id={} type={} name={}", h.heap_id, h.ty, cstr(&h.name));
    }
}

// ---------------------------------------------------------------------------
// Annex-B frame splitting: chunk per access unit (first slice starts a chunk)
// ---------------------------------------------------------------------------

fn split_access_units(stream: &[u8]) -> Vec<&[u8]> {
    let mut starts: Vec<(usize, u8)> = Vec::new(); // (offset, nal type)
    let mut i = 0;
    while i + 3 <= stream.len() {
        if stream[i] == 0 && stream[i + 1] == 0 && stream[i + 2] == 1 {
            let ty = if i + 3 < stream.len() {
                stream[i + 3] & 0x1f
            } else {
                0
            };
            starts.push((i, ty));
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut aus: Vec<&[u8]> = Vec::new();
    // seed at file head: NALs before the first slice (SPS/PPS/SEI...) become
    // AU0 — the codec-config chunk a stateful decoder parses first. Skipping
    // them desyncs the fw's slice-header parser (observed: EBD offset stops
    // mid-IDR, fw error h264VspRefPicListReordering RES_EMPTY_CHECK, every
    // EBD flagged V4L2_BUF_FLAG_DATA_CORRUPT, zero FBDs).
    let mut au_start: Option<usize> = Some(0);
    for &(off, ty) in starts.iter() {
        let is_slice = ty == 1 || ty == 5 || ty == 20; // non-IDR, IDR, IDR-lite
        if is_slice {
            if let Some(s) = au_start {
                aus.push(&stream[s..off]);
            }
            au_start = Some(off);
        }
    }
    if let Some(s) = au_start {
        aus.push(&stream[s..]);
    }
    aus
}

/// True if the chunk contains a slice NAL (non-IDR / IDR / IDR-lite) — used
/// to decide whether AU0 is pure codec config.
fn contains_slice(chunk: &[u8]) -> bool {
    let mut i = 0;
    while i + 3 <= chunk.len() {
        if chunk[i] == 0 && chunk[i + 1] == 0 && chunk[i + 2] == 1 {
            if i + 3 < chunk.len() {
                let ty = chunk[i + 3] & 0x1f;
                if ty == 1 || ty == 5 || ty == 20 {
                    return true;
                }
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// decode (ION + DMABUF)
// ---------------------------------------------------------------------------

fn find_m2m_node() -> Result<String, String> {
    for n in 0..64u32 {
        let path = format!("/dev/video{n}");
        if fs::metadata(&path).is_err() {
            continue;
        }
        let f = fs::File::open(&path).map_err(|e| format!("{path}: {e}"))?;
        let mut cap = V4l2Capability {
            driver: [0; 16],
            card: [0; 32],
            bus_info: [0; 32],
            version: 0,
            capabilities: 0,
            device_caps: 0,
            reserved: [0; 3],
        };
        if ioctl(f.as_raw_fd(), VIDIOC_QUERYCAP, &mut cap) < 0 {
            continue;
        }
        if cstr(&cap.card) != "msm_vidc_vdec" {
            continue;
        }
        return Ok(path);
    }
    Err("no msm_vidc_vdec device found".into())
}

pub fn decode(stream_path: &str, out_prefix: &str, want_frames: usize, heap_mask: u32) -> Result<(), String> {
    let stream = fs::read(stream_path).map_err(|e| format!("read {stream_path}: {e}"))?;
    let aus = split_access_units(&stream);
    // AU0 is codec config iff it holds no slice NAL (just leading SPS/PPS/SEI)
    let au0_config_only = aus.first().is_some_and(|a| !contains_slice(a));
    println!(
        "bitstream: {} bytes, {} access units, first-chunk {:?} (config_only={au0_config_only})",
        stream.len(),
        aus.len(),
        aus.first().map(|c| c.len())
    );

    let node = find_m2m_node()?;
    // O_NONBLOCK: DQBUF must return EAGAIN instead of blocking so we can
    // round-robin between the two queues on one fd.
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(&node)
        .map_err(|e| format!("open {node}: {e}"))?;
    let fd = file.as_raw_fd();
    println!("device: {node}");

    // OUTPUT: H264 bitstream
    let mut out_fmt = PixFormatMplane {
        width: 320,
        height: 240,
        pixelformat: FMT_H264,
        field: 0,
        colorspace: 0,
        plane_fmt: [PlanePixFormat {
            sizeimage: 512 * 1024,
            bytesperline: 0,
            reserved: [0; 6],
        }; 8],
        num_planes: 1,
        flags: 0,
        ycbcr_enc: 0,
        quantization: 0,
        xfer_func: 0,
        reserved: [0; 7],
    };
    eprintln!("S_FMT(output)...");
    fmt_ioctl(fd, VIDIOC_S_FMT, TYPE_OUTPUT_MPLANE, &mut out_fmt)?;
    // packed-struct fields must be copied out before formatting (E0793)
    let (ow, oh, ofmt) = (out_fmt.width, out_fmt.height, out_fmt.pixelformat);
    let out_len = out_fmt.plane_fmt[0].sizeimage as usize;
    eprintln!("OUTPUT fmt: {ow}x{oh} {} sizeimage={out_len}", fourcc_str(ofmt));

    // CAPTURE: NV12 — the driver's own output-format table (vdec_output_formats
    // in msm_vdec.c) lists plain NV12 with 2 planes; NV12M is not accepted.
    // Sizes stay 0 at this point (stateful decoder): the real geometry only
    // exists after the firmware parses SPS/PPS, reported as a SOURCE_CHANGE
    // event, then read back with G_FMT.
    let mut cap_fmt = PixFormatMplane {
        width: 320,
        height: 240,
        pixelformat: FMT_NV12,
        field: 0,
        colorspace: 0,
        plane_fmt: [PlanePixFormat {
            sizeimage: 0,
            bytesperline: 0,
            reserved: [0; 6],
        }; 8],
        num_planes: 2,
        flags: 0,
        ycbcr_enc: 0,
        quantization: 0,
        xfer_func: 0,
        reserved: [0; 7],
    };
    fmt_ioctl(fd, VIDIOC_S_FMT, TYPE_CAPTURE_MPLANE, &mut cap_fmt)?;
    let (cw, ch, cfmt) = (cap_fmt.width, cap_fmt.height, cap_fmt.pixelformat);
    let cap_planes = (cap_fmt.num_planes as usize).clamp(1, 4);
    eprintln!(
        "CAPTURE fmt (pre-parse): {cw}x{ch} {} planes={cap_planes}",
        fourcc_str(cfmt)
    );

    // subscribe BEFORE streaming so the event can't race past us.
    // SOURCE_CHANGE plus the whole msm_vidc private block (uapi
    // videodev2.h: V4L2_EVENT_MSM_VIDC_START = PRIVATE_START + 0x1000):
    // flush-done, port-settings sufficient/insufficient, sys-error,
    // buffer-reference release, overload/max-clients. The driver narrates
    // reconfigs and fw faults through these, not through SOURCE_CHANGE.
    const EVENT_PRIVATE_START: u32 = 0x0700_0000;
    const MSM_VIDC_EVENT_BASE: u32 = EVENT_PRIVATE_START + 0x1000;
    let sub_types: Vec<(u32, &str)> = vec![
        (EVENT_SOURCE_CHANGE, "SOURCE_CHANGE"),
        (1, "EOS"),
        (MSM_VIDC_EVENT_BASE + 1, "FLUSH_DONE"),
        (MSM_VIDC_EVENT_BASE + 2, "PORT_CHG_SUFFICIENT"),
        (MSM_VIDC_EVENT_BASE + 3, "PORT_CHG_INSUFFICIENT"),
        (MSM_VIDC_EVENT_BASE + 4, "BITDEPTH_CHG_INSUFFICIENT"),
        (MSM_VIDC_EVENT_BASE + 5, "SYS_ERROR"),
        (MSM_VIDC_EVENT_BASE + 6, "RELEASE_BUF_REF"),
        (MSM_VIDC_EVENT_BASE + 7, "RELEASE_UNQUEUED_BUF"),
        (MSM_VIDC_EVENT_BASE + 8, "HW_OVERLOAD"),
        (MSM_VIDC_EVENT_BASE + 9, "MAX_CLIENTS"),
    ];
    for (ty, name) in &sub_types {
        let mut sub = V4l2EventSubscription {
            ty: *ty,
            id: 0,
            flags: 0,
            reserved: [0; 5],
        };
        if ioctl(fd, VIDIOC_SUBSCRIBE_EVENT, &mut sub) < 0 {
            // non-fatal: some ids may be rejected; log and continue
            eprintln!("SUBSCRIBE_EVENT({name}) errno={}", errno());
        }
    }

    // This driver only advances the session state machine — firmware PIL
    // load, HFI session init — when BOTH ports are streaming (msm_vidc.c
    // msm_vidc_start_streaming: each port checks the other's
    // vb2_bufq.streaming before calling start_streaming()). A lone OUTPUT
    // STREAMON leaves every QBUF deferred (msm_comm_qbuf: state !=
    // START_DONE → DEFERRED flag): no SPS parse, no SOURCE_CHANGE, nothing.
    // So both queues go up front, CAPTURE sized by the pre-parse Venus NV12
    // geometry (128-aligned stride, 32-aligned scanlines); a real resolution
    // change still arrives as SOURCE_CHANGE later.
    const NBUF: usize = 4;
    let out_len = align4k(out_len);
    let mut out_bufs = Vec::new();
    for i in 0..NBUF {
        out_bufs.push(IonBuf::alloc(out_len, heap_mask).map_err(|e| format!("out buf {i}: {e}"))?);
    }
    eprintln!("ION: {NBUF} output ({out_len} B) allocated, heap_mask={heap_mask:#x}");

    let stride = ((ow as usize) + 127) & !127; // VENUS_Y_STRIDE: 128-align
    let scanlines = ((oh as usize) + 31) & !31; // VENUS_Y_SCANLINES: 32-align
    let uv_off = stride * scanlines; // where UV starts INSIDE plane 0 (host-side layout hint)
    // The S_FMT copy-back already carries the negotiated capture geometry
    // (msm_vdec.c: plane0.sizeimage = msm_vidc_calculate_dec_output_frame_
    // size — the FULL venus buffer incl. alignment+meta, NOT w*h*1.5; plane1
    // = extradata size; bytesperline = VENUS_Y_STRIDE). vb2 __prepare_userptr
    // rejects any QBUF plane length below the queue_setup size, so allocation
    // must follow the negotiated numbers.
    let cap_bpl = cap_fmt.plane_fmt[0].bytesperline as usize;
    let cap_sz0 = cap_fmt.plane_fmt[0].sizeimage as usize;
    let cap_sz1 = if cap_planes > 1 {
        cap_fmt.plane_fmt[1].sizeimage as usize
    } else {
        0
    };
    eprintln!(
        "CAPTURE negotiated: sizeimage=({cap_sz0},{cap_sz1}) bpl={cap_bpl} (local calc y+uv={})",
        uv_off + stride * (scanlines / 2)
    );
    // plane 0 = whole venus-layout frame; plane 1 = extradata slot
    // (HAL_BUFFER_EXTRADATA_OUTPUT), spare and never read by this probe
    let cap_lens = [
        align4k(cap_sz0.max(uv_off + stride * (scanlines / 2))),
        align4k(cap_sz1.max(4096)),
    ];
    let frame_len: usize = cap_lens[0];
    let mut cap_bufs = Vec::new();
    for i in 0..NBUF {
        cap_bufs.push(IonBuf::alloc(frame_len, heap_mask).map_err(|e| format!("cap buf {i}: {e}"))?);
    }
    eprintln!(
        "ION: {NBUF} capture ({frame_len} B, stride={stride} scanlines={scanlines} uv_off={uv_off})"
    );

    let mut rb = V4l2Requestbuffers {
        count: NBUF as u32,
        ty: TYPE_OUTPUT_MPLANE,
        memory: MEMORY_USERPTR,
        capabilities: 0,
        flags: 0,
        reserved: [0; 3],
    };
    if ioctl(fd, VIDIOC_REQBUFS, &mut rb) < 0 {
        return Err(format!(
            "REQBUFS(output) errno={} ({})",
            errno(),
            std::io::Error::from_raw_os_error(errno())
        ));
    }

    let mut rb = V4l2Requestbuffers {
        count: NBUF as u32,
        ty: TYPE_CAPTURE_MPLANE,
        memory: MEMORY_USERPTR,
        capabilities: 0,
        flags: 0,
        reserved: [0; 3],
    };
    if ioctl(fd, VIDIOC_REQBUFS, &mut rb) < 0 {
        return Err(format!(
            "REQBUFS(capture) errno={} ({})",
            errno(),
            std::io::Error::from_raw_os_error(errno())
        ));
    }

    for i in 0..NBUF {
        let ib = &cap_bufs[i];
        let planes = (0..cap_planes)
            .map(|p| qbuf_plane(ib.fd, ib.ptr, cap_lens[p.min(1)], 0))
            .collect();
        let mut b = mk_buffer(TYPE_CAPTURE_MPLANE, MEMORY_USERPTR, planes);
        b.index = i as u32;
        if ioctl(fd, VIDIOC_QBUF, &mut b) < 0 {
            return Err(format!(
                "QBUF(cap {i}) errno={} ({})",
                errno(),
                std::io::Error::from_raw_os_error(errno())
            ));
        }
    }

    // feed OUTPUT AUs BEFORE either STREAMON: msm_comm_qbuf marks them
    // DEFERRED, and the second STREAMON (the one that completes the
    // both-ports condition and reaches START_DONE) flushes everything via
    // msm_comm_qbufs() inside that same ioctl (msm_vidc.c:967)
    let mut fed = 0usize;
    let mut free_out: Vec<u32> = (0..NBUF as u32).collect();
    let mut aus_iter: Vec<&[u8]> = aus;
    let mut pending_feed = true;
    let mut ts: i64 = 0;
    let mut frames_done = 0usize;
    feed_aus(
        fd,
        &mut free_out,
        &mut aus_iter,
        &out_bufs,
        &mut ts,
        &mut fed,
        au0_config_only,
    )?;
    if aus_iter.is_empty() {
        pending_feed = false;
    }
    eprintln!("pre-fed {fed} OUTPUT AUs (deferred until streamon)");

    let mut on = TYPE_OUTPUT_MPLANE as i32;
    if ioctl(fd, VIDIOC_STREAMON, &mut on) < 0 {
        return Err(format!("STREAMON(output) errno={} ({})", errno(), std::io::Error::from_raw_os_error(errno())));
    }
    let mut on = TYPE_CAPTURE_MPLANE as i32;
    if ioctl(fd, VIDIOC_STREAMON, &mut on) < 0 {
        return Err(format!("STREAMON(capture) errno={} ({})", errno(), std::io::Error::from_raw_os_error(errno())));
    }
    eprintln!("both ports streaming (AUs pre-fed before streamon)");

    'outer: loop {
        if pending_feed {
            feed_aus(
                fd,
                &mut free_out,
                &mut aus_iter,
                &out_bufs,
                &mut ts,
                &mut fed,
                au0_config_only,
            )?;
            if aus_iter.is_empty() {
                pending_feed = false;
                eprintln!("all {fed} AUs fed");
            }
        }

        let mut pfd = libc::pollfd {
            fd,
            // POLLOUT too: OUTPUT (bitstream) completions only mark the fd
            // writable, never readable — polling POLLIN alone sleeps through
            // every EBD (v4l2 poll is per-queue-direction).
            events: libc::POLLIN | libc::POLLOUT | libc::POLLPRI,
            revents: 0,
        };
        let rc = unsafe { libc::poll(&mut pfd, 1, 15_000) };
        if rc == 0 {
            eprintln!("poll timeout after {frames_done} frames (revents={:#x})", pfd.revents);
            break;
        }
        if rc < 0 {
            return Err(format!("poll errno={}", errno()));
        }

        // v4l2 events arrive as POLLPRI; drain then fall through to buffers
        if pfd.revents & libc::POLLPRI != 0 {
            loop {
                let mut ev = V4l2EventRaw { data: [0; 136] };
                if ioctl(fd, VIDIOC_DQEVENT, &mut ev) < 0 {
                    break; // EAGAIN: queue drained
                }
                let ty = u32::from_ne_bytes(ev.data[0..4].try_into().unwrap());
                let changes = u32::from_ne_bytes(ev.data[4..8].try_into().unwrap());
                let name = sub_types
                    .iter()
                    .find(|(t, _)| *t == ty)
                    .map(|(_, n)| *n)
                    .unwrap_or("UNKNOWN");
                eprintln!("EVENT {name} (ty={ty:#x}) changes={changes:#x}");
                if ty == EVENT_SOURCE_CHANGE
                    || (MSM_VIDC_EVENT_BASE + 2..=MSM_VIDC_EVENT_BASE + 4).contains(&ty)
                {
                    fmt_ioctl(fd, VIDIOC_G_FMT, TYPE_CAPTURE_MPLANE, &mut cap_fmt)?;
                    let (nw, nh, nbpl) = (
                        cap_fmt.width,
                        cap_fmt.height,
                        cap_fmt.plane_fmt[0].bytesperline,
                    );
                    let sizes: Vec<u32> = (0..cap_fmt.num_planes.min(4) as usize)
                        .map(|i| cap_fmt.plane_fmt[i].sizeimage)
                        .collect();
                    eprintln!(
                        "CAPTURE fmt (reported): {nw}x{nh} bpl={nbpl} sizes={sizes:?} (allocated {:?}, uv_off={uv_off})",
                        cap_lens
                    );
                    let need: usize = sizes.iter().map(|&s| s as usize).sum();
                    if need > frame_len {
                        // bring-up simplification: we pre-sized CAPTURE from the
                        // S_FMT geometry; a larger real resolution means
                        // realloc + REQBUFS dance that this probe doesn't do yet
                        eprintln!(
                            "WARNING: reported {need} B exceeds allocated {frame_len} B — output planes may be truncated"
                        );
                    }
                }
            }
        }

        // buffer completions: OUTPUT first (frees a bitstream buffer); the
        // flags field tells us whether the fw returned it clean or errored
        // (V4L2_BUF_FLAG_ERROR 0x40)
        let mut b = mk_buffer(
            TYPE_OUTPUT_MPLANE,
            MEMORY_USERPTR,
            vec![qbuf_plane(0, std::ptr::null_mut(), out_len, 0)],
        );
        if ioctl(fd, VIDIOC_DQBUF, &mut b) == 0 {
            let pls: Vec<V4l2Plane> = unsafe {
                std::slice::from_raw_parts(b.m as *const V4l2Plane, b.length as usize).to_vec()
            };
            eprintln!(
                "EBD idx={} flags={:#x} bytesused={}{}",
                b.index,
                b.flags,
                pls.first().map(|p| p.bytesused).unwrap_or(0),
                if b.flags & 0x40 != 0 { " [ERROR]" } else { "" }
            );
            free_out.push(b.index);
            continue;
        }
        let mut b = mk_buffer(
            TYPE_CAPTURE_MPLANE,
            MEMORY_USERPTR,
            (0..cap_planes)
                .map(|i| qbuf_plane(0, std::ptr::null_mut(), cap_lens.get(i).copied().unwrap_or(0), 0))
                .collect(),
        );
        if ioctl(fd, VIDIOC_DQBUF, &mut b) < 0 {
            let e = errno();
            if e == libc::EAGAIN {
                continue;
            }
            return Err(format!("DQBUF errno={e}"));
        }

        // frame!
        let pls: Vec<V4l2Plane> = unsafe {
            std::slice::from_raw_parts(b.m as *const V4l2Plane, b.length as usize).to_vec()
        };
        eprintln!(
            "frame#{} buf={} seq={} flags={:08x} planes={}",
            frames_done,
            b.index,
            b.sequence,
            b.flags,
            b.length
        );
        for (i, pl) in pls.iter().enumerate() {
            if b.index as usize >= cap_bufs.len() {
                continue;
            }
            let ib = &cap_bufs[b.index as usize];
            // capture queues allow zero bytesused — fall back to the full
            // plane size; data_offset round-trips our qbuf values exactly
            // (driver write-back), so it already encodes the Y/UV split
            let n = if pl.bytesused > 0 {
                pl.bytesused as usize
            } else {
                cap_lens.get(i).copied().unwrap_or(0)
            };
            let start = unsafe { ib.ptr.add(pl.data_offset as usize) };
            let data = unsafe { std::slice::from_raw_parts(start, n) };
            let path = format!("{out_prefix}.{}.p{i}", frames_done);
            match fs::write(&path, data) {
                Ok(()) => eprintln!("  wrote {path} ({n} B, doff={} len={})", pl.data_offset, pl.length),
                Err(e) => eprintln!("  write {path} failed: {e}"),
            }
            let _ = std::io::stdout().flush();
        }
        frames_done += 1;

        // requeue the capture buffer
        let ib = &cap_bufs[b.index as usize];
        let planes = (0..cap_planes)
            .map(|p| qbuf_plane(ib.fd, ib.ptr, cap_lens[p.min(1)], 0))
            .collect();
        let mut vbuf = mk_buffer(TYPE_CAPTURE_MPLANE, MEMORY_USERPTR, planes);
        vbuf.index = b.index;
        if ioctl(fd, VIDIOC_QBUF, &mut vbuf) < 0 {
            return Err(format!("reQBUF(cap {}) errno={}", b.index, errno()));
        }
        if frames_done >= want_frames {
            break 'outer;
        }
    }

    eprintln!("decoded {frames_done} frames -> {out_prefix}.*");
    let mut off: i32 = TYPE_CAPTURE_MPLANE as i32;
    let _ = ioctl(fd, VIDIOC_STREAMOFF, &mut off);
    let mut off: i32 = TYPE_OUTPUT_MPLANE as i32;
    let _ = ioctl(fd, VIDIOC_STREAMOFF, &mut off);
    Ok(())
}

// ---------------------------------------------------------------------------
// entry
// ---------------------------------------------------------------------------

/// Subcommand entry.
pub fn run(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("caps") => {
            caps();
            Ok(())
        }
        Some("sizes") => {
            let node = args.get(1).cloned().unwrap_or_else(|| "/dev/video32".into());
            sizes(&node);
            Ok(())
        }
        Some("ion") => {
            ion();
            Ok(())
        }
        Some("decode") => {
            let in_path = args
                .get(1)
                .ok_or("usage: vidc decode <in.h264> <out-prefix> [frames] [heapmask]")?;
            let out_prefix = args
                .get(2)
                .ok_or("usage: vidc decode <in.h264> <out-prefix> [frames] [heapmask]")?;
            let want = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(3);
            let heap = args
                .get(4)
                .and_then(|s| u32::from_str_radix(s.trim_start_matches("0x"), 16).ok())
                .unwrap_or(DEFAULT_ION_HEAP_MASK);
            decode(in_path, out_prefix, want, heap)
        }
        _ => Err("usage: vidc caps | vidc sizes [node] | vidc ion | vidc decode <in.h264> <out> [n] [heapmask]".into()),
    }
}
