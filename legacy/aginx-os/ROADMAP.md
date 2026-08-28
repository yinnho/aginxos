# Aginx OS Development Roadmap

Last updated: 2026-03-30

## Current Status

Kernel runs on QEMU virt (aarch64) with:
- UART shell, GICv3 + Timer (polling mode, QEMU 10.x compat)
- Physical frame allocator, 32MB heap
- PCI bus enumeration (ECAM, requires `highmem=off`), virtio-net driver
- smoltcp TCP/IP stack, DHCP client
- 14 shell commands, task scheduler (idle + shell tasks)
- Preemptive multitasking infrastructure (context switch, scheduler_tick)

**Total: ~2200 lines of Rust**

**QEMU 10.x compatibility notes:**
- MMU: `msr sctlr_el1` with M=1 crashes; run with MMU disabled
- GIC: ICC_PMR_EL1 and ICC_IGRPEN1_EL1 inaccessible; interrupts don't fire
- Timer runs in WFI polling mode; timer interrupt requires GIC fix
- PCI: use `-machine virt,highmem=off` for ECAM at 0x3F000000

Pixel 5 (redfin) boot: fails, needs device tree and further debugging.

---

## Completed Features

| Module | Files | Status | Notes |
|--------|-------|--------|-------|
| Boot entry | entry.S | Done | Stack + FP/SIMD enable |
| UART | uart.rs, qup_uart.rs | Done | PL011 (QEMU) + QUP (Pixel 5) |
| MMU | mmu.rs | Done | 1GB block identity mapping (disabled on QEMU 10.x) |
| Frame allocator | frame_alloc.rs | Done | Physical page management |
| Heap allocator | allocator.rs | Done | linked_list_allocator, 32MB |
| GIC + Timer | gic.rs, interrupt.rs | Done | GICv3 Distributor (CPU intf disabled on QEMU 10.x) |
| Task scheduler | task.rs | Done | 16-slot TCB, context switch, round-robin |
| PCI bus | pci.rs | Done | ECAM enumeration (needs highmem=off) |
| virtio-net | net.rs | Done | Raw Ethernet TX/RX |
| TCP/IP | tcp.rs, smoltcp_dev.rs | Done | smoltcp + DHCP |
| Shell | main.rs | Done | 14 commands (tasks, spawn added) |
| Board support | boards/redfin.rs | Partial | Addresses defined, boot untested |
| VirtIO HAL | virtio_hal.rs | Done | DMA alloc, identity mapping |

---

## Development Roadmap (Priority Order)

### Phase 6: TCP/ICMP Commands (COMPLETED)
**Goal:** Add `ping`, `listen`, `connect`, `sendmsg` shell commands

**Status: DONE** - All commands working. DHCP, ping (3/3), TCP listen/connect, sendmsg verified in QEMU.

| Task | Description | Status |
|------|-------------|--------|
| DHCP IP assignment | Update smoltcp interface IP + default route | Done |
| ICMP ping | `ping <ip>` - 3/3 replies to gateway | Done |
| TCP listen | `listen <port>` - accept incoming TCP connections | Done |
| TCP connect | `connect <ip> <port>` - initiate TCP connection | Done |
| TCP send/recv | `sendmsg <text>` - send data on connection | Done |
| VirtIO VERSION_1 | Feature negotiation fix for RX path | Done |

