//! 语音对话协议 v0 — 无 LLM 的确定性状态机（M42a，产品定义 2026-09-04）。
//!
//! 手机即智能体：人的输入只有说（ASR 文本）和看（M42b），输出是嘴（TTS）
//! 和脸（屏幕）。本模块把 ASR 文本对封闭词表做确定性解析，驱动
//! 听→选（序数）→拼（字母/数字）→回读确认→执行 的对话流。
//! 不做模糊匹配、不做意图猜测——没听懂就说没听懂，这是自举地板：
//! WiFi 必须在 LLM 可用之前连得上。
//!
//! 纯逻辑、零 I/O：daemon（main.rs）喂 Ev，收 Out；所有副作用（TTS、
//! 扫描、wifi-join）都是 Out::Say / Out::Act，由 daemon 落地。

// ---------------- events / outputs ----------------

#[derive(Debug, Clone, PartialEq)]
pub enum Ev {
    /// 一段 ASR 识别文本（已归一化由本模块完成，原样传入即可）
    Heard(String),
    /// Act::Scan 完成，携带去重后的 SSID 列表
    ScanDone(Vec<String>),
    /// Act::Join 完成：Ok(ip) / Err(原因)
    JoinDone(Result<String, String>),
    /// 提示后超时无语音（daemon 计时喂入）
    Timeout,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    /// 扫描 Wi-Fi（nlscan，daemon 解析去重后回 ScanDone）
    Scan,
    /// 加入网络（wifi-join wlan0 ssid psk）
    Join { ssid: String, psk: String },
    /// 状态查询（时间/电池/IP）——daemon 读系统后经 inject_say 出声
    Status,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    /// 说出这句话（TTS + 屏上一行）
    Say(String),
    /// 只刷屏不出声（状态变了但不需要说）
    Show,
    /// 执行动作
    Act(Act),
}

// ---------------- state ----------------

#[derive(Debug, Clone, PartialEq)]
enum St {
    Idle,
    /// 列表已展示，等序数
    WifiList,
    /// 等密码拼读，逐段累积
    WifiPwd { ssid: String, psk: String },
    /// 密码已回读，等确认
    WifiConfirm { ssid: String, psk: String },
}

pub struct Vm {
    st: St,
    /// 对话行（(谁, 文本)），屏显用；cap 8 行滚动
    lines: Vec<(bool, String)>,
    /// 列表内容（SSID），屏显用
    list: Vec<String>,
    /// 已选中的列表项（1-based；0 = 未选）
    sel: usize,
}

impl Vm {
    pub fn new() -> Vm {
        Vm { st: St::Idle, lines: Vec::new(), list: Vec::new(), sel: 0 }
    }

