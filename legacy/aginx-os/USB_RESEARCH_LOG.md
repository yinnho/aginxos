# USB CDC ACM — Experiment Log

## Goal
Get USB gadget CDC ACM working on Pixel 5 (SM7250/redfin) so a Mac host detects the device over USB-C.

## Current Status: BLOCKED
DWC3 controller is alive and configurable, but **PHY has no power** (PM6350 LDO regulators off). Mac never sees a USB device.

---

## Hardware Context
- DWC3 controller: 0x0A600000, SNPSID=0x5533330A
- QUSB2 v2 PHY: 0x088E3000 (TZ write-protected from EL1)
- PM6350 PMIC: SID=1 (SPMI)
- USB PHY regulators: LDO2 (1.8V, pldo), LDO3 (3.072V, pldo), LDO18 (0.904V, nldo)
- These regulators are **RPMh-managed** — direct SPMI writes may be silently ignored

---

## Experiments

### Exp 1: init_abl_takeover (warm init, no core reset)
**When:** Multiple sessions
**Approach:** Take over ABL's DWC3 config without resetting core or PHY. Disconnect (clear RUN_STOP), reconfigure, reconnect (set RUN_STOP).
**Result:** DWC3 init succeeds (USB OK), but EVC=0 (no USB events). Mac sees nothing.
**Why:** PHY has no power. DWC3 can be configured in software but without a working PHY, no electrical signal reaches the host.
**Verdict:** ❌ PHY power is the root blocker.

### Exp 2: init_cold / init_cold_v2 (full cold init with core reset)
**When:** Multiple sessions
**Approach:** Enable GCC USB30 clocks → DWC3 core soft reset → PHY init → configure DWC3 → connect.
**Result:** After GCTL_CORESOFTRESET, all DWC3 registers read as 0 (controller dies). SNPSID was valid before reset.
**Why:** Core soft reset may cause GDSC power domain collapse, or the DWC3 doesn't recover properly on this platform without full platform init (regulators, PHY reset sequence).
**Verdict:** ❌ Core soft reset kills DWC3.

### Exp 3: init_cold_smc (cold init with SMC IO for PHY writes)
**When:** Session 2026-04-29
**Approach:** Use SMC IO (scm_io_write32) for PHY register writes to bypass TZ write protection. DWC3 core reset via direct MMIO.
**Result:** DWC3 alive before reset, dead after reset (same as Exp 2). SMC IO for PHY didn't help because DWC3 died first.
**Verdict:** ❌ Same core reset issue as Exp 2.

