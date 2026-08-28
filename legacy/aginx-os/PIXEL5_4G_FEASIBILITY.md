# Pixel 5 (SC7180) X52 Modem Bare-Metal 4G Feasibility Assessment

# Aginx OS Project - Network Connectivity Research

# Date: 2026-04-04

## Executive Summary

**Verdict: NOT recommended for bare-metal implementation.** The full SMEM->GLINK->QRTR->QMI->rmnet protocol stack required to establish a 4G data connection through the Snapdragon X52 modem is extremely complex (5 custom undocumented protocol layers, ~100K+ lines of Linux kernel code). The recommended path is **USB Ethernet adapter** (1-2 weeks) or **IPA shortcut** (8-12 weeks) as alternatives.

---

## 1. The Protocol Stack

On Pixel 5 (SC7180/SM7250), the Linux 4G data path is:

```
App -> rmnet/MAP -> QRTR -> GLINK -> SMEM -> Modem (X52)
```

The modem is a **separate processor** (remoteproc_mpss) at `0x04080000` with its own firmware. The AP (application processor) communicates with it modem through multiple protocol layers, all running over shared memory.

### Protocol Layers (AP to Modem)

| Layer | Role | Complexity |
|-------|------|------------|
| **SMEM** | Shared Memory at2MB @ 0x80900000 | Low |
| **GLINK** | Generic Link transport over SMEM | Very High |
| **QRTR** | IPC Router over GLINK | Very High |
| **QMI** | Binary TLV RPC for modem control | Very High |
| **rmnet/MAP** | Multiplexing/Aggregation Protocol for IP data | Medium |
| **IPA** | IP Accelerator hardware @ 0x01e40000 | Medium |

---

## 2. Hardware Resources (from SC7180 Device Tree)

### SMEM (Shared Memory)
- Physical address: `0x80900000`, size: `2MB`
- Hardware mutex: TCSR mutex #3
- Used for all AP-to-modem communication

### Modem Processor (remoteproc_mpss)
- Register space: `0x04080000` (0x4040 bytes)
- Power domains: CX, MX, MSS (via RPMH)
- SMEM states: items 435 (to modem), 428 (from modem)
- GLINK edge: interrupt GIC_SPI 277, mailbox apss_shared[12]

### SMP2P (Modem Signaling)
- SMEM items: 435 (out), 428 (in)
- Interrupt: GIC_SPI 296
- Mailbox: apss_shared[14]
- Local PID: 0, Remote PID: 1

### IPA (IP Accelerator)
- Registers: `0x01e40000` (28KB ipa-reg, 8KB ipa-shared, 176KB gsi)
- Interrupts: GIC_SPI 311 (ipa), 432 (gsi), plus SMP2P clock-query and setup-ready
- SMEM states: ipa_smp2p_out[0] (clock-enabled-valid), ipa_smp2p_out[1] (clock-enabled)
- Uses GSI (Generic Software Interface) DMA engine

### RMTFS (Remote File System)
- Address: `0x94600000` (2MB)
- Required for modem firmware storage

### WiFi (WCN3990)
- Address: `0x18800000` (8MB)
- Uses AHB bus (not PCI on Pixel 5)
- Driver: ath11k (100K+ lines, C code)

---

## 3. Layer-by-Layer Analysis

### SMEM (Shared Memory)
**Complexity: LOW | Effort: 1-2 weeks**

Straightforward MMIO access. Key operations:
1. Initialize TCSR mutex #3 for hardware locking
2. Partition 2MB region into items using `smem_alloc()` algorithm
3. Find items by ID (fixed table at start of region)
4. Read/write item data

**SMEM item structure** (from Linux `drivers/soc/qcom/smem.c`):
- Each item has header: `struct smem_header { size, allocated; }` followed by data
- Items allocated by `qcom_smem_alloc(host0, item_id, size, ...)` 
- SMEM items are pre-allocated by bootloader; need `smem_get_item_count()` and `smem_get_item()`

### GLINK (Generic Link)
**Complexity: VERY HIGH | Effort: 4-6 weeks**

This is the hardest layer. GLINK is a peer-to-peer transport protocol:
- **No public specification** - only reference is Qualcomm doc 80-P2598-1 REV C (on Scribd)
- Multiplexes logical "channels" over physical transport (SMEM in this case)
- **Channel intents**: signal TX/RX, data TX/RX, state change
- **Negotiation**: Open/Close channel with remote processor, requires specific packet formats
- Linux implementation: `drivers/rpmsg/qcom_glink_smem.c` (~2000 lines)

Key structures to implement:
1. **G-Link SMEM Transport**: FIFO pairs in SMEM items (one per direction)
2. **Channel open/close**: negotiate channel IDs with remote processor
3. **TX/RX ring buffers**: circular buffers in SMEM for data transfer
4. **Interrupt signaling**: doorbell notifications via mailbox/IPC

Risk: Protocol may vary between SoC generations. GLINK is being replaced by newer transports in recent Qualcomm chips.

