//! M41 bring-up: Venus (msm-vidc) stateful M2M H264 → NV12 decode probe.
//!
//! Subcommands:
//!   vidc caps                          — QUERYCAP every /dev/videoN
//!   vidc sizes [node]                  — ioctl size-bit sweep (ABI archaeology)
//!   vidc ion                           — enumerate ION heaps
//!   vidc decode <in.h264> <out> [n] [heapmask]
//!                                      — decode n frames (default 3),
//!                                        dumps <out>.<i>.p0/.p1 planes
//!   vidc enc <in.yuv> <out.h264> <w> <h> [frames] [heapmask]
//!                                      — encode raw yuv420p frames to H264
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
/// _IOWR('V', 96, struct v4l2_decoder_cmd) — 4.19 layout is cmd+flags+ a
/// 64-byte union = 72 B. This driver shares the STOP branch between dec and
/// enc (msm_comm's switch: "This case also for V4L2_ENC_CMD_STOP"), and the
/// encoder's only EOS door is this ioctl (no V4L2_BUF_FLAG_LAST path).
const VIDIOC_DECODER_CMD: u32 = ioc(3, b'V', 96, 72);
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
/// Vendor extensions (this kernel's uapi videodev2.h — NOT upstream values):
/// EOS 0x02000000 would collide with upstream TSTAMP_SRC bits, so pinning
/// each explicitly. First device run with 0x2000 (upstream monotonic ts bit)
/// never saw EOS and drained on poll-timeout instead.
const V4L2_BUF_FLAG_EOS: u32 = 0x0200_0000;
const V4L2_BUF_FLAG_KEYFRAME: u32 = 0x0000_0008;
const V4L2_BUF_FLAG_CODECCONFIG: u32 = 0x0002_0000;
const V4L2_DEC_CMD_STOP: u32 = 1;

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
    find_vidc_node("msm_vidc_vdec", "decoder")
}

fn find_vidc_node(card: &str, what: &str) -> Result<String, String> {
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
        if cstr(&cap.card) != card {
            continue;
        }
        return Ok(path);
    }
    Err(format!("no msm_vidc {what} device found"))
}

pub fn decode(
    stream_path: &str,
    out_prefix: &str,
    want_frames: usize,
    heap_mask: u32,
    disp: Option<&mut DrmDisplay>,
    av: Option<&mut crate::snd::AvAudio>,
) -> Result<(), String> {
    let mut disp = disp;
    let mut av = av;
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
            if disp.is_none() {
                let path = format!("{out_prefix}.{}.p{i}", frames_done);
                match fs::write(&path, data) {
                    Ok(()) => eprintln!("  wrote {path} ({n} B, doff={} len={})", pl.data_offset, pl.length),
                    Err(e) => eprintln!("  write {path} failed: {e}"),
                }
                let _ = std::io::stdout().flush();
            }
        }
        frames_done += 1;

        // zero-copy scanout: import this venus dmabuf straight into the DPU
        // overlay plane (hardware-scaled), before the fw gets the buffer back
        if let Some(d) = disp.as_deref_mut() {
            // A/V sync pacing: gate each scanout on the audio clock (the
            // played-frames counter is the master). Frame n's PTS is n/30
            // shifted by the one tick AU0 burns when it is config-only (the
            // timestamp counter starts on the leading SPS/PPS chunk). The
            // audio timeline starts only AFTER the first frame is on screen
            // — sample 0 then coincides with picture 0 by construction
            // (first device run with start-before-show measured a constant
            // +92 ms audio lead: both clocks tick at 1x, so a start skew
            // never self-corrects).
            let clocked = av.as_deref().is_some_and(|c| c.started());
            if clocked {
                if let Some(clk) = av.as_deref_mut() {
                    let n = frames_done - 1;
                    let pts = (n as i64 + au0_config_only as i64) * 33_333;
                    clk.wait_until(pts);
                    if n % 300 == 0 {
                        let now = clk.played_us();
                        eprintln!(
                            "sync: frame {n} pts={:.3}s audio={:.3}s delta={:+.1}ms",
                            pts as f64 / 1e6,
                            now as f64 / 1e6,
                            (now - pts) as f64 / 1e3
                        );
                    }
                }
            } else if av.is_none() {
                // ~30 fps pacing so the burst isn't a single flash (plain
                // show without an audio clock; with the clock pending this
                // is frame 0 and it goes out now to anchor the timeline)
                std::thread::sleep(std::time::Duration::from_millis(33));
            }
            let (nw, nh, nbpl) = (
                cap_fmt.width,
                cap_fmt.height,
                cap_fmt.plane_fmt[0].bytesperline,
            );
            if let Err(e) = d.show_frame(
                b.index as usize,
                &cap_bufs[b.index as usize],
                nw,
                nh,
                nbpl,
                frame_len,
            ) {
                eprintln!("scanout: {e} (continuing decode)");
            }
            if let Some(clk) = av.as_deref_mut() {
                if !clk.started() {
                    if let Err(e) = clk.start() {
                        eprintln!("snd: {e} — video paces on fixed 33ms");
                    }
                }
            }
        }

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
// encode (ION + DMABUF) — M41b: raw yuv420p in, H264 bitstream out
// ---------------------------------------------------------------------------

/// v4l2_decoder_cmd, 4.19 layout: cmd, flags, then a 64-byte union we treat
/// as an opaque array (the STOP variant only reads nothing extra from it).
#[repr(C)]
struct V4l2DecoderCmd {
    cmd: u32,
    flags: u32,
    raw: [u32; 16],
}

/// Convert one yuv420p frame (Y then U then V planar, w×h each/4) into the
/// venus linear NV12 layout: Y at `stride` pitch for `scanlines` rows, UV
/// interleaved (U,V byte pairs) at `stride * scanlines`. Padding stays zeroed
/// so the untouched alignment gap is deterministic.
fn fill_venus_nv12(
    dst: &mut [u8],
    stride: usize,
    scanlines: usize,
    w: usize,
    h: usize,
    src: &[u8],
) -> Result<(), String> {
    let y_sz = w * h;
    let c_sz = w * h / 4;
    if src.len() < y_sz + 2 * c_sz {
        return Err(format!("yuv frame too short: {} B", src.len()));
    }
    let uv_off = stride * scanlines;
    if uv_off + stride * (h.div_ceil(2)) > dst.len() {
        return Err(format!(
            "venus layout exceeds buffer: need {} have {}",
            uv_off + stride * (h.div_ceil(2)),
            dst.len()
        ));
    }
    for row in 0..h {
        let s = row * w;
        dst[row * stride..row * stride + w].copy_from_slice(&src[s..s + w]);
    }
    let (u_off, v_off) = (y_sz, y_sz + c_sz);
    for row in 0..h / 2 {
        let d = uv_off + row * stride;
        for x in 0..w / 2 {
            dst[d + 2 * x] = src[u_off + row * (w / 2) + x];
            dst[d + 2 * x + 1] = src[v_off + row * (w / 2) + x];
        }
    }
    Ok(())
}

