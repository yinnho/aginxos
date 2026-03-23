use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 访问模式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccessMode {
    ReadOnly,
    WriteOnly,
    ReadWrite,
}

/// 能力 - 语义化的权限描述
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Capability {
    /// 文件系统访问
    FileSystem {
        paths: Vec<PathBuf>,
        mode: AccessMode,
    },
    /// 网络访问
    Network {
        domains: Vec<String>,
        ports: Vec<u16>,
    },
    /// 命令执行
    Execute {
        commands: Vec<String>,
    },
    /// 创建子 Agent
    SpawnAgent {
        max_count: usize,
    },
    /// 调用 LLM
    CallLLM {
        model: String,
        max_tokens: usize,
    },
}

impl Capability {
    /// 检查此能力是否允许另一个能力
    pub fn allows(&self, other: &Capability) -> bool {
        match (self, other) {
            (Capability::FileSystem { paths: a_paths, mode: a_mode },
             Capability::FileSystem { paths: b_paths, mode: b_mode }) => {
                // 检查路径覆盖
                let path_ok = b_paths.iter().all(|bp|
                    a_paths.iter().any(|ap| bp.starts_with(ap))
                );
                // 检查权限覆盖
                let mode_ok = matches!((a_mode, b_mode),
                    (AccessMode::ReadWrite, _) |
                    (AccessMode::ReadOnly, AccessMode::ReadOnly) |
                    (AccessMode::WriteOnly, AccessMode::WriteOnly)
                );
                path_ok && mode_ok
            }
            (Capability::Network { domains: a_domains, ports: a_ports },
             Capability::Network { domains: b_domains, ports: b_ports }) => {
                // 域名匹配（支持通配符 *）
                let domain_ok = b_domains.iter().all(|bd|
                    a_domains.iter().any(|ad| {
                        if ad == "*" { return true; }
                        if ad.starts_with("*.") {
                            bd.ends_with(&ad[1..]) || bd == &ad[2..]
                        } else {
                            bd == ad
                        }
                    })
                );
                // 端口匹配（空表示所有端口）
                let port_ok = b_ports.is_empty() || a_ports.is_empty() ||
                    b_ports.iter().all(|bp| a_ports.contains(bp));
                domain_ok && port_ok
            }
            (Capability::Execute { commands: a_cmds },
             Capability::Execute { commands: b_cmds }) => {
                b_cmds.iter().all(|bc| a_cmds.contains(bc))
            }
            (Capability::SpawnAgent { max_count: a_max },
             Capability::SpawnAgent { max_count: b_max }) => {
                b_max <= a_max
            }
            (Capability::CallLLM { model: a_model, max_tokens: a_tokens },
             Capability::CallLLM { model: b_model, max_tokens: b_tokens }) => {
                (a_model == "*" || a_model == b_model) && b_tokens <= a_tokens
            }
            _ => false,
        }
    }

    /// 检查能力是否匹配（同类型）
    pub fn matches(&self, other: &Capability) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

/// 能力集合
#[derive(Debug, Clone, Default)]
pub struct CapabilitySet {
    caps: Vec<Capability>,
}

impl CapabilitySet {
    pub fn new() -> Self {
        Self { caps: vec![] }
    }

    pub fn add(&mut self, cap: Capability) {
        if !self.has(&cap) {
            self.caps.push(cap);
        }
    }

    pub fn remove(&mut self, cap: &Capability) {
        self.caps.retain(|c| !c.matches(cap));
    }

    pub fn has(&self, cap: &Capability) -> bool {
        self.caps.iter().any(|c| c.allows(cap))
    }

    pub fn iter(&self) -> impl Iterator<Item = &Capability> {
        self.caps.iter()
    }
}