    pub fn state_name(&self) -> &'static str {
        match self.st {
            St::Idle => "idle",
            St::WifiList => "list",
            St::WifiPwd { .. } => "pwd",
            St::WifiConfirm { .. } => "confirm",
        }
    }

    /// 屏显对话行：(is_user, text)
    pub fn lines(&self) -> &[(bool, String)] {
        &self.lines
    }
    pub fn list(&self) -> &[String] {
        &self.list
    }
    pub fn sel(&self) -> usize {
        self.sel
    }
    /// 当前半成品密码（屏显/回读用）
    pub fn psk(&self) -> &str {
        match &self.st {
            St::WifiPwd { psk, .. } | St::WifiConfirm { psk, .. } => psk,
            _ => "",
        }
    }

    /// 状态查询走 Act::Status：daemon 读系统（时间/电池/IP）后用 inject_say
    /// 把拼好的话送回来出声（SM 纯逻辑，不碰钟）。
    pub fn inject_say(&mut self, s: &str) -> Out {
        self.lines.push((false, s.to_string()));
        self.trim_lines();
        Out::Say(s.to_string())
    }

    fn say(&mut self, outs: &mut Vec<Out>, s: &str) {
        self.lines.push((false, s.to_string()));
        self.trim_lines();
        outs.push(Out::Say(s.to_string()));
    }
    fn heard_line(&mut self, s: &str) {
        self.lines.push((true, s.to_string()));
        self.trim_lines();
    }
    fn trim_lines(&mut self) {
        while self.lines.len() > 8 {
            self.lines.remove(0);
        }
    }

    /// 驱动一步。返回要 daemon 落地的输出序列。
    pub fn step(&mut self, ev: Ev) -> Vec<Out> {
        let mut outs = Vec::new();
        match ev {
            Ev::Heard(raw) => {
                let text = norm(&raw);
                if text.is_empty() {
                    outs.push(Out::Show);
                    return outs;
                }
                self.heard_line(&raw);
                // 取消优先于一切状态
                if is_cancel(&text) {
                    self.st = St::Idle;
                    self.sel = 0;
                    self.say(&mut outs, "已取消。");
                    outs.push(Out::Show);
                    return outs;
                }
                match std::mem::replace(&mut self.st, St::Idle) {
                    St::Idle => self.step_idle(&text, &mut outs),
                    St::WifiList => self.step_list(&text, &mut outs),
                    St::WifiPwd { ssid, psk } => self.step_pwd(&text, ssid, psk, &mut outs),
                    St::WifiConfirm { ssid, psk } => {
                        self.step_confirm(&text, ssid, psk, &mut outs)
                    }
                }
            }
            Ev::ScanDone(list) => {
                self.list = list;
                let n = self.list.len();
                if n == 0 {
                    self.st = St::Idle;
                    self.say(&mut outs, "没找到网络。");
                } else {
                    self.st = St::WifiList;
                    self.sel = 0;
                    self.say(&mut outs, &format!("找到{n}个网络，屏幕上选，说第几个。"));
                }
                outs.push(Out::Show);
            }
            Ev::JoinDone(r) => {
                self.st = St::Idle;
                match r {
                    Ok(ip) => self.say(&mut outs, &format!("连上了，地址{ip}。")),
                    Err(e) => self.say(&mut outs, &format!("没连上，{e}。再试可以说无线。")),
                }
                outs.push(Out::Show);
            }
            Ev::Timeout => {
                if !matches!(self.st, St::Idle) {
                    self.st = St::Idle;
                    self.sel = 0;
                    self.say(&mut outs, "超时，已退出。");
                    outs.push(Out::Show);
                }
            }
        }
        outs
    }

    fn step_idle(&mut self, text: &str, outs: &mut Vec<Out>) {
        if is_wifi(text) {
            self.say(outs, "扫描网络。");
            outs.push(Out::Act(Act::Scan));
        } else if is_status(text) {
            self.say(outs, "看一下。");
            outs.push(Out::Act(Act::Status));
        } else if is_hello(text) {
            self.say(outs, "我在。说无线连网，或说状态。");
        } else if is_help(text) {
            self.say(outs, "我能连无线。按住音量下键说话。");
        } else {
            self.say(outs, "没听懂。说无线，或说状态。");
        }
        outs.push(Out::Show);
    }

    fn step_list(&mut self, text: &str, outs: &mut Vec<Out>) {
        if let Some(n) = match_ordinal(text) {
            if n >= 1 && n as usize <= self.list.len() {
                let ssid = self.list[(n - 1) as usize].clone();
                self.sel = n as usize;
                self.st = St::WifiPwd { ssid: ssid.clone(), psk: String::new() };
                self.say(outs, &format!("第{n}个，{ssid}。说密码，字母说大写A小写b，数字说数字三。说完说完了。"));
            } else {
                self.st = St::WifiList;
                self.say(outs, &format!("只有{}个，重新说。", self.list.len()));
            }
        } else if is_wifi(text) || is_rescan(text) {
            self.st = St::WifiList;
            self.say(outs, "重新扫描。");
            outs.push(Out::Act(Act::Scan));
        } else {
            self.st = St::WifiList;
            self.say(outs, "说第几个，或者说取消。");
        }
        outs.push(Out::Show);
    }

    fn step_pwd(&mut self, text: &str, ssid: String, mut psk: String, outs: &mut Vec<Out>) {
        if is_done(text) {
            if psk.is_empty() {
                self.say(outs, "还没听到密码，继续说。");
                self.st = St::WifiPwd { ssid, psk };
            } else {
                let readback = spell_readback(&psk);
                self.say(outs, &format!("密码是{readback}，对吗？"));
                self.st = St::WifiConfirm { ssid, psk };
            }
        } else if is_backspace(text) {
            psk.pop();
            let readback = spell_readback(&psk);
            self.say(outs, &format!("删掉一个，{readback}。"));
            self.st = St::WifiPwd { ssid, psk };
        } else if is_restart(text) {
            self.say(outs, "重新说密码。");
            self.st = St::WifiPwd { ssid, psk: String::new() };
        } else {
            let chars = parse_spelling(text);
            if chars.is_empty() {
                self.say(outs, "没听清。字母说大写A，数字说数字三。");
                self.st = St::WifiPwd { ssid, psk };
            } else {
                psk.extend(chars.iter().cloned());
                let readback = spell_readback(&psk);
                self.say(outs, &format!("收到，{readback}。继续，或说完了。"));
                self.st = St::WifiPwd { ssid, psk };
            }
        }
        outs.push(Out::Show);
    }

    fn step_confirm(&mut self, text: &str, ssid: String, psk: String, outs: &mut Vec<Out>) {
        if is_deny(text) {
            self.st = St::WifiPwd { ssid, psk: String::new() };
            self.say(outs, "重新说密码。");
        } else if is_confirm(text) {
            self.st = St::Idle;
            self.sel = 0;
            self.say(outs, "连接中。");
            outs.push(Out::Act(Act::Join { ssid, psk }));
        } else if is_readback(text) {
            let readback = spell_readback(&psk);
            self.st = St::WifiConfirm { ssid, psk };
            self.say(outs, &format!("密码是{readback}，对吗？"));
        } else {
            self.st = St::WifiConfirm { ssid, psk };
            self.say(outs, "说对，或说不对。");
        }
        outs.push(Out::Show);
    }
}