### Exp 4: init_warm_smc (warm init with SMC IO for DWC3 writes)
**When:** Session 2026-04-29
**Approach:** Use SMC IO for all DWC3 register reads/writes to bypass potential cache/MMU issues.
**Result:** SMC IO reads of DWC3 registers return 0 (TZ doesn't support IO access for DWC3 address range). SMC IS_CALL_AVAIL returns 0 (available) but actual IO read returns 0.
**Why:** TZ firmware restricts SMC IO to certain address ranges. DWC3 (0x0A600000) is not in the allowed range. PHY registers (0x088E3000) might work but we didn't test due to AHB2PHY issue.
**Verdict:** ❌ SMC IO doesn't work for DWC3.

### Exp 5: PHY register reads (phy_bus_diag)
**When:** Session 2026-04-29
**Approach:** Read PHY registers at 0x088E3000+0x1A0 (PLL_STATUS) and 0x088E3000+0x210 (PORT_POWERDOWN).
**Result:** System hangs. No output after the read attempt.
**Why:** AHB2PHY bridge not properly initialized. enable_usb30_clocks_minimal() enables the AHB2PHY clock but does NOT reset the bridge. Without reset, the bridge is in undefined state and reads hang the bus.
**Note:** enable_usb30_clocks_debug() DOES include AHB2PHY BCR reset, which should fix this. But that function also resets QUSB2PHY, which might kill ABL's PHY state.
**Verdict:** ❌ Hangs. Need AHB2PHY BCR reset first.

### Exp 6: CMD-DB access via direct MMIO
**When:** Previous sessions
**Approach:** Read CMD-DB SRAM at PA 0x00C80000 to get RPMh VRM addresses for PM6350 LDO2/LDO3/LDO18.
**Result:** System hangs on first read.
**Why:** Initially thought to be MMU mapping bug (mmu_enable.S shared L2 tables). But mmu_enable.S is dead code — kernel runs on ABL's page tables. ABL may not map this region, or it may be TZ-protected.
**Verdict:** ❌ Inaccessible from EL1.

### Exp 7: CMD-DB access via SMC IO
**When:** Session 2026-04-29
**Approach:** Use scm_io_read32(0x00C80000) to read CMD-DB through TrustZone.
**Result:** Status=0xFFFFFFFF (error), value=0x00000000. TZ denies access.
**Why:** CMD-DB SRAM is TZ-protected. TZ firmware won't allow EL1 to read it even via SMC IO.
**Verdict:** ❌ TZ blocks CMD-DB access.

### Exp 8: RPMh TCS scan
**When:** Session 2026-04-29
**Approach:** Scan RPMh DRV TCS entries at 0x0AF00000 and 0x0AF20000 to recover VRM addresses left by ABL.
**Result:** DRV1 TCS0 has CMD_ENABLE=0xE0000000, CMD_WAIT=1, but all command slots empty (addr=0, data=0). ABL cleaned up TCS before jumping to kernel.
**Note:** 0x0AF00000 is DISP_RSC, not apps RSC. Apps RSC DRV0=0x0AF20000, DRV1=0x0AF30000, DRV2=0x0AF40000.
**Verdict:** ❌ No VRM addresses recoverable from TCS state.

### Exp 9: SPMI LDO scan and enable
**When:** Session 2026-04-29 (and earlier sessions)
**Approach:** Scan all 256 APIDs, find peripherals with LDO type (0x02, 0x06, 0x1A), enable them via SPMI observer writes.
**Result:** Unknown — output scrolled off screen. SPMI LDO found=X ok=X was printed but values not captured. USB still doesn't work afterward.
**Why:** Even if SPMI writes "succeed" (no error status), RPMh-managed regulators silently ignore direct writes. The enable bit might read back as set but the actual regulator output doesn't change.
**Verdict:** ❓ Writes may appear successful but regulators stay off.

### Exp 10: SPMI targeted LDO enable (PM6350 LDO2/LDO3/LDO18)
**When:** Session 2026-04-29
**Approach:** Use find_apid() to find specific PM6350 LDOs by PPID (LDO2=0x141, LDO3=0x142, LDO18=0x151), read their status, enable them.
**Result:** System hung. [11ldo] lines were blank.
**Why:** find_apid() scans the APID table (reads from SPMI core registers). This worked in the SPMI init section but may hang when called again during USB init. Or the PPIDs are wrong and find_apid() loops indefinitely.
**Verdict:** ❌ Hung. PPIDs may be wrong, or re-scanning APID table after init has issues.

### Exp 11: SMC IS_CALL_AVAIL probe
**When:** Session 2026-04-29
**Approach:** Check if TZ supports SMC IO read (0x02000501) and write (0x02000502).
**Result:** Both return 0 (available).
**Note:** Despite returning "available", actual IO reads to DWC3/CMD-DB addresses fail. The IS_CALL_AVAIL check only confirms the SMC handler exists, not that specific addresses are accessible.
**Verdict:** ✅ SMC IO exists, but only works for certain address ranges.

---

## Key Findings

1. **DWC3 controller works**: SNPSID=0x5533330A, registers readable/writable via direct MMIO when GDSC is powered
2. **Core soft reset kills DWC3**: After GCTL_CORESOFTRESET, all registers read as 0
3. **PHY registers hang bus**: AHB2PHY bridge must be reset before PHY register access
4. **CMD-DB inaccessible**: Both direct MMIO and SMC IO blocked
5. **RPMh TCS empty**: ABL cleaned up before handoff
6. **VRM addresses unknown**: Cannot be determined from public sources
7. **SMC IO restricted**: Works for some addresses (PHY?) but not DWC3 or CMD-DB

## Root Cause
PM6350 LDO2/LDO3/LDO18 regulators are not powered. The QUSB2 PHY needs these to transmit USB signals. Without PHY power, the DWC3 can be configured in software but no electrical signal reaches the host.

## Possible Paths Forward

### Path A: Fix AHB2PHY + PHY init
1. Use enable_usb30_clocks_debug() (includes AHB2PHY BCR reset)
2. Read PHY PLL status safely
3. If PLL not locked, try PHY init with direct MMIO (TZ may allow reads, only writes are blocked)
4. Do warm init (no core reset)
**Risk:** AHB2PHY BCR reset might kill DWC3 access temporarily

### Path B: RPMh VRM with hardcoded addresses
1. Boot a Linux kernel on Pixel 5 with debug logging to capture VRM addresses
2. Hardcode addresses in our kernel
3. Write RPMh TCS commands to enable regulators
**Risk:** Need a working Linux kernel first

### Path C: PMIC GPIO regulator control
1. Some USB PHY regulators might be GPIO-controlled rather than RPMh-only
2. Check if there's a direct GPIO enable path

### Path D: Preserve ABL's regulator state
1. Check if ABL leaves regulators enabled at kernel entry
2. If yes, don't let our init disable them (currently enable_usb30_clocks_minimal writes to GDSCR)
3. The GDSC power-on write (0x0) should be harmless if already on
**Risk:** ABL may disable regulators before jump

### Path E: Use ABL's USB fastboot as-is
1. Don't reinitialize DWC3 at all
2. Hook into ABL's fastboot USB stack
3. Replace descriptor responses with CDC ACM
**Risk:** Very complex, ABL's USB stack runs at EL1 and may be overwritten

### Path F: Boot Linux first, capture VRM addresses
1. Boot a mainline Linux kernel on Pixel 5
2. Enable RPMh regulator debug logging
3. Capture VRM addresses from kernel log
4. Use those addresses in our bare-metal kernel
**Risk:** Need working Linux kernel for redfin

---

## Register State at Kernel Entry (after USB init)
```
SNPSID = 0x5533330A  (DWC3 alive)
DCTL   = 0x00F00000  (RUN_STOP=0, ABL left disconnected)
DSTS   = 0x00120001  (CONNECTSPD=HS, stale from ABL)
GCTL   = 0x00102000  (Device mode, no reset)
```

After init_abl_takeover:
```
DCTL   = has RUN_STOP set
DSTS   = 0x00120001  (CONNECTSPD=HS, but just DCFG mirror)
EVC    = 0x00000000  (NO USB events — PHY dead)
```

---

## Critical Address Corrections (2026-04-29 Research)

Previous experiments used WRONG hardware addresses from SC7180/incorrect sources:

| Resource | Wrong Address (used in Exp 6,8) | Correct Address (sm6350.dtsi) |
|----------|----------------------------------|-------------------------------|
| CMD-DB SRAM | 0x00C80000 | **0x80860000** (128KB, DRAM region) |
| APPS_RSC DRV0 | N/A | **0x18200000** |
| APPS_RSC DRV1 | N/A | **0x18210000** |
| APPS_RSC DRV2 | 0x0AF00000 (DISP_RSC!) | **0x18220000** |
| TCS offset | 0xC00 (guessed) | **0xD00** (from DTS `qcom,tcs-offset`) |

**Why previous experiments failed:**
- Exp 6 (CMD-DB MMIO): Hung because 0x00C80000 is not mapped or is TZ-protected
- Exp 7 (CMD-DB SMC): Failed because TZ blocks that address range
- Exp 8 (RPMh TCS): Found empty TCS because 0x0AF00000 is DISP_RSC, not APPS_RSC

**Correct addresses are from mainline Linux sm6350.dtsi**, which covers SM7250 (same SoC family).

### CMD-DB Format (from mainline Linux cmd-db.c)
- Base: 0x80860000, Size: 128KB
- Magic bytes at offset 4: `{0xdb, 0x30, 0x03, 0x0c}`
- Header: version(4) + magic(4) + rsc_hdr[32](512) + checksum(4) + reserved(4) = 528 bytes
- Data starts at offset 528
- rsc_hdr: slv_id(u16), header_offset(u16), data_offset(u16), cnt(u16), version(u16), reserved[3]
- VRM entries have slv_id=4
- Target IDs: "ldoa2" (LDO2), "ldoa3" (LDO3), "ldoa18" (LDO18)
- VRM address: 20-bit value, bits[3:0]=register offset (0=voltage, 4=enable, 8=mode)

### Implementation
- `cmd_db.rs`: CMD-DB parser module (reads VRM addresses from 0x80860000)
- `rpmh.rs`: RPMh TCS driver (writes enable commands to APPS_RSC DRV2 at 0x18220000)
- Both integrated into USB init flow in main.rs step [11db]/[11r]

### Why This Time Is Different
1. CMD-DB at 0x80860000 is in DRAM region (identity-mapped by our MMU), not device MMIO
2. APPS_RSC at 0x18220000 is in the device range covered by L1 entry 0 (0-0x3FFF_FFFF)
3. TCS offset is 0xD00 (from DTS), not 0xC00 (guessed)