**Key fixes applied:**
- Added `VIRTIO_F_VERSION_1` feature negotiation (was missing, causing RX to never work)
- Added `iface.update_ip_addrs()` + default route on DHCP configure (smoltcp couldn't send unicast without source IP)
- Increased ping/timeout delays for reliable operation

**Verification:**
```bash
dhclient       # Got IP: 10.0.2.15 (1 poll)
ping 10.0.2.2   # 3/3 received
status         # Shows MAC + IP
listen 8080    # Listening on :8080
```

---

### Phase 7: Process/Task Management (COMPLETED)
**Goal:** Multi-tasking support - the foundation for userspace

| Task | Description | Status |
|------|-------------|--------|
| Task struct | TCB with kernel_sp, state, func, name | Done |
| Context switch | aarch64 exception frame save/restore | Done |
| Round-robin scheduler | Task switching via timer interrupt | Done* |
| Task create/kill | `task_create()` + `task_list()` | Done |
| Idle task | WFI loop when no tasks ready | Done |
| Shell commands | `tasks`, `spawn` | Done |

*Timer interrupt disabled due to QEMU 10.x GIC incompatibility; kernel runs in WFI polling mode.

**Verification:**
```
tasks
Tasks:
  0: idle (Ready) SP=0x40038ef0
  1: shell (Running) SP=0x00000000
```

---

### Phase 8: Userspace Support (High Priority)
**Goal:** EL0 user mode execution + system call interface

Why: Security isolation. The Agent runtime, SSH server, and drivers must run in userspace.

| Task | Description | Est. Lines |
|------|-------------|-----------|
| Exception vectors | aarch64 sync/IRQ/FIQ/SError handlers | ~120 |
| EL0 entry | Drop to userspace (ERET) | ~40 |
| System call handler | SVC exception handler | ~60 |
| Syscall dispatch | aginx-syscall integration | ~80 |
| User memory mapping | Separate page tables per task | ~100 |

**Verification:** Userspace program that makes a `write` syscall to print to UART.

---

### Phase 9: File System (Medium Priority)
**Goal:** Persistent storage via virtio-blk + simple filesystem

Why: Needed for loading programs, configuration, firmware files.

| Task | Description | Est. Lines |
|------|-------------|-----------|
| virtio-blk driver | Block device driver via PCI | ~150 |
| Block layer | Read/write block interface | ~50 |
| Simple FS | FAT32 or minimal custom FS | ~300 |
| Shell file commands | `ls`, `cat`, `write` | ~80 |

**Verification:** Write a file, reboot, read it back.

---

### Phase 10: Scheme/IPC Layer (Medium Priority)
**Goal:** Plan 9 style resource abstraction (like Redox OS)

Why: This is the core architecture for Aginx OS. Enables `network:`, `tcp:`, `ssh:` schemes.

| Task | Description | Est. Lines |
|------|-------------|-----------|
| Scheme trait | `open`, `read`, `write`, `close` | ~60 |
| IPC message passing | Between kernel and userspace | ~150 |
| Scheme registry | `/scheme/` namespace | ~80 |
| File descriptor table | Per-task FD table | ~50 |

**Verification:** Userspace program opens a scheme and reads/writes data.

---

### Phase 11: SSH Server (Medium Priority)
**Goal:** Remote shell access via SSH

Why: This is the primary user-facing feature. Enables remote management of the OS.

| Task | Description | Est. Lines |
|------|-------------|-----------|
| SSH protocol | Key exchange, encryption (rustls or custom) | ~500 |
| PTY | Pseudo-terminal for shell | ~100 |
| sshd daemon | Userspace SSH server | ~200 |

**Verification:** `ssh root@10.0.2.15` from host.

---

### Phase 12: Pixel 5 Real Hardware Boot (Low Priority)
**Goal:** Boot kernel on actual Pixel 5 hardware

Why: Ultimate goal is running on real hardware, but requires significant work.

| Task | Description | Est. Lines |
|------|-------------|-----------|
| Device tree | Obtain/create DTB for SM7350 | ~200 |
| Boot image | Correct Android boot.img v3 format | ~50 |
| UART debug | Verify correct QUP UART address | ~20 |
| WiFi driver | ath11k driver (QCA6390) | ~2000+ |

**Verification:** Kernel boots, UART output visible, shell responds.

---

### Phase 13: Agent Runtime (Long-term)
**Goal:** LLM Agent execution environment

Why: The ultimate vision - an OS designed for AI agents.

| Task | Description | Est. Lines |
|------|-------------|-----------|
| Agent harness | Task lifecycle management | ~300 |
| LLM gateway | HTTP client for API calls | ~400 |
| Tool framework | Function calling interface | ~200 |
| Sandbox | Capability-based isolation | ~300 |

---

## Priority Summary

```
Phase 6 (TCP Commands)      ←── Quick win, enables network testing
    ↓
Phase 7 (Task Management)   ←── Foundation for everything
    ↓
Phase 8 (Userspace)         ←── Security + real OS capability
    ↓
Phase 9 (File System)       ←── Persistence + program loading
    ↓
Phase 10 (Scheme/IPC)       ←── Core architecture
    ↓
Phase 11 (SSH Server)       ←── Remote access
    ↓
Phase 12 (Real Hardware)    ←── Pixel 5 boot
    ↓
Phase 13 (Agent Runtime)    ←── Long-term vision
```

## Recommended Next Step

**Phase 6: TCP Commands** - smallest effort, biggest immediate impact. Can be done entirely in QEMU and tested from the host.

Estimated effort: ~220 lines of new code, all in kernel/src/.