### QRTR (Qualcomm IPC Router)
**Complexity: VERY HIGH | Effort: 3-4 weeks**

Socket-like routing protocol:
- **Packet headers**: `struct qrtr_hdr_v1` (6 words: version, type, src_node,1, src_port,1, dst_node,1, dst_port,1)
- **Address assignment**: dynamic node/port allocation
- **Service enumeration**: lookup services by name
- **Control messages**: NEW_CONNECT, DEL_CLIENT, DEL_SERVER, etc.
- Linux implementation: `net/qrtr/qrtr.c` (~1500 lines) + `af_qrtr.c`

The QRTR layer adds node routing on top of GLINK. Each processor gets a node ID. Services register on port numbers. This is essentially a lightweight socket layer.

### QMI (Qualcomm Messaging Interface)
**Complexity: VERY HIGH | Effort: 3-4 weeks**

Binary RPC protocol with TLV (Type-Length-Value) encoding:
- **Service IDs**: WDS (0x0001), DMS (0x0002), NAS (0x0003), QOS (0x0004), etc.
- **Message IDs**: WDS_START_NETWORK_REQ (0x0020), WDS_START_NETWORK_RESP (0x0021)
- **TLV encoding**: each field has type (1 byte), length (2 bytes), value (N bytes)
- **SDU (Service Domain Unit)**: groups related services

Minimum QMI message sequence for data:
1. **INDICATION_REGISTER** (register for indications)
2. **WDS_SET_DATA_FORMAT** (configure data path - big-endian MAP headers)
3. **WDS_START_NETWORK_REQ** (start data connection, provide APN)
4. **WDS_START_NETWORK_RESP** (get bearer ID, IP config)
5. DHCP may or may not be needed depending on APN config

The QMI WDS messages alone require implementing:
- QMI message header encoder/decoder
- TLV field parser
- At least 10 different message types
- Transaction ID tracking

### rmnet/MAP (Multiplexing and Aggregation Protocol)
**Complexity: MEDIUM | Effort: 1-2 weeks**

Packet framing for IP data:
- **MAP v1 header**: command bit, pad bits, mux ID (5 bits), payload length (11 bits)
- **MAP v4 header**: adds checksum
- **MAP v5 header**: adds next header field
- **Aggregation**: multiple MAP packets in single transfer
- **Flow control**: MAP command packets for flow control
- Mux IDs route to different PDN (Packet Data Network) contexts

### IPA (IP Accelerator)
**Complexity: MEDIUM | Effort: 3-5 weeks**

Hardware offload engine for IP data path:
- **GSI (Generic Software Interface)**: DMA engine for IP packet transfer
- **Registers**: ipa-reg (0x01e40000, 28KB), ipa-shared (0x01e47000, 8KB), gsi (0x01e04000, 176KB)
- **Channels**: command TX/RX, packet TX/RX
- **Endpoints**: modem TX, modem RX, AP TX, AP RX
- **Initialization**: complex sequence involving RPMH power domains and QMP messages
- **Advantage**: can bypass SMEM->GLINK->QRTR for actual IP data, using GSI DMA

---

## 4. Non-Linux Implementations

**None found.** The following were checked:
- FreeBSD: No QRTR support
- postmarketOS: Uses Linux kernel QRTR + userspace libqrtr
- Zephyr RTOS: No Qualcomm modem support
- QNX: Proprietary BSPs only
- Redox OS: Partially ported Linux drivers, no QRTR/GLINK
- FreeRTOS: No Qualcomm modem support

Linux is the **only OS** known to implement the full SMEM->GLINK->QRTR->QMI stack. This makes bare-metal porting very risky due to lack of reference implementations.

---

## 5. Alternative Paths

### Path A: IPA Shortcut (Bypass QRTR for data)
- Use SMEM + GLINK + QMI only for **control plane** (modem setup)
- Use IPA GSI DMA for **data plane** (actual IP packets)
- Reduces need for QRTR/rmnet in data path
- Still requires GLINK + QMI (very complex)
- **Estimated effort**: 8-12 months total
- **Risk**: IPA initialization requires QMP and RPMH interactions

### Path B: USB Ethernet Adapter (RECOMMENDED)
- Use DWC3 USB host at 0x0a600000 (xHCI)
- Plug in USB-C to ethernet adapter (RTL8152/ASIX)
- Required: USB host driver (xHCI) + CDC ECM driver
- Rust crates available: `rust-osdev/xhci`, `crab-usb`, `usb-oxide`
- Redox OS has full xHCI implementation in Rust
- **Estimated effort**: 1-2 weeks for basic networking
- **Risk**: Low (standard protocols, well-documented)

### Path C: Full Modem Stack
- Implement all 5 layers: SMEM -> GLINK -> QRTR -> QMI -> rmnet
- **Estimated effort**: 14-18 months
- **Risk**: Very high (undocumented protocols, no reference implementations)

---

## 6. Implementation Phases (if proceeding)

