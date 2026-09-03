//! 按键说话（PTT）：/dev/input/event1 上的 KEY_VOLUMEDOWN，按住=采集、
//! 松手=提交。电源键不动（短按灭屏/长按关机是 M15 语义），音量下键此前
//! 一直被 aterm 忽略——产品唤醒键就选它。evdev 是广播语义：voiced 自己
//! 开 fd，与 aterm 读同一节点互不抢。

use std::fs::File;
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;

pub const PTT_DEV: &str = "/dev/input/event1";
pub const EV_KEY: u16 = 0x01;
pub const KEY_VOLUMEDOWN: u16 = 114;

pub struct Ptt {
    f: File,
    buf: [u8; 512],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PttEv {
    Down,
    Up,
}

impl Ptt {
    pub fn open() -> Option<Ptt> {
        // O_NONBLOCK：主循环 poll 里读，没数据立刻返回
        let f = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(PTT_DEV)
            .ok()?;
        Some(Ptt { f, buf: [0; 512] })
    }

    pub fn fd(&self) -> i32 {
        self.f.as_raw_fd()
    }

    /// 非阻塞排干 pending 事件（主循环 poll POLLIN 后调用）。
    /// 64 位下 input_event = 16(timeval)+2+2+4 = 24 字节无填充。
    pub fn poll(&mut self) -> Vec<PttEv> {
        let mut out = Vec::new();
        loop {
            match self.f.read(&mut self.buf) {
                Ok(0) => break,
                Ok(n) => {
                    let mut off = 0;
                    while off + 24 <= n {
                        let ty = u16::from_le_bytes([self.buf[off + 16], self.buf[off + 17]]);
                        let code = u16::from_le_bytes([self.buf[off + 18], self.buf[off + 19]]);
                        let val = i32::from_le_bytes([
                            self.buf[off + 20],
                            self.buf[off + 21],
                            self.buf[off + 22],
                            self.buf[off + 23],
                        ]);
                        if ty == EV_KEY && code == KEY_VOLUMEDOWN {
                            match val {
                                1 => out.push(PttEv::Down),
                                0 => out.push(PttEv::Up),
                                _ => {} // 2 = repeat，忽略
                            }
                        }
                        off += 24;
                    }
                }
                Err(_) => break, // EAGAIN
            }
        }
        out
    }
}
