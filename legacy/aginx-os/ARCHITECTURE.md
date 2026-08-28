# Aginx OS Architecture

An agent-oriented operating system for Google Pixel 5 (redfin).

## System Overview

```
┌─────────────────────────────────────────────────────────┐
│                    User Space                            │
├─────────────────────────────────────────────────────────┤
│  AgentOS Runtime (Agent Harness + LLM Gateway)          │
├─────────────────────────────────────────────────────────┤
│  SSH Server │ Ion Shell │ Core Utils                     │
├─────────────────────────────────────────────────────────┤
│  smolnetd (TCP/IP via smoltcp)                          │
├─────────────────────────────────────────────────────────┤
│  ath11kd (WiFi) │ pcid (PCI)                             │
├─────────────────────────────────────────────────────────┤
│  Scheme Layer (network:, tcp:, ssh:)                     │
├─────────────────────────────────────────────────────────┤
│  Aginx Kernel (Microkernel)                              │
├─────────────────────────────────────────────────────────┤
│  Hardware: Pixel 5 (QCA6390 WiFi)                        │
└─────────────────────────────────────────────────────────┘
```

## Components

### Kernel (kernel/)
- Microkernel: task scheduling, memory, IPC, schemes
- aarch64 architecture support (MMU, GIC, timer)

### Libraries (libs/)
- aginx-syscall: syscall definitions
- aginx-scheme: scheme implementation helpers
- aginx-event: event loop

### Drivers (drivers/)
- driver-network: NetworkAdapter trait
- pcid: PCI daemon
- ath11kd: WiFi driver (QCA6390)

### Schemes (schemes/)
- init: init process
- ion: shell
- smolnetd: TCP/IP stack
- sshd: SSH daemon

## Build

```bash
make          # Build all
make kernel   # Build kernel only
make qemu     # Run in QEMU
```

## Target

- Hardware: Google Pixel 5 (redfin)
- CPU: Snapdragon 765G (aarch64)
- WiFi: QCA6390 (ath11k)
