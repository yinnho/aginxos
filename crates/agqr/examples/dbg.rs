//! host 诊断：把 quircs 在一张图上的逐码错误打出来——
//! 抽取失败（finder/定时/网格）还是 ECC 失败（数据坏），一眼分开。
//! 用法：cargo run -p agqr --example dbg -- <jpg>
fn main() {
    let path = std::env::args().nth(1).expect("usage: dbg <jpg>");
    let jpeg = std::fs::read(&path).expect("read");
    let bm = agimg::decode_scaled(&jpeg, agqr::MAX_DECODE_SIDE, agqr::MAX_DECODE_SIDE)
        .expect("decode jpeg");
    let luma = agqr::luma_from_xrgb(bm.w, bm.h, &bm.pix);
    println!("decode at {}x{}", bm.w, bm.h);
    let mut quirc = quircs::Quirc::default();
    let mut n = 0;
    for code in quirc.identify(bm.w as usize, bm.h as usize, &luma) {
        n += 1;
        match code {
            Ok(c) => match c.decode() {
                Ok(d) => println!("code {n}: OK {:?}", String::from_utf8_lossy(&d.payload)),
                Err(e) => println!("code {n}: EXTRACTED but DECODE FAILED: {e:?}"),
            },
            Err(e) => println!("code {n}: EXTRACT FAILED: {e:?}"),
        }
    }
    if n == 0 {
        println!("no candidate patterns at all");
    }
    // decode_luma 的兜底轮：Bradley 局部二值化后再 identify
    let bin = agqr::bradley_binarize(bm.w as usize, bm.h as usize, &luma);
    let t0 = std::time::Instant::now();
    let payloads = agqr::decode_luma(bm.w as usize, bm.h as usize, &luma);
    println!(
        "decode_luma (raw+bradley): {:?} in {:.0}ms [binarize included]",
        payloads,
        t0.elapsed().as_millis()
    );
    let _ = bin;
    // 尺度阶梯：更小的解码边长 = 更强低通，平均掉屏幕摩尔纹
    for side in [960u32, 640, 480, 320, 240] {
        let Some(s) = agimg::decode_scaled(&jpeg, side, side) else {
            continue;
        };
        let l = agqr::luma_from_xrgb(s.w, s.h, &s.pix);
        let t0 = std::time::Instant::now();
        let p = agqr::decode_luma(s.w as usize, s.h as usize, &l);
        println!(
            "scale {side:>4} ({}x{}): {:?} in {:.0}ms",
            s.w,
            s.h,
            p,
            t0.elapsed().as_millis()
        );
    }
    // 全图 Otsu（quircs 内部同款公式）打阈值，看 raw 轮的切分落点
    let (w, h) = (bm.w as usize, bm.h as usize);
    let mut histogram = [0u32; 256];
    for &v in &luma {
        histogram[v as usize] += 1;
    }
    let num = (w * h) as f64;
    let sum: f64 = histogram
        .iter()
        .enumerate()
        .map(|(i, &c)| i as f64 * c as f64)
        .sum();
    let (mut qb, mut sb, mut best, mut thr) = (0f64, 0f64, -1f64, 0usize);
    for (i, &c) in histogram.iter().enumerate() {
        qb += c as f64;
        if qb == 0.0 {
            continue;
        }
        let qw = num - qb;
        if qw == 0.0 {
            break;
        }
        sb += i as f64 * c as f64;
        let m1 = sb / qb;
        let m2 = (sum - sb) / qw;
        let d = m1 - m2;
        let var = d * d * qb * qw;
        if var >= best {
            best = var;
            thr = i;
        }
    }
    println!("global otsu threshold: {thr}");
    let otsu_bin: Vec<u8> = luma.iter().map(|&v| if v < thr as u8 { 0 } else { 255 }).collect();

    // 模糊+Bradley 参数扫描：盒式模糊半径 × 直接 identify（Otsu）
    let (w, h) = (bm.w as usize, bm.h as usize);
    for r in [0usize, 2, 4, 6, 10, 16] {
        let blurred = if r == 0 {
            luma.clone()
        } else {
            box_blur(w, h, &luma, r)
        };
        let p = agqr::decode_luma(w, h, &blurred);
        println!("blur r={r:>2}: {p:?}");
    }
    // finder 环密采样剖面（每 2px）：量化黑模块内部纹波幅度
    for y in [190usize, 292] {
        let mut prof = String::new();
        for (i, x) in (750..1070usize).step_by(2).enumerate() {
            if i % 16 == 0 && !prof.is_empty() {
                println!("dense y={y} {prof}");
                prof.clear();
            }
            prof.push_str(&format!("{:>4} ", luma[y * w + x]));
        }
        if !prof.is_empty() {
            println!("dense y={y} {prof}");
        }
    }
    // 二值图游程结构（finder 中心行附近）：1:1:3:1:1 应清晰可读
    for y in [292usize, 300, 308] {
        let mut runs: Vec<(u8, usize)> = Vec::new();
        for x in 0..w {
            let v = otsu_bin[y * w + x];
            match runs.last_mut() {
                Some((c, n)) if *c == v => *n += 1,
                _ => runs.push((v, 1)),
            }
        }
        let txt: Vec<String> = runs
            .iter()
            .filter(|(_, n)| *n >= 6)
            .map(|(c, n)| format!("{}{}", if *c == 0 { "B" } else { "W" }, n))
            .collect();
        println!("runs y={y}: {}", txt.join(" "));
    }
    // 落盘诊断图（24bit BMP）：原始灰度 / Bradley 二值 / 模糊后二值 / 全图Otsu 二值
    dump_bmp("/tmp/dbg-raw.bmp", w, h, &luma);
    dump_bmp("/tmp/dbg-bin.bmp", w, h, &agqr::bradley_binarize(w, h, &luma));
    dump_bmp("/tmp/dbg-blurbin.bmp", w, h, &agqr::bradley_binarize(w, h, &box_blur(w, h, &luma, 6)));
    dump_bmp("/tmp/dbg-otsu.bmp", w, h, &otsu_bin);
    // QR 中线的灰度剖面：每 8px 采样，看模块黑白真实电平
    let y = h / 2;
    let mut prof = String::new();
    for (i, x) in (0..w).step_by(8).enumerate() {
        if i % 16 == 0 && !prof.is_empty() {
            println!("y={y} {prof}");
            prof.clear();
        }
        prof.push_str(&format!("{:>4} ", luma[y * w + x]));
    }
    if !prof.is_empty() {
        println!("y={y} {prof}");
    }
}