// ---------------- 归一化 ----------------

/// ASR 后处理归一化：去标点/空白，全角转半角。保留大小写（拼读语义在大小写）。
/// 例："连接无线。第三个密码，大写A小写B数字3。" → "连接无线第三个密码大写A小写B数字3"
pub fn norm(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        // 全角 ASCII → 半角
        let ch = match ch as u32 {
            0xFF01..=0xFF5E => char::from_u32(ch as u32 - 0xFEE0).unwrap_or(ch),
            0x3000 => ' ',
            _ => ch,
        };
        // 标点（中英）与空白全去
        if ch.is_whitespace() {
            continue;
        }
        if matches!(
            ch,
            '。' | '，' | '、' | '！' | '？' | '：' | '；' | '…' | '—' | '·' | '“' | '”' | '‘'
                | '’' | '（' | '）' | '《' | '》' | ',' | '.' | '!' | '?' | ':' | ';' | '-'
                | '_' | '\'' | '"' | '(' | ')' | '<' | '>' | '+' | '=' | '*' | '&' | '#'
                | '@' | '$' | '%' | '|' | '\\' | '/' | '[' | ']' | '{' | '}' | '^' | '~' | '`'
        ) {
            continue;
        }
        out.push(ch);
    }
    out
}

// ---------------- 词表匹配 ----------------

fn contains_any(lower: &str, words: &[&str]) -> bool {
    words.iter().any(|w| lower.contains(w))
}