pub fn encode(
    yuv_path: &str,
    out_path: &str,
    w: usize,
    h: usize,
    want_frames: usize,
    heap_mask: u32,
) -> Result<(), String> {
    let data = fs::read(yuv_path).map_err(|e| format!("read {yuv_path}: {e}"))?;
    let yuv_frame = w * h * 3 / 2;
    if data.len() < yuv_frame {
        return Err(format!(
            "{yuv_path}: {} B < one {}x{} yuv420p frame ({yuv_frame} B)",
            data.len(),
            w,
            h
        ));
    }
    let n_frames = (data.len() / yuv_frame).min(want_frames.max(1));
    println!(
        "input: {} bytes, {n_frames} yuv420p frames of {w}x{h} (encoding all)",
        data.len()
    );
    let node = find_vidc_node("msm_vidc_venc", "encoder")?;
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(&node)
        .map_err(|e| format!("open {node}: {e}"))?;
    let fd = file.as_raw_fd();
    println!("device: {node}");

    // CAPTURE first (bitstream port): the encoder's session-open + default
    // profile (H264 -> HIGH) hook runs on this port's S_FMT in msm_venc.c.
    let mut cap_fmt = PixFormatMplane {
        width: w as u32,
        height: h as u32,
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
    fmt_ioctl(fd, VIDIOC_S_FMT, TYPE_CAPTURE_MPLANE, &mut cap_fmt)?;
    let (cw, ch, cfmt) = (cap_fmt.width, cap_fmt.height, cap_fmt.pixelformat);
    let cap_planes = (cap_fmt.num_planes as usize).clamp(1, 4);
    let bitstream_len = cap_fmt.plane_fmt[0].sizeimage as usize;
    eprintln!(
        "CAPTURE fmt: {cw}x{ch} {} planes={cap_planes} sizeimage={bitstream_len}",
        fourcc_str(cfmt)
    );

    // OUTPUT (raw NV12 in): the S_FMT copy-back is authoritative for the
    // venus geometry — bytesperline = VENUS_Y_STRIDE, reserved[0] (u16) =
    // VENUS_Y_SCANLINES (the driver stores the u32 into the __u16 field;
    // real heights fit).
    let mut out_fmt = PixFormatMplane {
        width: w as u32,
        height: h as u32,
        pixelformat: FMT_NV12,
        field: 0,
        colorspace: 0,
        plane_fmt: [PlanePixFormat {
            sizeimage: 0,
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
    fmt_ioctl(fd, VIDIOC_S_FMT, TYPE_OUTPUT_MPLANE, &mut out_fmt)?;
    let (ow, oh, ofmt) = (out_fmt.width, out_fmt.height, out_fmt.pixelformat);
    let out_planes = (out_fmt.num_planes as usize).clamp(1, 4);
    let stride = out_fmt.plane_fmt[0].bytesperline as usize;
    let scanlines = out_fmt.plane_fmt[0].reserved[0] as usize;
    let yuv_len = out_fmt.plane_fmt[0].sizeimage as usize;
    eprintln!(
        "OUTPUT fmt: {ow}x{oh} {} planes={out_planes} sizeimage={yuv_len} stride={stride} scanlines={scanlines} (uv_off={})",
        fourcc_str(ofmt),
        stride * scanlines
    );
    if stride == 0 || scanlines == 0 {
        return Err("driver reported zero stride/scanlines".into());
    }

    // same event set as decode: the private block narrates fw faults; the
    // encoder's drain signal is instead V4L2_BUF_FLAG_EOS on a CAPTURE buffer
    const EVENT_PRIVATE_START: u32 = 0x0700_0000;
    const MSM_VIDC_EVENT_BASE: u32 = EVENT_PRIVATE_START + 0x1000;
    let sub_types: Vec<(u32, &str)> = vec![
        (EVENT_SOURCE_CHANGE, "SOURCE_CHANGE"),
        (1, "EOS"),
        (MSM_VIDC_EVENT_BASE + 1, "FLUSH_DONE"),
        (MSM_VIDC_EVENT_BASE + 5, "SYS_ERROR"),
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
            eprintln!("SUBSCRIBE_EVENT({name}) errno={}", errno());
        }
    }

    // INPUT min_host = 4 (raw frames), CAPTURE min_host = 5 (bitstream);
    // queue_setup only warns below min, but honor it anyway.
    const NBUF_IN: usize = 4;
    const NBUF_CAP: usize = 6;
    let out_lens: [usize; 2] = [
        align4k(yuv_len),
        if out_planes > 1 {
            align4k(out_fmt.plane_fmt[1].sizeimage.max(4096) as usize)
        } else {
            0
        },
    ];
    let cap_len = align4k(bitstream_len.max(4096));
    let mut out_bufs = Vec::new();
    for i in 0..NBUF_IN {
        out_bufs.push(IonBuf::alloc(out_lens[0], heap_mask).map_err(|e| format!("raw buf {i}: {e}"))?);
    }
    let mut cap_bufs = Vec::new();
    for i in 0..NBUF_CAP {
        cap_bufs.push(IonBuf::alloc(cap_len, heap_mask).map_err(|e| format!("bs buf {i}: {e}"))?);
    }
    eprintln!(
        "ION: {nbuf_in} raw ({} B) + {nbuf_cap} bitstream ({cap_len} B), heap_mask={heap_mask:#x}",
        out_lens[0],
        nbuf_in = NBUF_IN,
        nbuf_cap = NBUF_CAP,
    );

    let mut rb = V4l2Requestbuffers {
        count: NBUF_IN as u32,
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
        count: NBUF_CAP as u32,
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

    // pre-queue every bitstream buffer
    for i in 0..NBUF_CAP {
        let ib = &cap_bufs[i];
        let planes = (0..cap_planes)
            .map(|_| qbuf_plane(ib.fd, ib.ptr, cap_len, 0))
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

    let mut outfile = fs::File::create(out_path).map_err(|e| format!("create {out_path}: {e}"))?;

    // feed raw frames into free OUTPUT buffers (pre-STREAMON ones ride the
    // same deferred flush the decoder uses — msm_comm_qbufs at START_DONE)
    let mut free_in: Vec<u32> = (0..NBUF_IN as u32).collect();
    let mut next_frame = 0usize;
    let mut fed = 0usize;
    let mut stop_sent = false;
    let mut chunks = 0usize;
    let mut total_bytes = 0usize;
    let mut first_chunk_flags = String::new();

    while let Some(bi) = free_in.pop() {
        if next_frame >= n_frames {
            break;
        }
        let b = unsafe {
            std::slice::from_raw_parts_mut(out_bufs[bi as usize].ptr, out_lens[0])
        };
        unsafe { libc::memset(b.as_mut_ptr() as *mut libc::c_void, 0, out_lens[0]) };
        fill_venus_nv12(
            b,
            stride,
            scanlines,
            w,
            h,
            &data[next_frame * yuv_frame..(next_frame + 1) * yuv_frame],
        )?;
        let planes = if out_planes > 1 {
            vec![
                qbuf_plane(out_bufs[bi as usize].fd, out_bufs[bi as usize].ptr, out_lens[0], 0),
                qbuf_plane(out_bufs[bi as usize].fd, out_bufs[bi as usize].ptr, out_lens[1], 0),
            ]
        } else {
            vec![qbuf_plane(
                out_bufs[bi as usize].fd,
                out_bufs[bi as usize].ptr,
                out_lens[0],
                0,
            )]
        };
        let mut vbuf = mk_buffer(TYPE_OUTPUT_MPLANE, MEMORY_USERPTR, planes);
        vbuf.index = bi;
        vbuf.timestamp = Timeval {
            tv_sec: 0,
            tv_usec: (next_frame as i64) * 33_333,
        };
        if ioctl(fd, VIDIOC_QBUF, &mut vbuf) < 0 {
            return Err(format!(
                "QBUF(in {bi}, frame {next_frame}) errno={} ({})",
                errno(),
                std::io::Error::from_raw_os_error(errno())
            ));
        }
        next_frame += 1;
        fed += 1;
    }
    eprintln!("pre-fed {fed} raw frames (deferred until streamon)");

    let mut on = TYPE_OUTPUT_MPLANE as i32;
    if ioctl(fd, VIDIOC_STREAMON, &mut on) < 0 {
        return Err(format!("STREAMON(output) errno={} ({})", errno(), std::io::Error::from_raw_os_error(errno())));
    }
    let mut on = TYPE_CAPTURE_MPLANE as i32;
    if ioctl(fd, VIDIOC_STREAMON, &mut on) < 0 {
        return Err(format!("STREAMON(capture) errno={} ({})", errno(), std::io::Error::from_raw_os_error(errno())));
    }
    eprintln!("both ports streaming");

    'outer: loop {
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN | libc::POLLOUT | libc::POLLPRI,
            revents: 0,
        };
        let rc = unsafe { libc::poll(&mut pfd, 1, 15_000) };
        if rc == 0 {
            eprintln!("poll timeout after {chunks} chunks ({total_bytes} B)");
            break;
        }
        if rc < 0 {
            return Err(format!("poll errno={}", errno()));
        }

        if pfd.revents & libc::POLLPRI != 0 {
            loop {
                let mut ev = V4l2EventRaw { data: [0; 136] };
                if ioctl(fd, VIDIOC_DQEVENT, &mut ev) < 0 {
                    break;
                }
                let ty = u32::from_ne_bytes(ev.data[0..4].try_into().unwrap());
                let changes = u32::from_ne_bytes(ev.data[4..8].try_into().unwrap());
                let name = sub_types
                    .iter()
                    .find(|(t, _)| *t == ty)
                    .map(|(_, n)| *n)
                    .unwrap_or("UNKNOWN");
                eprintln!("EVENT {name} (ty={ty:#x}) changes={changes:#x}");
            }
        }

        // INPUT completion (raw frame consumed) → recycle to feed more, or
        // send the STOP drain once the input is exhausted
        let mut b = mk_buffer(
            TYPE_OUTPUT_MPLANE,
            MEMORY_USERPTR,
            vec![qbuf_plane(0, std::ptr::null_mut(), out_lens[0], 0)],
        );
        if ioctl(fd, VIDIOC_DQBUF, &mut b) == 0 {
            eprintln!(
                "EBD idx={} frame consumed flags={:#x}",
                b.index, b.flags
            );
            if next_frame < n_frames {
                let ib = &out_bufs[b.index as usize];
                let dst = unsafe { std::slice::from_raw_parts_mut(ib.ptr, out_lens[0]) };
                unsafe { libc::memset(dst.as_mut_ptr() as *mut libc::c_void, 0, out_lens[0]) };
                fill_venus_nv12(
                    dst,
                    stride,
                    scanlines,
                    w,
                    h,
                    &data[next_frame * yuv_frame..(next_frame + 1) * yuv_frame],
                )?;
                let planes = if out_planes > 1 {
                    vec![
                        qbuf_plane(ib.fd, ib.ptr, out_lens[0], 0),
                        qbuf_plane(ib.fd, ib.ptr, out_lens[1], 0),
                    ]
                } else {
                    vec![qbuf_plane(ib.fd, ib.ptr, out_lens[0], 0)]
                };
                let mut vbuf = mk_buffer(TYPE_OUTPUT_MPLANE, MEMORY_USERPTR, planes);
                vbuf.index = b.index;
                vbuf.timestamp = Timeval {
                    tv_sec: 0,
                    tv_usec: (next_frame as i64) * 33_333,
                };
                if ioctl(fd, VIDIOC_QBUF, &mut vbuf) < 0 {
                    return Err(format!("reQBUF(in {}) errno={}", b.index, errno()));
                }
                next_frame += 1;
                fed += 1;
            } else if !stop_sent {
                // drain: the driver allocates its own internal EOS buffer and
                // queues it to the fw (state must already be START_DONE)
                let mut dcmd = V4l2DecoderCmd {
                    cmd: V4L2_DEC_CMD_STOP,
                    flags: 0,
                    raw: [0; 16],
                };
                if ioctl(fd, VIDIOC_DECODER_CMD, &mut dcmd) < 0 {
                    return Err(format!(
                        "DECODER_CMD(STOP) errno={} ({})",
                        errno(),
                        std::io::Error::from_raw_os_error(errno())
                    ));
                }
                stop_sent = true;
                eprintln!("all {fed} frames fed — drain requested");
            }
            continue;
        }

        // CAPTURE completion (bitstream chunk)
        let mut b = mk_buffer(
            TYPE_CAPTURE_MPLANE,
            MEMORY_USERPTR,
            (0..cap_planes)
                .map(|_| qbuf_plane(0, std::ptr::null_mut(), cap_len, 0))
                .collect(),
        );
        if ioctl(fd, VIDIOC_DQBUF, &mut b) < 0 {
            let e = errno();
            if e == libc::EAGAIN {
                continue;
            }
            return Err(format!("DQBUF errno={e}"));
        }
        let pls: Vec<V4l2Plane> = unsafe {
            std::slice::from_raw_parts(b.m as *const V4l2Plane, b.length as usize).to_vec()
        };
        let pl = pls.first().copied().unwrap_or_else(|| qbuf_plane(0, std::ptr::null_mut(), 0, 0));
        let n = pl.bytesused as usize;
        if n > 0 {
            let start = unsafe { cap_bufs[b.index as usize].ptr.add(pl.data_offset as usize) };
            let chunk = unsafe { std::slice::from_raw_parts(start, n) };
            outfile
                .write_all(chunk)
                .map_err(|e| format!("write {out_path}: {e}"))?;
            total_bytes += n;
        }
        let flag_str = {
            let mut s = String::new();
            if b.flags & V4L2_BUF_FLAG_KEYFRAME != 0 {
                s.push_str(" KEY");
            }
            if b.flags & V4L2_BUF_FLAG_CODECCONFIG != 0 {
                s.push_str(" CONFIG");
            }
            if b.flags & V4L2_BUF_FLAG_EOS != 0 {
                s.push_str(" EOS");
            }
            s
        };
        if chunks < 4 || b.flags & V4L2_BUF_FLAG_EOS != 0 {
            eprintln!(
                "chunk#{} buf={} seq={} bytes={n} flags={:#x}{}",
                chunks, b.index, b.sequence, b.flags, flag_str
            );
        }
        if chunks == 0 {
            first_chunk_flags = flag_str.clone();
        }
        chunks += 1;

        // requeue the bitstream buffer
        let ib = &cap_bufs[b.index as usize];
        let planes = (0..cap_planes)
            .map(|_| qbuf_plane(ib.fd, ib.ptr, cap_len, 0))
            .collect();
        let mut vbuf = mk_buffer(TYPE_CAPTURE_MPLANE, MEMORY_USERPTR, planes);
        vbuf.index = b.index;
        if ioctl(fd, VIDIOC_QBUF, &mut vbuf) < 0 {
            return Err(format!("reQBUF(cap {}) errno={}", b.index, errno()));
        }

        // HAL_BUFFERFLAG_EOS rides the last bitstream buffer — drain complete
        if b.flags & V4L2_BUF_FLAG_EOS != 0 {
            eprintln!("EOS flag seen — encode complete");
            break 'outer;
        }
    }

    let _ = outfile.flush();
    println!(
        "encoded {fed} frames -> {out_path} ({total_bytes} B, {chunks} chunks, first{})",
        if first_chunk_flags.is_empty() { String::new() } else { format!(" chunk{first_chunk_flags}") }
    );
    let mut off: i32 = TYPE_CAPTURE_MPLANE as i32;
    let _ = ioctl(fd, VIDIOC_STREAMOFF, &mut off);
    let mut off: i32 = TYPE_OUTPUT_MPLANE as i32;
    let _ = ioctl(fd, VIDIOC_STREAMOFF, &mut off);
    Ok(())
}

// ---------------------------------------------------------------------------
// DRM plane scanout — DPU zero-copy display of venus NV12 dmabufs
// ---------------------------------------------------------------------------

/// ioctl numbers from the device's own UAPI (android-4.19 drm.h; redbull is
/// the redfin kernel tree). Mode ioctls all share base 'd' and the struct
/// size is part of the request word, so it must match the kernel layout.
const DRM_IOCTL_MODE_GETRESOURCES: u32 = ioc(3, b'd', 0xA0, 64);
const DRM_IOCTL_MODE_GETCRTC: u32 = ioc(3, b'd', 0xA1, std::mem::size_of::<DrmModeCrtc>() as u32);
const DRM_IOCTL_MODE_SETCRTC: u32 = ioc(3, b'd', 0xA2, std::mem::size_of::<DrmModeCrtc>() as u32);
const DRM_IOCTL_MODE_GETENCODER: u32 = ioc(3, b'd', 0xA6, 20);
const DRM_IOCTL_MODE_GETCONNECTOR: u32 = ioc(3, b'd', 0xA7, 64);
const DRM_IOCTL_MODE_CREATE_DUMB: u32 = ioc(3, b'd', 0xB2, 32);
const DRM_IOCTL_MODE_MAP_DUMB: u32 = ioc(3, b'd', 0xB3, 16);
const DRM_IOCTL_MODE_GETPLANE_RES: u32 = ioc(3, b'd', 0xB5, 16);
const DRM_IOCTL_MODE_GETPLANE: u32 = ioc(3, b'd', 0xB6, std::mem::size_of::<DrmGetPlane>() as u32);
const DRM_IOCTL_MODE_SETPLANE: u32 = ioc(3, b'd', 0xB7, 48);
const DRM_IOCTL_MODE_ADDFB2: u32 = ioc(3, b'd', 0xB8, std::mem::size_of::<DrmFbCmd2>() as u32);
const DRM_IOCTL_MODE_OBJ_GETPROPERTIES: u32 = ioc(3, b'd', 0xB9, 32);
const DRM_IOCTL_MODE_OBJ_SETPROPERTY: u32 = ioc(3, b'd', 0xBA, 24);
const DRM_IOCTL_MODE_GETPROP: u32 = ioc(3, b'd', 0xAA, 168);
const DRM_IOCTL_PRIME_FD_TO_HANDLE: u32 = ioc(3, b'd', 0x2e, 12);
/// DRM_IOCTL_SET_CLIENT_CAP (drm.h 0x0d, IOW). struct drm_set_client_cap.
const DRM_IOCTL_SET_CLIENT_CAP: u32 = ioc(1, b'd', 0x0d, 16);
const DRM_CLIENT_CAP_UNIVERSAL_PLANES: u64 = 2;
const DRM_CLIENT_CAP_ATOMIC: u64 = 3;

const DRM_MODE_OBJECT_PLANE: u32 = 0xeeee_eeee;
const DRM_FORMAT_XRGB8888: u32 = 0x3432_5258;

/// layouts proven by aterm's drm.rs on this exact device.
#[repr(C)]
#[derive(Default)]
struct DrmGetConnector {
    encoders_ptr: u64,
    modes_ptr: u64,
    props_ptr: u64,
    prop_values_ptr: u64,
    count_modes: i32,
    count_props: i32,
    count_encoders: i32,
    encoder_id: u32,
    connector_id: u32,
    connector_type: u32,
    connector_type_id: u32,
    pad: u32,
}

#[repr(C)]
#[derive(Default)]
struct DrmGetEncoder {
    encoder_id: u32,
    encoder_type: u32,
    crtc_id: u32,
    possible_crtcs: u32,
    possible_clones: u32,
}

#[repr(C)]
#[derive(Default)]
struct DrmCreateDumb {
    height: u32,
    width: u32,
    bpp: u32,
    flags: u32,
    handle: u32,
    pitch: u32,
    size: u64,
}

#[repr(C)]
#[derive(Default)]
struct DrmMapDumb {
    handle: u32,
    pad: u32,
    offset: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct DrmModeinfo {
    clock: u32,
    hdisplay: u16,
    hsync_start: u16,
    hsync_end: u16,
    htotal: u16,
    hskew: u16,
    vdisplay: u16,
    vsync_start: u16,
    vsync_end: u16,
    vtotal: u16,
    vscan: u16,
    vrefresh: u32,
    flags: u32,
    ty: u32,
    name: [u8; 32],
}

#[repr(C)]
#[derive(Default)]
struct DrmModeCrtc {
    set_connectors_ptr: u64,
    count_connectors: u32,
    crtc_id: u32,
    fb_id: u32,
    x: u32,
    y: u32,
    gamma_size: u32,
    mode_valid: u32,
    mode: DrmModeinfo,
}

#[repr(C)]
#[derive(Default)]
struct DrmGetPlane {
    plane_id: u32,
    crtc_id: u32,
    fb_id: u32,
    possible_crtcs: u32,
    gamma_size: u32,
    count_format_types: u32,
    format_type_ptr: u64,
}

/// NOTE: kernel field order is src_x, src_y, src_h, src_w (h before w —
/// upstream quirk, kept verbatim).
#[repr(C)]
#[derive(Default)]
struct DrmSetPlane {
    plane_id: u32,
    crtc_id: u32,
    fb_id: u32,
    flags: u32,
    crtc_x: i32,
    crtc_y: i32,
    crtc_w: u32,
    crtc_h: u32,
    src_x: u32,
    src_y: u32,
    src_h: u32,
    src_w: u32,
}

#[repr(C)]
#[derive(Default)]
struct DrmFbCmd2 {
    fb_id: u32,
    width: u32,
    height: u32,
    pixel_format: u32,
    flags: u32,
    handles: [u32; 4],
    pitches: [u32; 4],
    offsets: [u32; 4],
    modifier: [u64; 4],
}

#[repr(C)]
#[derive(Default)]
struct DrmPrimeHandle {
    handle: u32,
    flags: u32,
    fd: i32,
}

#[repr(C)]
#[derive(Default)]
struct DrmObjGetProps {
    props_ptr: u64,
    prop_values_ptr: u64,
    count_props: u32,
    obj_id: u32,
    obj_type: u32,
}

#[repr(C)]
#[derive(Default)]
struct DrmObjSetProp {
    value: u64,
    prop_id: u32,
    obj_id: u32,
    obj_type: u32,
}

/// drm_mode_get_property: name is a __u32[32] in the UAPI but the kernel
/// strncpy's raw bytes into it — read as bytes, stop at NUL.
#[repr(C)]
struct DrmGetProp {
    values_ptr: u64,
    enum_blob_ptr: u64,
    prop_id: u32,
    flags: u32,
    name: [u8; 128],
    count_values: u32,
    count_enum_blobs: u32,
}

impl Default for DrmGetProp {
    fn default() -> Self {
        DrmGetProp {
            values_ptr: 0,
            enum_blob_ptr: 0,
            prop_id: 0,
            flags: 0,
            name: [0; 128],
            count_values: 0,
            count_enum_blobs: 0,
        }
    }
}

/// Owns one YUV plane on the active CRTC. Import-and-show per frame; the
/// venus dmabufs never get copied (zero-copy scanout). This kernel gates
/// SETPLANE/OBJ_SETPROPERTY on DRM_MASTER (drm_ioctl.c, no CAP_SYS_ADMIN
/// bypass), so we take master for the lifetime of the display — aterm must
/// not be holding it (kill aterm first; its handoff supervisor revives it
/// after we exit and it re-SET_MASTERs then).
pub struct DrmDisplay {
    file: fs::File,
    crtc_id: u32,
    plane_id: u32,
    zpos_prop: u32, // 0 = no zpos prop found
    mode_w: u32,
    mode_h: u32,
    /// GEM handle + FB per capture buffer index (lazily imported).
    gem: [u32; 4],
    fb: [u32; 4],
}

impl DrmDisplay {
    fn open() -> Result<DrmDisplay, String> {
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_CLOEXEC)
            .open("/dev/dri/card0")
            .map_err(|e| format!("open card0: {e}"))?;
        let fd = file.as_raw_fd();

        // SETPLANE is DRM_MASTER-gated on this kernel — take it. Fails with
        // EINVAL if aterm (or anyone) still holds it.
        const DRM_IOCTL_SET_MASTER: u32 = ioc(0, b'd', 0x1e, 0);
        if ioctl(fd, DRM_IOCTL_SET_MASTER, std::ptr::null_mut::<u8>()) < 0 {
            return Err(format!(
                "SET_MASTER errno={} (is aterm still up? kill it first — {})",
                errno(),
                "its handoff will revive it after we exit"
            ));
        }

        // without UNIVERSAL_PLANES only overlays are listed — and on sde the
        // YUV-capable VIG pipes are the CRTCs' primaries, so every visible
        // plane is RGB-only. With the cap, primaries appear too; the idle
        // CRTC's unbound VIG primary is our NV12+scaler plane.
        #[repr(C)]
        #[derive(Default)]
        struct SetClientCap {
            capability: u64,
            value: u64,
        }
        let mut cap = SetClientCap {
            capability: DRM_CLIENT_CAP_UNIVERSAL_PLANES,
            value: 1,
        };
        if ioctl(fd, DRM_IOCTL_SET_CLIENT_CAP, &mut cap) < 0 {
            eprintln!("drm: SET_CLIENT_CAP(universal planes) errno={}", errno());
        }
        // zpos & friends are atomic properties — without this cap they don't
        // show up in OBJ_GETPROPERTIES at all (we saw 10 props, no zpos).
        let mut cap = SetClientCap {
            capability: DRM_CLIENT_CAP_ATOMIC,
            value: 1,
        };
        if ioctl(fd, DRM_IOCTL_SET_CLIENT_CAP, &mut cap) < 0 {
            eprintln!("drm: SET_CLIENT_CAP(atomic) errno={}", errno());
        }

        // aterm quirk (proven on device): the second GETRESOURCES must carry
        // zero fbs/encoders counts when their pointers are null.
        #[repr(C)]
        #[derive(Default)]
        struct CardRes {
            fb_id_ptr: u64,
            crtc_id_ptr: u64,
            connector_id_ptr: u64,
            encoder_id_ptr: u64,
            count_fbs: u32,
            count_crtcs: u32,
            count_connectors: u32,
            count_encoders: u32,
            min_width: u32,
            max_width: u32,
            min_height: u32,
            max_height: u32,
        }
        let mut res = CardRes::default();
        if ioctl(fd, DRM_IOCTL_MODE_GETRESOURCES, &mut res) < 0 {
            return Err(format!("GETRESOURCES #1 errno={}", errno()));
        }
        let mut crtcs = [0u32; 8];
        let mut conns = [0u32; 8];
        res.count_crtcs = res.count_crtcs.min(8);
        res.crtc_id_ptr = crtcs.as_mut_ptr() as u64;
        res.connector_id_ptr = conns.as_mut_ptr() as u64;
        res.count_connectors = res.count_connectors.min(8);
        // unused id lists must carry BOTH a null pointer and a zero count,
        // or the copy-out EFAULTs (aterm hit the same on fbs/encoders)
        res.count_fbs = 0;
        res.count_encoders = 0;
        if ioctl(fd, DRM_IOCTL_MODE_GETRESOURCES, &mut res) < 0 {
            return Err(format!("GETRESOURCES #2 errno={}", errno()));
        }

        // active CRTC = one with a framebuffer latched (aterm SETCRTC'd it)
        let mut crtc_id = 0u32;
        let mut crtc_idx = 0u32;
        let mut mode_w = 0u32;
        let mut mode_h = 0u32;
        for i in 0..res.count_crtcs {
            let mut gc = DrmModeCrtc {
                crtc_id: crtcs[i as usize],
                ..Default::default()
            };
            if ioctl(fd, DRM_IOCTL_MODE_GETCRTC, &mut gc) < 0 {
                continue;
            }
            if gc.mode_valid != 0 || gc.fb_id != 0 {
                crtc_id = crtcs[i as usize];
                crtc_idx = i;
                mode_w = gc.mode.hdisplay as u32;
                mode_h = gc.mode.vdisplay as u32;
                eprintln!(
                    "drm: active crtc {} (#{i}) mode {}x{} fb={} name={}",
                    crtc_id,
                    mode_w,
                    mode_h,
                    gc.fb_id,
                    cstr(&gc.mode.name)
                );
                break;
            }
        }
        if crtc_id == 0 {
            // We hold master and aterm is dead — but aterm's death dropped
            // the old master and msm's master-drop hook blanked the CRTC, so
            // there is no "active CRTC + free master" state to inherit. Do
            // our own modeset (aterm's proven recipe): DSI connector -> its
            // mode -> black dumb fb -> SETCRTC.
            eprintln!("drm: no active crtc (aterm dead) — cold modeset");
            let n_conn = res.count_connectors as usize;
            let mut conn_id = 0u32;
            let mut enc_id = 0u32;
            let mut mode = DrmModeinfo::default();
            for i in 0..n_conn {
                let mut gc = DrmGetConnector {
                    connector_id: conns[i],
                    ..Default::default()
                };
                let mut modes = [DrmModeinfo::default(); 8];
                let mut encs = [0u64; 8];
                gc.encoders_ptr = encs.as_mut_ptr() as u64;
                gc.count_encoders = 8;
                gc.modes_ptr = modes.as_mut_ptr() as u64;
                gc.count_modes = 8;
                if ioctl(fd, DRM_IOCTL_MODE_GETCONNECTOR, &mut gc) < 0 {
                    continue;
                }
                if gc.count_modes < 1 {
                    continue;
                }
                // encoder_id can read 0 after the previous master exited —
                // fall back to the first compatible encoder (aterm quirk)
                let e = if gc.encoder_id != 0 {
                    gc.encoder_id
                } else if gc.count_encoders > 0 {
                    encs[0] as u32
                } else {
                    0
                };
                if e == 0 {
                    continue;
                }
                if conn_id == 0 || gc.connector_type == 16 {
                    conn_id = conns[i];
                    enc_id = e;
                    mode = modes[0];
                    if gc.connector_type == 16 {
                        break;
                    }
                }
            }
            if conn_id == 0 {
                return Err("cold modeset: no usable connector".into());
            }
            let mut ge = DrmGetEncoder {
                encoder_id: enc_id,
                ..Default::default()
            };
            if ioctl(fd, DRM_IOCTL_MODE_GETENCODER, &mut ge) < 0 || ge.crtc_id == 0 {
                // pick the first crtc the encoder can drive
                for c in 0..res.count_crtcs as usize {
                    if ge.possible_crtcs & (1 << c) != 0 {
                        ge.crtc_id = crtcs[c];
                        break;
                    }
                }
            }
            if ge.crtc_id == 0 {
                return Err("cold modeset: no crtc for encoder".into());
            }
            crtc_id = ge.crtc_id;
            crtc_idx = (0..res.count_crtcs as usize)
                .find(|&i| crtcs[i] == crtc_id)
                .unwrap_or(0) as u32;
            mode_w = mode.hdisplay as u32;
            mode_h = mode.vdisplay as u32;
            eprintln!(
                "drm: conn {} enc {} -> crtc {} mode {}x{} {}",
                conn_id,
                enc_id,
                crtc_id,
                mode_w,
                mode_h,
                cstr(&mode.name)
            );

            // black backdrop: dumb fb, zeroed (API doesn't guarantee it)
            let mut dumb = DrmCreateDumb {
                width: mode_w,
                height: mode_h,
                bpp: 32,
                ..Default::default()
            };
            if ioctl(fd, DRM_IOCTL_MODE_CREATE_DUMB, &mut dumb) < 0 {
                return Err(format!("CREATE_DUMB errno={}", errno()));
            }
            let mut md = DrmMapDumb {
                handle: dumb.handle,
                ..Default::default()
            };
            if ioctl(fd, DRM_IOCTL_MODE_MAP_DUMB, &mut md) < 0 {
                return Err(format!("MAP_DUMB errno={}", errno()));
            }
            let map = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    dumb.size as usize,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_SHARED,
                    fd,
                    md.offset as libc::off_t,
                )
            };
            if map == libc::MAP_FAILED {
                return Err("mmap dumb failed".into());
            }
            unsafe {
                libc::memset(map, 0, dumb.size as usize);
                libc::munmap(map, dumb.size as usize);
            }
            let mut fb2 = DrmFbCmd2 {
                width: mode_w,
                height: mode_h,
                pixel_format: DRM_FORMAT_XRGB8888,
                ..Default::default()
            };
            fb2.handles[0] = dumb.handle;
            fb2.pitches[0] = dumb.pitch;
            if ioctl(fd, DRM_IOCTL_MODE_ADDFB2, &mut fb2) < 0 {
                return Err(format!("ADDFB2(black) errno={}", errno()));
            }
            let conn_list = [conn_id];
            let mut sc = DrmModeCrtc {
                set_connectors_ptr: conn_list.as_ptr() as u64,
                count_connectors: 1,
                crtc_id,
                fb_id: fb2.fb_id,
                mode_valid: 1,
                mode,
                ..Default::default()
            };
            let mut rc = ioctl(fd, DRM_IOCTL_MODE_SETCRTC, &mut sc);
            if rc < 0 {
                // aterm quirk: retry with no connectors latched
                sc.set_connectors_ptr = 0;
                sc.count_connectors = 0;
                rc = ioctl(fd, DRM_IOCTL_MODE_SETCRTC, &mut sc);
            }
            if rc < 0 {
                return Err(format!("SETCRTC errno={}", errno()));
            }
            eprintln!("drm: cold modeset ok (fb {})", fb2.fb_id);
        }

        // plane table; pick an unused (crtc_id==0) overlay whose format list
        // carries NV12 and that can live on the active crtc
        #[repr(C)]
        #[derive(Default)]
        struct PlaneRes {
            plane_id_ptr: u64,
            count_planes: u32,
        }
        let mut pr = PlaneRes::default();
        if ioctl(fd, DRM_IOCTL_MODE_GETPLANE_RES, &mut pr) < 0 {
            return Err(format!("GETPLANERESOURCES errno={}", errno()));
        }
        let mut planes = [0u32; 24];
        pr.count_planes = pr.count_planes.min(24);
        pr.plane_id_ptr = planes.as_mut_ptr() as u64;
        if ioctl(fd, DRM_IOCTL_MODE_GETPLANE_RES, &mut pr) < 0 {
            return Err(format!("GETPLANERESOURCES #2 errno={}", errno()));
        }

        let mut chosen = 0u32;
        let mut fmts_buf = [0u32; 64];
        for i in 0..pr.count_planes {
            let mut gp = DrmGetPlane {
                plane_id: planes[i as usize],
                format_type_ptr: fmts_buf.as_mut_ptr() as u64,
                count_format_types: 64,
                ..Default::default()
            };
            if ioctl(fd, DRM_IOCTL_MODE_GETPLANE, &mut gp) < 0 {
                eprintln!("drm: plane {} GETPLANE errno={}", planes[i as usize], errno());
                continue;
            }
            let n = (gp.count_format_types as usize).min(64);
            let has_nv12 = fmts_buf[..n].contains(&FMT_NV12);
            let fmt_names: Vec<String> = fmts_buf[..n].iter().map(|&f| fourcc_str(f)).collect();
            eprintln!(
                "drm: plane {} crtc={} possible={:#x} nfmts={} nv12={} fmts=[{}]",
                gp.plane_id, gp.crtc_id, gp.possible_crtcs, gp.count_format_types, has_nv12,
                fmt_names.join(",")
            );
            if chosen == 0
                && gp.crtc_id == 0
                && has_nv12
                && gp.possible_crtcs & (1 << crtc_idx) != 0
            {
                chosen = gp.plane_id;
            }
        }
        if chosen == 0 {
            return Err("no free overlay plane with NV12 on the active crtc".into());
        }
        eprintln!("drm: chose overlay plane {chosen}");

        // read the chosen plane's props (names + current values) for the log;
        // remember zpos so we can raise the layer above the terminal
        let mut props = [0u64; 24];
        let mut vals = [0u64; 24];
        let mut og = DrmObjGetProps {
            props_ptr: props.as_mut_ptr() as u64,
            prop_values_ptr: vals.as_mut_ptr() as u64,
            count_props: 24,
            obj_id: chosen,
            obj_type: DRM_MODE_OBJECT_PLANE,
        };
        let mut zpos_prop = 0u32;
        if ioctl(fd, DRM_IOCTL_MODE_OBJ_GETPROPERTIES, &mut og) == 0 {
            let n = (og.count_props as usize).min(24);
            for p in 0..n {
                let mut gprop = DrmGetProp {
                    prop_id: props[p] as u32,
                    values_ptr: [0u64; 2].as_mut_ptr() as u64,
                    count_values: 2,
                    ..Default::default()
                };
                if ioctl(fd, DRM_IOCTL_MODE_GETPROP, &mut gprop) < 0 {
                    continue;
                }
                let end = gprop.name.iter().position(|&b| b == 0).unwrap_or(128);
                let name = String::from_utf8_lossy(&gprop.name[..end]).into_owned();
                eprintln!("drm:   plane prop {name} = {}", vals[p]);
                if name == "zpos" {
                    zpos_prop = props[p] as u32;
                }
            }
        } else {
            eprintln!("drm: plane props read errno={}", errno());
        }

        // sde in custom-client mode defaults EVERY plane's zpos to 0, so our
        // layer and the modeset backdrop would share blend stage 0 and the
        // src-split order check rejects the overlapping full-width rects
        // ("invalid coordinates, stage:0 l:0-1080 r:0-1080"). Move this plane
        // one stage up BEFORE the first SETPLANE (255 would blow the
        // maxblendstages range — max is 7 on this catalog).
        if zpos_prop != 0 {
            let mut sp = DrmObjSetProp {
                value: 1,
                prop_id: zpos_prop,
                obj_id: chosen,
                obj_type: DRM_MODE_OBJECT_PLANE,
            };
            if ioctl(fd, DRM_IOCTL_MODE_OBJ_SETPROPERTY, &mut sp) < 0 {
                eprintln!("drm: zpos=1 errno={} (continuing)", errno());
            } else {
                eprintln!("drm: plane {chosen} zpos -> 1");
            }
        } else {
            eprintln!("drm: no zpos prop — stage collision expected");
        }

        Ok(DrmDisplay {
            file,
            crtc_id,
            plane_id: chosen,
            zpos_prop,
            mode_w,
            mode_h,
            gem: [0; 4],
            fb: [0; 4],
        })
    }

    /// Locate the UV plane start inside the venus buffer: the fw writes only
    /// h luma rows, so the gap between y_end and UV is zeros (observed:
    /// 320x240 -> stride 512, y rows end 0x1E000, UV at 0x40000). Scan
    /// 4K-aligned blocks for the first nonzero one past the luma.
    fn scan_uv_off(ptr: *const u8, len: usize, y_end: usize) -> Option<usize> {
        let data = unsafe { std::slice::from_raw_parts(ptr, len) };
        let mut off = (y_end + 0xfff) & !0xfff;
        while off + 64 <= len {
            if data[off..off + 64].iter().any(|&b| b != 0) {
                return Some(off);
            }
            off += 0x1000;
        }
        None
    }

    /// Import the frame's dmabuf as an NV12 fb (once per buffer) and push it
    /// to the overlay plane, aspect-fit into the panel with DPU scaling.
    fn show_frame(&mut self, idx: usize, buf: &IonBuf, w: u32, h: u32, stride: u32, size: usize) -> Result<(), String> {
        let fd = self.file.as_raw_fd();
        let i = idx.min(3);
        if self.fb[i] == 0 {
            let y_end = stride as usize * h as usize;
            let uv_off = Self::scan_uv_off(buf.ptr, size, y_end)
                .ok_or("scan_uv_off: no chroma block found past luma")?;
            eprintln!("drm: buf{idx} uv_off={uv_off:#x} (y_end={y_end:#x})");

            let mut ph = DrmPrimeHandle {
                fd: buf.fd,
                ..Default::default()
            };
            if ioctl(fd, DRM_IOCTL_PRIME_FD_TO_HANDLE, &mut ph) < 0 {
                return Err(format!("PRIME_FD_TO_HANDLE errno={} ({})", errno(), std::io::Error::from_raw_os_error(errno())));
            }
            self.gem[i] = ph.handle;
            let mut fb = DrmFbCmd2 {
                width: w,
                height: h,
                pixel_format: FMT_NV12,
                ..Default::default()
            };
            fb.handles[0] = ph.handle;
            fb.handles[1] = ph.handle;
            fb.pitches[0] = stride;
            fb.pitches[1] = stride;
            fb.offsets[0] = 0;
            fb.offsets[1] = uv_off as u32;
            if ioctl(fd, DRM_IOCTL_MODE_ADDFB2, &mut fb) < 0 {
                return Err(format!("ADDFB2 errno={} ({})", errno(), std::io::Error::from_raw_os_error(errno())));
            }
            self.fb[i] = fb.fb_id;
            eprintln!("drm: buf{idx} -> fb {} (gem handle {})", fb.fb_id, ph.handle);
        }

        // aspect-fit into the panel
        let scale = (self.mode_w as f64 / w as f64).min(self.mode_h as f64 / h as f64);
        let dw = ((w as f64) * scale) as u32;
        let dh = ((h as f64) * scale) as u32;
        let x = ((self.mode_w - dw) / 2) as i32;
        let y = ((self.mode_h - dh) / 2) as i32;

        let mut sp = DrmSetPlane {
            plane_id: self.plane_id,
            crtc_id: self.crtc_id,
            fb_id: self.fb[i],
            flags: 0,
            crtc_x: x,
            crtc_y: y,
            crtc_w: dw,
            crtc_h: dh,
            src_x: 0,
            src_y: 0,
            src_h: h << 16,
            src_w: w << 16,
        };
        if ioctl(fd, DRM_IOCTL_MODE_SETPLANE, &mut sp) < 0 {
            return Err(format!("SETPLANE errno={} ({})", errno(), std::io::Error::from_raw_os_error(errno())));
        }
        // (zpos is raised to 1 once at open(), before the first SETPLANE)
        Ok(())
    }

    /// Detach the plane (fb_id=0 disables it in the legacy path).
    fn disable(&mut self) {
        let mut sp = DrmSetPlane {
            plane_id: self.plane_id,
            crtc_id: self.crtc_id,
            fb_id: 0,
            ..Default::default()
        };
        let rc = ioctl(self.file.as_raw_fd(), DRM_IOCTL_MODE_SETPLANE, &mut sp);
        eprintln!("drm: plane off rc={rc}");
    }
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
            decode(in_path, out_prefix, want, heap, None, None)
        }
        Some("show") => {
            let in_path = args
                .get(1)
                .ok_or("usage: vidc show <in.h264> [hold_secs] [heapmask]")?;
            let hold: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5);
            let heap = args
                .get(3)
                .and_then(|s| u32::from_str_radix(s.trim_start_matches("0x"), 16).ok())
                .unwrap_or(DEFAULT_ION_HEAP_MASK);
            let mut disp = Some(DrmDisplay::open()?);
            decode(in_path, "", usize::MAX, heap, disp.as_mut(), None)?;
            if let Some(d) = disp.as_mut() {
                eprintln!("holding plane for {hold}s...");
                std::thread::sleep(std::time::Duration::from_secs(hold));
                d.disable();
            }
            Ok(())
        }
        Some("play") => {
            // A/V sync: venus decode → DPU scanout, paced by the audio
            // device's played-frames clock (MM1 → QUIN_TDM_RX_0, the mixer
            // routing audio-bringup bakes at boot). Audio timeline starts
            // when the decoder's first frame is in hand.
            let in_path = args
                .get(1)
                .ok_or("usage: vidc play <in.h264> <in.s16> [vol] [heapmask]")?;
            let pcm_path = args
                .get(2)
                .ok_or("usage: vidc play <in.h264> <in.s16> [vol] [heapmask]")?;
            let vol: i32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(70);
            let heap = args
                .get(4)
                .and_then(|s| u32::from_str_radix(s.trim_start_matches("0x"), 16).ok())
                .unwrap_or(DEFAULT_ION_HEAP_MASK);
            let mut disp = Some(DrmDisplay::open()?);
            let mut av = crate::snd::AvAudio::new("/dev/snd/pcmC0D0p", 48_000, 2, pcm_path, vol)?;
            decode(in_path, "", usize::MAX, heap, disp.as_mut(), Some(&mut av))?;
            // let the audio tail drain (feeder DRAINs at EOF), then tear
            // the plane down — the receipt window is the playing clip itself
            av.finish();
            if let Some(d) = disp.as_mut() {
                d.disable();
            }
            Ok(())
        }
        Some("enc") => {
            let in_path = args.get(1).ok_or(
                "usage: vidc enc <in.yuv> <out.h264> <w> <h> [frames] [heapmask]",
            )?;
            let out_path = args.get(2).ok_or(
                "usage: vidc enc <in.yuv> <out.h264> <w> <h> [frames] [heapmask]",
            )?;
            let w: usize = args
                .get(3)
                .and_then(|s| s.parse().ok())
                .ok_or("usage: vidc enc <in.yuv> <out.h264> <w> <h> [frames] [heapmask]")?;
            let h: usize = args
                .get(4)
                .and_then(|s| s.parse().ok())
                .ok_or("usage: vidc enc <in.yuv> <out.h264> <w> <h> [frames] [heapmask]")?;
            let want = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(usize::MAX);
            let heap = args
                .get(6)
                .and_then(|s| u32::from_str_radix(s.trim_start_matches("0x"), 16).ok())
                .unwrap_or(DEFAULT_ION_HEAP_MASK);
            encode(in_path, out_path, w, h, want, heap)
        }
        _ => Err("usage: vidc caps | vidc sizes [node] | vidc ion | vidc decode <in.h264> <out> [n] [heapmask] | vidc show <in.h264> [hold] [heapmask] | vidc play <in.h264> <in.s16> [vol] [heapmask] | vidc enc <in.yuv> <out.h264> <w> <h> [frames] [heapmask]".into()),
    }
}