fn dump_bmp(path: &str, w: usize, h: usize, gray: &[u8]) {
    let row = w * 3;
    let pad = (4 - row % 4) % 4;
    let data_len = (row + pad) * h;
    let mut buf = Vec::with_capacity(54 + data_len);
    buf.extend_from_slice(b"BM");
    buf.extend_from_slice(&((54 + data_len) as u32).to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&54u32.to_le_bytes());
    buf.extend_from_slice(&40u32.to_le_bytes());
    buf.extend_from_slice(&(w as i32).to_le_bytes());
    buf.extend_from_slice(&(h as i32).to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&24u16.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&(data_len as u32).to_le_bytes());
    buf.extend_from_slice(&[0u8; 16]);
    for y in (0..h).rev() {
        for x in 0..w {
            let v = gray[y * w + x];
            buf.extend_from_slice(&[v, v, v]);
        }
        buf.extend_from_slice(&vec![0u8; pad]);
    }
    std::fs::write(path, buf).expect("write bmp");
    println!("dumped {path} ({w}x{h})");
}

/// 可分离盒式模糊（水平+垂直各一趟，滑动窗和）。
fn box_blur(w: usize, h: usize, src: &[u8], r: usize) -> Vec<u8> {
    let mut tmp = vec![0u16; w * h];
    for y in 0..h {
        let row = y * w;
        let mut acc: u32 = 0;
        for x in 0..r.min(w) {
            acc += src[row + x] as u32;
        }
        for x in 0..w {
            let add = if x + r < w { src[row + x + r] as u32 } else { 0 };
            let sub = if x >= r + 1 { src[row + x - r - 1] as u32 } else { 0 };
            acc += add;
            acc = acc.saturating_sub(sub);
            let cnt = ((x + r).min(w - 1) - x.saturating_sub(r) + 1) as u32;
            tmp[row + x] = (acc / cnt) as u16;
        }
    }
    let mut out = vec![0u8; w * h];
    for x in 0..w {
        let mut acc: u32 = 0;
        for y in 0..r.min(h) {
            acc += tmp[y * w + x] as u32;
        }
        for y in 0..h {
            let add = if y + r < h { tmp[(y + r) * w + x] as u32 } else { 0 };
            let sub = if y >= r + 1 { tmp[(y - r - 1) * w + x] as u32 } else { 0 };
            acc += add;
            acc = acc.saturating_sub(sub);
            let cnt = ((y + r).min(h - 1) - y.saturating_sub(r) + 1) as u32;
            out[y * w + x] = (acc / cnt) as u8;
        }
    }
    out
}