fn is_wifi(t: &str) -> bool {
    let l = t.to_lowercase();
    // 裸"网"不收（"网站""网游"全误触）；wi+fi 拆开是 ASR 常态
    contains_any(&l, &["无线", "网络", "连网", "上网", "wifi", "wi-fi", "wi fi", "连一下"])
        || (l.contains("wi") && l.contains("fi"))
}

fn is_status(t: &str) -> bool {
    contains_any(t, &["状态", "时间", "几点", "电池", "电量", "ip", "地址"])
}

fn is_hello(t: &str) -> bool {
    contains_any(t, &["你在吗", "在吗", "你好", "喂"])
}

fn is_help(t: &str) -> bool {
    contains_any(t, &["帮助", "能做什么", "你会什么", "怎么用"])
}

fn is_cancel(t: &str) -> bool {
    contains_any(t, &["取消", "算了", "不弄了", "退出", "停止", "不干了"])
}

fn is_confirm(t: &str) -> bool {
    contains_any(t, &["对", "是的", "确认", "没错", "好的", "好", "是", "正确", "可以"])
}

fn is_deny(t: &str) -> bool {
    // 否定必须先于肯定判（"不对" 含 "对"）
    contains_any(t, &["不对", "不是", "错了", "错误", "否", "不行", "不可以", "换一个"])
}

fn is_done(t: &str) -> bool {
    contains_any(t, &["完了", "说完了", "好了", "结束", "没有了", "没了", "搞定", "完毕"])
}

fn is_backspace(t: &str) -> bool {
    contains_any(t, &["删掉", "退格", "删一个", "删除"])
}

fn is_restart(t: &str) -> bool {
    contains_any(t, &["重说", "重来", "重新说", "重新来"])
}

fn is_rescan(t: &str) -> bool {
    contains_any(t, &["刷新", "重新扫", "再扫"])
}

fn is_readback(t: &str) -> bool {
    contains_any(t, &["再说一遍", "重复", "念一遍", "再念"])
}

// ---------------- 序数 ----------------

/// 中文数字词 → 数值（v0 支持一..十 + ASCII 数字 + 组合 十一..九十九 不做，
/// 列表通常 ≤10）。返回 None 表示不是数字词。
fn digit_val(s: &str) -> Option<u8> {
    match s {
        "零" | "〇" | "0" => Some(0),
        "一" | "1" => Some(1),
        "二" | "两" | "2" => Some(2),
        "三" | "3" => Some(3),
        "四" | "4" => Some(4),
        "五" | "5" => Some(5),
        "六" | "6" => Some(6),
        "七" | "7" => Some(7),
        "八" | "8" => Some(8),
        "九" | "9" => Some(9),
        "十" | "10" => Some(10),
        _ => None,
    }
}

/// 从归一化文本里解析序数选择："第三个" / "3个" / 整句就是 "三" / "第2个吧"。
/// 扫描顺序：第X个 → X个 → 裸 X。
pub fn match_ordinal(t: &str) -> Option<u8> {
    // 第X个
    if let Some(i) = t.find('第') {
        let rest = &t[i + '第'.len_utf8()..];
        for len in 1..=2.min(rest.chars().count()) {
            let head: String = rest.chars().take(len).collect();
            if let Some(v) = digit_val(&head) {
                // 后面跟 个 即命中（"第三个" / "第3个吧"）
                let after: String = rest.chars().skip(len).take(1).collect();
                if after == "个" {
                    return Some(v);
                }
            }
        }
    }
    // X个（整串开头）
    let mut chars = t.chars();
    if let Some(first) = chars.next() {
        let one = first.to_string();
        if let Some(v) = digit_val(&one) {
            let second: String = chars.take(1).collect();
            if second == "个" {
                return Some(v);
            }
        }
    }
    // 裸数字词（整句就是一个数字——列表态最常见："三"）
    if t.chars().count() <= 2 {
        let mut parts = t.chars();
        let one: String = parts.next().map(String::from).unwrap_or_default();
        let two: String = parts.next().map(String::from).unwrap_or_default();
        if let Some(v) = digit_val(&one) {
            // "3" / "三"；"10" 是两个字符 '1'+'0'
            if two.is_empty() {
                return Some(v);
            }
        }
        if t == "10" {
            return Some(10);
        }
    }
    None
}