### Phase 1: SMEM (1-2 weeks)
- Map SMEM memory at 0x80900000
- Implement TCSR mutex #3 for hardware locking
- Implement SMEM item lookup (find items by ID)
- Test: read known items from bootloader-initialized SMEM

### Phase 2: GLINK SMEM Transport (4-6 weeks)
- Study `drivers/rpmsg/qcom_glink_smem.c` in detail
- Implement FIFO pair management in SMEM items
- Implement channel open/close negotiation
- Implement TX/RX with interrupt-based doorbell
- Test: verify GLINK channel negotiation with modem

### Phase 3: QRTR + QMI (3-4 weeks)
- Implement QRTR packet routing
- Implement QMI TLV encoder/decoder
- Send WDS SET DATA FORMAT
- Send WDS START NETWORK
- Test: verify data connection establishment

### Phase 4: Data Path (2-3 weeks)
- Option A: Implement rmnet/MAP framing
- Option B: Program IPA GSI channels for DMA transfer
- Integrate with smoltcp for TCP/IP
- Test: ping through 4G connection

---

## 7. Key Technical Risks

1. **Protocol Stability**: GLINK and QRTR are internal Qualcomm protocols. They may change between SoC revisions without notice.

2. **Firmware Dependency**: The modem requires firmware files (mba.mbn, qdsp6sw.mbn). These are binary blobs loaded by the Linux remoteproc subsystem. Bare-metal would need to load these too.

3. **Power Domain Complexity**: The modem requires 3 RPMH power domains (CX, MX, MSS). RPMH itself requires the RPMH processor (a separate management processor) to aggregate votes.

4. **Boot Order**: The modem must not respond to GLINK until Linux remoteproc boots it. A bare-metal kernel would need to replicate the remoteproc boot sequence.

5. **No Test Environment**: Without a reference implementation outside Linux, debugging the bare-metal stack requires the real hardware (Pixel 5) at all times. QEMU cannot emulate the X52 modem.

---

## 8. Hardware Register Map Summary

| Register Block | Physical Address | Size | Notes |
|----------------|-------------------|------|-------|
| SMEM | 0x80900000 | 2MB | Shared memory AP<->modem |
| SMEM Global Heap | 0x80900000+512 | 512B | Header with item count/flags |
| TCSR Mutex | 0x01f40000 | 128KB | Hardware mutex for SMEM |
| Modem Remoteproc | 0x04080000 | 16KB | Modem register space |
| GLINK Mailbox | 0x17c00000 | 64KB | apss_shared mailbox |
| IPA | 0x01e40000 | 28KB | IP Accelerator registers |
| IPA Shared | 0x01e47000 | 8KB | IPA shared memory |
| GSI | 0x01e04000 | 176KB | DMA engine for IPA |
| RMTFS | 0x94600000 | 2MB | Remote file system for modem |
| DWC3 USB | 0x0a600000 | 56KB | USB controller (xHCI) |
| WiFi (WCN3990) | 0x18800000 | 8MB | WiFi (AHB bus, no PCI) |

---

## 9. Decision Matrix

| Criterion | USB Ethernet | IPA Shortcut | Full Modem Stack |
|----------------------|---------------|-------------------|---------------------|
| Implementation Time | 1-2 weeks | 8-12 months | 14-18 months |
| Protocol Complexity | Low (CDC ECM) | Medium (IPA+GSI) | Very High (5 layers) |
| External Hardware | Yes (adapter) | No | No |
| Bandwidth | 100Mbps | 100+ Mbps | 100+ Mbps |
| Reliability | High (standard) | Medium (complex) | Low (fragile) |
| Risk Level | Low | Medium | Very High |

---

## 10. Recommendation

**Primary: USB Ethernet adapter** (1-2 weeks)
- Fastest path to network connectivity
- Well-documented USB + CDC ECM protocols
- Existing Rust xHCI implementations available
- Standard USB-C to ethernet adapters are cheap (~$15)

**Secondary: IPA shortcut** (if USB not feasible)
- Better bandwidth, no external hardware needed
- Still very complex (8-12 months)

**Not recommended: Full modem stack** (14-18 months)
- Only justified if no USB alternative exists
- Extreme complexity with no reference implementations

---

## Sources
- SC7180 Device Tree: `arch/arm64/boot/dts/qcom/sc7180.dtsi` (Linux kernel)
- G-Link User Guide: 80-P2598-1 REV C (Scribd)
- Qualcomm bare-metal: 80-VB419-99
- Linaro IPA: "IPA Linux Driver Overview"
- Linux QRTR: `net/qrtr/qrtr.c`
- Linux GLINK: `drivers/rpmsg/qcom_glink_smem.c`
- Linux QMI: `libqmi/data/qmi-service-wds.json`
- Linux rmnet: `docs.kernel.org/networking/device_drivers/cellular/qualcomm/rmnet.html`
- Redox OS xHCI: `/Users/mac8684/Downloads/drivers-master/usb/xhcid/`
