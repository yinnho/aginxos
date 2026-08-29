// Launcher: four big buttons at boot (clone / codex / grok / sh) plus a
// thin toolbar strip above the keyboard ([SH] always, [BACK] when an app
// runs). Program exit -> back to launcher. Touch regions computed from the
// keyboard geometry so the layout scales with panel size.

pub const BIN_CLONE: &str = "/var/bin/aclone";
pub const BIN_CODEX: &str = "/var/bin/codex";
pub const BIN_GROK: &str = "/var/bin/grok";
pub const BIN_SH: &str = "/bin/sh";

pub struct Entry {
    pub label: &'static str,
    pub bin: &'static str,
    pub avail: bool,
}

pub fn entries() -> Vec<Entry> {
    vec![
        ("CLONE - LOCAL AGENT", BIN_CLONE),
        ("CODEX", BIN_CODEX),
        ("GROK", BIN_GROK),
        ("SH", BIN_SH),
    ]
    .into_iter()
    .map(|(label, bin)| Entry {
        label,
        bin,
        avail: bin == BIN_SH || std::path::Path::new(bin).is_file(),
    })
    .collect()
}

pub struct Geom {
    pub bx: usize,
    pub bw: usize,
    pub bh: usize,
    pub gap: usize,
    pub by0: usize,
    pub toolbar_h: usize,
    pub kb_panel_y: usize,
}

impl Geom {
    pub fn new(w: usize, _h: usize, kb_panel_y: usize) -> Geom {
        let m = 90;
        let toolbar_h = 44;
        let avail_h = kb_panel_y - toolbar_h;
        let gap = 40;
        let bh = ((avail_h - 120 - gap * 3) / 4).min(180);
        Geom {
            bx: m,
            bw: w - 2 * m,
            bh,
            gap,
            by0: toolbar_h + 70,
            toolbar_h,
            kb_panel_y,
        }
    }

    pub fn button_at(&self, x: usize, y: usize, n: usize) -> Option<usize> {
        if x < self.bx || x >= self.bx + self.bw {
            return None;
        }
        for i in 0..n {
            let y0 = self.by0 + i * (self.bh + self.gap);
            if y >= y0 && y < y0 + self.bh {
                return Some(i);
            }
        }
        None
    }

    /// Toolbar regions while an app runs: left = SH (new shell), right =
    /// BACK (kill app, return to launcher).
    pub fn toolbar_hit(&self, x: usize, y: usize, running: bool) -> Option<Toolbar> {
        if y >= self.toolbar_h {
            return None;
        }
        if x < 200 {
            Some(Toolbar::Sh)
        } else if running && x + 200 >= self.bx + self.bw {
            Some(Toolbar::Back)
        } else {
            None
        }
    }
}

pub enum Toolbar {
    Sh,
    Back,
}