// ---------------- 拼读 ----------------

/// 从归一化文本提取拼读字符序列：大写A→'A'，小写b→'b'，数字3→'3'，
/// 裸字母/数字按原样收。未知片段跳过（ASR 噪声）。
pub fn parse_spelling(t: &str) -> Vec<char> {
    let chars: Vec<char> = t.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    'outer: while i < chars.len() {
        let rest: String = chars[i..].iter().collect();
        // 大写/小写 + 字母
        for (word, upper) in [("大写", true), ("小写", false)] {
            if rest.starts_with(word) {
                if let Some(&c) = chars.get(i + word.chars().count()) {
                    if c.is_ascii_alphabetic() {
                        out.push(if upper { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() });
                        i += word.chars().count() + 1;
                        continue 'outer;
                    }
                }
            }
        }
        // 数字 + 数字词
        if rest.starts_with("数字") {
            let one: String = chars.get(i + 2).map(|c| c.to_string()).unwrap_or_default();
            let two: String = chars.get(i + 3).map(|c| c.to_string()).unwrap_or_default();
            if let Some(v) = digit_val(&one) {
                out.push((b'0' + v) as char);
                i += 3;
                continue;
            }
            let pair = format!("{one}{two}");
            if pair == "10" {
                // "数字一零" 连读（罕见），v0 不支持，跳过
            }
        }
        let c = chars[i];
        if c.is_ascii_alphanumeric() {
            out.push(c);
            i += 1;
            continue;
        }
        // 裸数字词（"数字一零"里的零、口语溜掉的"七"）也收——密码态整句
        // 就是拼读，词表外的汉字走 skip，数字词不该丢
        let one = c.to_string();
        if let Some(v) = digit_val(&one) {
            out.push((b'0' + v) as char);
            i += 1;
            continue;
        }
        i += 1; // 噪声词（"那个""呃"等）跳过
    }
    out
}

/// 回读串：密码字符 → 可 TTS 的拼读文本。"Ab3" → "大写A，小写b，数字3"
pub fn spell_readback(psk: &str) -> String {
    psk.chars()
        .map(|c| {
            if c.is_ascii_uppercase() {
                format!("大写{c}")
            } else if c.is_ascii_lowercase() {
                format!("小写{c}")
            } else {
                format!("数字{c}")
            }
        })
        .collect::<Vec<_>>()
        .join("，")
}

// ---------------- tests ----------------

#[cfg(test)]
mod tests {
    use super::*;

