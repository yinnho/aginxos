// Metadata protocol (M24, ARCH §4): every ag-* command declares itself in a
// header comment block — `# ag:key=value` lines within the first 80 lines of
// the file. For sh shims that is the command file itself (the canonical
// form); compiled commands carry the same keys in a sibling `.agmd` sidecar
// (generated from `// ag:` lines in the source, so the source stays the
// single source of truth — M25+ brings the generator; today no compiled
// ag-* commands ship).
//
// Keys: summary (one line, required by `--check`), args (usage string;
// a `<token>` marks a REQUIRED positional — bare invocation is refused),
// examples (repeatable, one invocation per line, ARGS-ONLY — the router
// prefixes `ag <route>` so aliases/overrides render right), group, name
// (route override), alias (repeatable, extra route), hidden,
// requires-sudo (strict true/false), exec (pass-through shim's target
// binary — lint-only, lets `--check` catch a missing binary without
// executing anything).

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

pub const HEADER_MAX_LINES: usize = 80;

#[derive(Default, Clone)]
pub struct Meta {
    pub summary: Option<String>,
    pub args: Option<String>,
    pub examples: Vec<String>,
    pub group: Option<String>,
    pub name: Option<String>,
    pub aliases: Vec<String>,
    pub hidden: bool,
    pub requires_sudo: bool,
    pub exec: Option<String>,
}

impl Meta {
    /// Any `<...>` token in args= marks a required positional.
    pub fn requires_args(&self) -> bool {
        self.args
            .as_deref()
            .map(|a| a.split_whitespace().any(|t| t.starts_with('<')))
            .unwrap_or(false)
    }

    /// Menu group: explicit ag:group=, else the route's first hyphen token
    /// (ag-cam-shot → "cam"), else "misc".
    pub fn group_or_derived(&self, file_route: &str) -> String {
        if let Some(g) = &self.group {
            return g.clone();
        }
        file_route.split('-').next().unwrap_or("misc").to_string()
    }
}

/// Parse `key=value` lines fed by the caller (leading `# ag:` / `// ag:`
/// already stripped). Returns the metadata plus lint errors — dispatch-time
/// reads ignore the errors, `commands --check` reports them.
pub fn parse_pairs(pairs: Vec<(String, String)>) -> (Meta, Vec<String>) {
    let mut m = Meta::default();
    let mut errs = Vec::new();
    fn bad_bool(v: &str, key: &str, errs: &mut Vec<String>) -> bool {
        match v {
            "true" => true,
            "false" => false,
            other => {
                errs.push(format!("bad boolean {key}={other} (only true|false)"));
                false
            }
        }
    }
    for (k, v) in pairs {
        match k.as_str() {
            "summary" => m.summary = Some(v),
            "args" => m.args = Some(v),
            "examples" => m.examples.push(v),
            "group" => m.group = Some(v),
            "name" => m.name = Some(v),
            "alias" => m.aliases.push(v),
            "hidden" => m.hidden = bad_bool(&v, "hidden", &mut errs),
            "requires-sudo" => m.requires_sudo = bad_bool(&v, "requires-sudo", &mut errs),
            "exec" => m.exec = Some(v),
            other => errs.push(format!("unknown key ag:{other}=")),
        }
    }
    (m, errs)
}

/// Pull `prefix`-commented pairs from the first HEADER_MAX_LINES lines.
/// `prefix` is `# ag:` for shims/.agmd, `// ag:` for sources.
pub fn parse_header<R: BufRead>(r: R, prefix: &str) -> (Meta, Vec<String>) {
    let mut pairs = Vec::new();
    for line in r.lines().take(HEADER_MAX_LINES) {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix(prefix) {
            let rest = rest.strip_prefix(' ').unwrap_or(rest);
            if let Some(eq) = rest.find('=') {
                pairs.push((
                    rest[..eq].trim().to_string(),
                    rest[eq + 1..].trim().to_string(),
                ));
            }
        }
    }
    parse_pairs(pairs)
}

fn is_script(path: &Path) -> bool {
    let mut b = [0u8; 2];
    let Ok(mut f) = File::open(path) else { return false };
    matches!(f.read_exact(&mut b), Ok(())) && &b == b"#!"
}

pub fn sidecar_for(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".agmd");
    PathBuf::from(s)
}

/// Metadata for one command file, dispatch-time lenient (errors swallowed:
/// help prints what it has, missing header = no metadata). Compiled
/// (non-shebang) files read the `.agmd` sidecar; if absent, no metadata.
pub fn read_for(path: &Path) -> Option<Meta> {
    let src: Box<dyn BufRead> = if is_script(path) {
        Box::new(BufReader::new(File::open(path).ok()?))
    } else {
        Box::new(BufReader::new(File::open(sidecar_for(path)).ok()?))
    };
    let (m, _errs) = parse_header(src, "# ag:");
    Some(m)
}

/// Same, but strict — the `--check` view. Returns lint errors for the file
/// (including "binary without .agmd").
pub fn read_strict(path: &Path) -> (Option<Meta>, Vec<String>) {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return (None, vec!["unreadable".to_string()]),
    };
    if is_script(path) {
        let (m, mut errs) = parse_header(BufReader::new(file), "# ag:");
        if m.summary.is_none() {
            errs.push("missing ag:summary=".to_string());
        }
        (Some(m), errs)
    } else {
        let sc = sidecar_for(path);
        if !sc.exists() {
            return (
                None,
                vec!["compiled command without .agmd sidecar".to_string()],
            );
        }
        let (m, mut errs) = parse_header(
            BufReader::new(File::open(&sc).unwrap_or_else(|_| File::open("/dev/null").unwrap())),
            "# ag:",
        );
        if m.summary.is_none() {
            errs.push("missing ag:summary= (in .agmd)".to_string());
        }
        (Some(m), errs)
    }
}

/// Group descriptions: /etc/ag/groups.desc, one `key=value` per line, `#`
/// comments. A file so packages can introduce groups without reflashing
/// the router (file-is-registry). Env AG_GROUPS_DESC overrides for tests
/// and for the build-time --check gate.
pub fn load_groups() -> BTreeMap<String, String> {
    let p = std::env::var("AG_GROUPS_DESC").unwrap_or_else(|_| "/etc/ag/groups.desc".into());
    let mut out = BTreeMap::new();
    if let Ok(f) = File::open(p) {
        for line in BufReader::new(f).lines().map_while(Result::ok) {
            let t = line.trim();
            if t.is_empty() || t.starts_with('#') {
                continue;
            }
            if let Some(eq) = t.find('=') {
                out.insert(t[..eq].trim().to_string(), t[eq + 1..].trim().to_string());
            }
        }
    }
    out
}