    fn heard(vm: &mut Vm, s: &str) -> Vec<Out> {
        vm.step(Ev::Heard(s.into()))
    }
    fn says(outs: &[Out]) -> Vec<String> {
        outs.iter()
            .filter_map(|o| match o {
                Out::Say(s) => Some(s.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn norm_strips_punct_and_width() {
        assert_eq!(norm("连接无线。第三个密码，大写A小写B数字3。"), "连接无线第三个密码大写A小写B数字3");
        assert_eq!(norm("  WiFi 　测试 ！"), "WiFi测试");
        assert_eq!(norm("Ａｂ３"), "Ab3");
    }

    #[test]
    fn ordinal_forms() {
        assert_eq!(match_ordinal("第三个"), Some(3));
        assert_eq!(match_ordinal("第3个吧"), Some(3));
        assert_eq!(match_ordinal("第十个"), Some(10));
        assert_eq!(match_ordinal("就第二个"), Some(2));
        assert_eq!(match_ordinal("5个"), Some(5));
        assert_eq!(match_ordinal("三"), Some(3));
        assert_eq!(match_ordinal("10"), Some(10));
        assert_eq!(match_ordinal("俩"), None);
        assert_eq!(match_ordinal("无线"), None);
    }

    #[test]
    fn spelling_forms() {
        assert_eq!(parse_spelling("大写A小写b数字3"), vec!['A', 'b', '3']);
        // ASR 常把字母全大写呈现：小写B 仍是小写语义（大小写来自说的前缀）
        assert_eq!(parse_spelling("大写A小写B数字3"), vec!['A', 'b', '3']);
        assert_eq!(parse_spelling("大写ABC"), vec!['A', 'B', 'C']); // 连念大写默认全大
        assert_eq!(parse_spelling("aB9"), vec!['a', 'B', '9']);
        assert_eq!(parse_spelling("呃数字七那个小写m"), vec!['7', 'm']);
        assert_eq!(parse_spelling("数字三数字一零"), vec!['3', '1', '0']);
    }

    #[test]
    fn readback_roundtrip() {
        assert_eq!(spell_readback("Ab3"), "大写A，小写b，数字3");
        // 拼读→回读可循环：人听到的格式 == 说出的格式
        let chars = parse_spelling("大写A小写b数字3");
        let s: String = chars.into_iter().collect();
        assert_eq!(spell_readback(&s), "大写A，小写b，数字3");
    }

    #[test]
    fn confirm_beats_deny_order() {
        assert!(is_deny("不对"));
        assert!(is_confirm("对"));
        assert!(is_confirm("是的没错"));
        // "不对" 判定路径：deny 先查
        assert!(!is_confirm_checked_after_deny("不对"));
    }
    fn is_confirm_checked_after_deny(t: &str) -> bool {
        !is_deny(t) && is_confirm(t)
    }

    #[test]
    fn wifi_flow_full() {
        let mut vm = Vm::new();
        // 1. 触发扫描
        let o = heard(&mut vm, "连接无线。");
        assert!(o.contains(&Out::Act(Act::Scan)));
        // 2. 扫描结果回来
        let o = vm.step(Ev::ScanDone(vec!["Legrand AP".into(), "2501".into()]));
        assert_eq!(says(&o), vec!["找到2个网络，屏幕上选，说第几个。"]);
        assert_eq!(vm.list(), &["Legrand AP".to_string(), "2501".to_string()]);
        // 3. 序数选择
        let o = heard(&mut vm, "第二个");
        assert_eq!(says(&o), vec!["第2个，2501。说密码，字母说大写A小写b，数字说数字三。说完说完了。"]);
        // 4. 密码拼读（ASR 原文带标点）
        let o = heard(&mut vm, "大写A小写B数字3");
        assert_eq!(says(&o)[0], "收到，大写A，小写b，数字3。继续，或说完了。");
        assert_eq!(vm.psk(), "Ab3"); // 小写B：大小写来自说的前缀
        // 5. 完成 → 回读
        let o = heard(&mut vm, "完了");
        assert_eq!(says(&o), vec!["密码是大写A，小写b，数字3，对吗？"]);
        // 6. 确认 → 执行
        let o = heard(&mut vm, "对");
        assert!(o.contains(&Out::Act(Act::Join { ssid: "2501".into(), psk: "Ab3".into() })));
        // 7. 结果
        let o = vm.step(Ev::JoinDone(Ok("192.168.0.166".into())));
        assert_eq!(says(&o), vec!["连上了，地址192.168.0.166。"]);
        assert_eq!(vm.state_name(), "idle");
    }

    #[test]
    fn pwd_backspace_and_restart() {
        let mut vm = Vm::new();
        vm.step(Ev::Heard("无线".into()));
        vm.step(Ev::ScanDone(vec!["X".into()]));
        vm.step(Ev::Heard("第一个".into()));
        heard(&mut vm, "大写A数字2");
        assert_eq!(vm.psk(), "A2");
        let _o = heard(&mut vm, "删掉");
        assert_eq!(vm.psk(), "A");
        let _o = heard(&mut vm, "重说");
        assert_eq!(vm.psk(), "");
    }

    #[test]
    fn confirm_deny_reasks_pwd() {
        let mut vm = Vm::new();
        vm.step(Ev::Heard("连wifi".into()));
        vm.step(Ev::ScanDone(vec!["S".into()]));
        vm.step(Ev::Heard("第一个".into()));
        heard(&mut vm, "小写z");
        heard(&mut vm, "好了");
        assert_eq!(vm.state_name(), "confirm");
        let _o = heard(&mut vm, "不对");
        assert_eq!(vm.state_name(), "pwd");
        assert_eq!(vm.psk(), "");
    }

    #[test]
    fn ordinal_out_of_range_and_rescan() {
        let mut vm = Vm::new();
        vm.step(Ev::Heard("上网".into()));
        vm.step(Ev::ScanDone(vec!["A".into(), "B".into()]));
        let o = heard(&mut vm, "第五个");
        assert_eq!(says(&o), vec!["只有2个，重新说。"]);
        let o = heard(&mut vm, "刷新");
        assert!(o.contains(&Out::Act(Act::Scan)));
    }

    #[test]
    fn cancel_from_any_state() {
        let mut vm = Vm::new();
        vm.step(Ev::Heard("无线".into()));
        vm.step(Ev::ScanDone(vec!["A".into(), "B".into()]));
        vm.step(Ev::Heard("第一个".into()));
        heard(&mut vm, "大写Q");
        let o = heard(&mut vm, "算了");
        assert_eq!(says(&o), vec!["已取消。"]);
        assert_eq!(vm.state_name(), "idle");
        assert_eq!(vm.psk(), "");
    }

    #[test]
    fn timeout_resets_and_idle_stays_quiet() {
        let mut vm = Vm::new();
        vm.step(Ev::Heard("无线".into()));
        vm.step(Ev::ScanDone(vec!["A".into()]));
        let o = vm.step(Ev::Timeout);
        assert_eq!(says(&o), vec!["超时，已退出。"]);
        let o = vm.step(Ev::Timeout); // Idle 下超时不出声
        assert!(o.is_empty());
    }

    #[test]
    fn gibberish_gets_fixed_reply() {
        let mut vm = Vm::new();
        let o = heard(&mut vm, "今天天气哈哈哈");
        assert_eq!(says(&o), vec!["没听懂。说无线，或说状态。"]);
    }

    #[test]
    fn empty_heard_is_silent_show() {
        let mut vm = Vm::new();
        let o = heard(&mut vm, "。！？ ");
        assert_eq!(o, vec![Out::Show]);
    }

    #[test]
    fn readback_repeat_in_confirm() {
        let mut vm = Vm::new();
        vm.step(Ev::Heard("无线".into()));
        vm.step(Ev::ScanDone(vec!["S".into()]));
        vm.step(Ev::Heard("第一个".into()));
        heard(&mut vm, "大写A");
        heard(&mut vm, "完了");
        let o = heard(&mut vm, "再说一遍");
        assert_eq!(says(&o), vec!["密码是大写A，对吗？"]);
    }

    #[test]
    fn asr_real_world_samples() {
        // 2026-09-04 实测 brain ASR 中文样本的归一化路径
        assert_eq!(norm("连接无线。第三个密码，大写A小写B数字3。"), "连接无线第三个密码大写A小写B数字3");
        // 序数识别在真实 ASR 里常出 "第3个"（阿拉伯数字）
        assert_eq!(match_ordinal(&norm("第3个")), Some(3));
        // 密码段识别常夹标点
        assert_eq!(parse_spelling(&norm("密码，大写A小写B数字3。")), vec!['A', 'b', '3']);
    }
}
