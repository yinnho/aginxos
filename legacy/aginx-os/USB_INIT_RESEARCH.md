# Pixel 5 (SC7180/SM7250) USB Gadget Bare-Metal Initialization Research

Complete register-level initialization sequence for USB gadget (CDC ACM serial)
on Pixel 5, derived from Linux kernel source analysis.

---

## Hardware Address Map

| Component | Physical Address | Notes |
|-----------|-----------------|-------|
| DWC3 Core | 0x0A600000 | SNPSID=0x5533330A (DWC_usb3 v3.30a) |
| QSCRATCH Wrapper | 0x0A6F8800 | DWC3 base + 0xF8800, size 0x400 |
| SNPS Femto HS PHY | 0x088E3000 | USB 2.0 High-Speed PHY, span 0x110 |
| QMP USB3-DP Combo PHY | 0x088E9000 | USB3 SerDes + DP, NOT needed for USB 2.0 |
| GCC | 0x00100000 | Global Clock Controller |

---

## Question (a): Exact Linux Initialization Order

From Linux kernel probe sequence (dwc3-qcom.c -> dwc3/core.c -> dwc3/gadget.c):

### Phase 1: GCC Clock/Power/Reset (dwc3-qcom.c + gcc-sc7180.c)

```
1. Power on USB30 GDSC
   write32(GCC + 0x0F004, 0x0)              // GDSCR: clear PWR_ON bits = power on
   poll until (GCC + 0x0F004) & BIT(31)      // Wait for PWR_ON status

2. Assert all resets (but preserve AHB state on warm boot)
   write32(GCC + 0x6A000, 0)                 // AHB2PHY_BCR: deassert (don't reset)

3. Enable AHB2PHY bridge clock (MUST be on before PHY BCR reset)
   write32(GCC + 0x6A004, read32(GCC + 0x6A004) | 0x1)  // AHB2PHY_CLK: enable
   poll until (GCC + 0x6A004) & BIT(31) == 0             // Wait for CLK_OFF to clear

4. USB30 PRIM block reset
   write32(GCC + 0x0F000, 1)                 // USB30_PRIM_BCR: assert
   udelay(100)
   write32(GCC + 0x0F000, 0)                 // USB30_PRIM_BCR: deassert
   poll until (GCC + 0x0F000) & 0x1 == 0     // Wait for reset to clear

5. QUSB2 PHY reset
   write32(GCC + 0x26000, 1)                 // QUSB2PHY_PRIM_BCR: assert
   udelay(100)
   write32(GCC + 0x26000, 0)                 // QUSB2PHY_PRIM_BCR: deassert
   poll until (GCC + 0x26000) & 0x1 == 0

6. Enable branch clocks (in this order):
   write32(GCC + 0x0502C, read32(GCC + 0x0502C) | 0x1)  // CFG_NOC_AXI_CLK
   write32(GCC + 0x0F010, read32(GCC + 0x0F010) | 0x1)  // MASTER_CLK
   write32(GCC + 0x0F018, read32(GCC + 0x0F018) | 0x1)  // MOCK_UTMI_CLK
   write32(GCC + 0x0F014, read32(GCC + 0x0F014) | 0x1)  // SLEEP_CLK
   write32(GCC + 0x0F050, read32(GCC + 0x0F050) | 0x1)  // PHY_AUX_CLK
   write32(GCC + 0x0F054, read32(GCC + 0x0F054) | 0x1)  // PHY_COM_AUX_CLK
   write32(GCC + 0x8201C, read32(GCC + 0x8201C) | 0x1)  // AGGR_NOC_AXI_CLK
   // Poll each for CLK_OFF to clear
```

### Phase 2: QSCRATCH Wrapper Configuration (dwc3-qcom.c)

Before DWC3 core init, the Qualcomm QSCRATCH wrapper must be configured:

```c
// Select UTMI clock (USB 2.0 PHY interface clock)
// Step 1: Disable PIPE clock
setbits32(QSCRATCH + 0x08, BIT(8));        // GENERAL_CFG: PIPE_UTMI_CLK_DIS
usleep_range(100, 1000);
// Step 2: Select UTMI clock source + software PIPE status
setbits32(QSCRATCH + 0x08, BIT(0) | BIT(3)); // GENERAL_CFG: PIPE_UTMI_CLK_SEL | PIPE3_PHYSTATUS_SW
usleep_range(100, 1000);
// Step 3: Re-enable clock
clrbits32(QSCRATCH + 0x08, BIT(8));        // GENERAL_CFG: clear PIPE_UTMI_CLK_DIS
```

### Phase 3: SNPS Femto HS PHY Init (phy-qcom-snps-femto-v2.c)

18-step init sequence at PHY base 0x088E3000:

```
1. QUSB2PHY PRIM BCR reset (already done in GCC phase above)
   // But the PHY driver does its own reset too:
   write32(GCC + 0x26000, 1);  udelay(100);  write32(GCC + 0x26000, 0);

2. Check PHY is alive: read32(PHY + 0x54) should be non-zero (COMMON0)

3. Enable override mode
   setbits32(PHY + 0x94, BIT(1))             // CFG0: CMN_CTRL_OVERRIDE_EN

4. Hold PHY in reset
   setbits32(PHY + 0x50, BIT(1))             // UTMI_CTRL5: POR=1

5. Clear FSEL (freq select)
   clrbits32(PHY + 0x54, 0x70)              // COMMON0: FSEL=0

6. PLL tuning
   setbits32(PHY + 0x58, BIT(5))            // COMMON1: PLLBTUNE=1

7. Reference clock select
   write32(PHY + 0xA0, (read32(PHY+0xA0) & ~0x3) | 0x2) // REFCLK_CTRL: REFCLK_SEL=2

8. VBUS detect (CRITICAL for gadget mode)
   setbits32(PHY + 0x58, BIT(4))            // COMMON1: VBUSVLDEXTSEL0=1
   setbits32(PHY + 0x60, BIT(0))            // CTRL1: VBUSVLDEXT0=1

9. Pixel 5 specific param overrides
   write32(PHY + 0x6C, 0x63)                // OVERRIDE_X0: disconnect + squelch
   write32(PHY + 0x70, 0x85)                // OVERRIDE_X1: amplitude + preemphasis
   write32(PHY + 0x74, 0x17)                // OVERRIDE_X2: rise/fall + crossover

10. Enable PHY (release from reset / power on)
    setbits32(PHY + 0x5C, BIT(0))           // COMMON2: VREGBYPASS=1
    setbits32(PHY + 0x64, BIT(3) | BIT(2))  // CTRL2: SUSPEND_N_SEL + SUSPEND_N
    setbits32(PHY + 0x3C, BIT(0))           // UTMI_CTRL0: SLEEPM=1 (exit sleep)
    clrbits32(PHY + 0x54, BIT(2))           // COMMON0: SIDDQ=0 (power ON)
    clrbits32(PHY + 0x50, BIT(1))           // UTMI_CTRL5: POR=0 (release reset)
    clrbits32(PHY + 0x64, BIT(3))           // CTRL2: SUSPEND_N_SEL=0
    clrbits32(PHY + 0x94, BIT(1))           // CFG0: CMN_CTRL_OVERRIDE_EN=0
```

### Phase 4: DWC3 Core Init (dwc3/core.c dwc3_core_init)

```
1. Write Linux version code to GUID (optional, for debugging)
   write32(DWC3 + 0xC128, LINUX_VERSION_CODE)

2. PHY setup (already done above)

3. Core soft reset
   reg = read32(DWC3 + 0xC710);              // DCTL
   reg |= DWC3_DCTL_CSFTRST;                 // BIT(30)
   reg &= ~DWC3_DCTL_RUN_STOP;               // clear BIT(31)
   write32(DWC3 + 0xC710, reg);
   poll until (DWC3 + 0xC710) & BIT(30) == 0  // Wait for CSFTRST to self-clear
   // v3.30a: may need up to 10 retries with 20ms sleep each

4. Setup GCTL (Global Control)
   reg = read32(DWC3 + 0xC110);
   reg &= ~DWC3_GCTL_PRTCAPDIR(~0);          // Clear port capability
   reg |= DWC3_GCTL_PRTCAPDIR(DEVICE_MODE);  // Set to device mode (0x2 << 12)
   // SC7180 quirks: keep SUSPHY clear during init
   // For v3.30a (>= 2.10a): set DSBLCLKGTNG if not FPGH
   write32(DWC3 + 0xC110, reg);

5. Set number of endpoints
   // Read from GHWPARAMS registers, or hardcode for SC7180

6. Setup GUSB2PHYCFG (clear SUSPHY before PHY init)
   reg = read32(DWC3 + 0xC200);              // GUSB2PHYCFG(0)
   reg &= ~DWC3_GUSB2PHYCFG_SUSPHY;          // BIT(6): clear suspend PHY
   // SC7180 quirks from DT:
   //   snps,dis_u2_susphy_quirk -> keep SUSPHY clear
   //   snps,dis_enblslpm_quirk -> clear ENBLSLPM
   write32(DWC3 + 0xC200, reg);

7. PHY power on (via PHY subsystem - we already did this)

8. Setup event buffers
   write32(DWC3 + 0xC400, event_buf_addr_lo);  // GEVNTADRLO(0)
   write32(DWC3 + 0xC404, 0);                   // GEVNTADRH(0)
   write32(DWC3 + 0xC408, buf_size);            // GEVNTSIZ(0)
   write32(DWC3 + 0xC40C, 0);                   // GEVNTCOUNT(0): clear stale events

9. Enable SUSPHY (after init is complete, unless quirk)
   setbits32(DWC3 + 0xC200, BIT(6));           // GUSB2PHYCFG: SUSPHY
```

### Phase 5: DWC3 Gadget Init + D+ Pull-up (dwc3/gadget.c)

```
1. Device configuration
   write32(DWC3 + 0xC700, (read32(DWC3+0xC700) & ~0x7) | 0x0); // DCFG: speed=HS

2. VBUS override via QSCRATCH (called from pre_run_stop notifier)
   setbits32(QSCRATCH + 0x30, BIT(24));       // SS_PHY_CTRL: LANE0_PWR_PRESENT
   setbits32(QSCRATCH + 0x10, BIT(20) | BIT(28)); // HS_PHY_CTRL: UTMI_OTG_VBUS_VALID | SW_SESSVLD_SEL

3. Start device: D+ pull-up assertion
   reg = read32(DWC3 + 0xC710);               // DCTL
   reg |= DWC3_DCTL_RUN_STOP;                  // BIT(31): start device
   write32(DWC3 + 0xC710, reg);
   // There is NO SOFTDISCONNECT bit to clear in DWC3!
   // Setting RUN_STOP causes the link state machine to transition
   // from Powered to Default (asserts D+ pull-up via PHY)

4. Wait for connection
   poll DSTS (0xC70C) until link state != powered
```

---

## Question (b): DWC3 DCTL Register Layout

### DCTL Register (offset 0xC704 / 0xC710)

```
Bit 31: RUN_STOP (RW)
  - 0 = Stop device, disconnect from bus
  - 1 = Start device, assert D+ pull-up
  - CONFIRMED WRITABLE on this hardware

Bit 30: CSFTRST (RW, self-clearing)
  - Core Soft Reset
  - Write 1 to assert, polls until self-clears
  - CONFIRMED WORKING on this hardware

Bit 29: LSFTRST (RW, self-clearing)
  - Link Soft Reset

Bits 28-27: RO, reserved

Bits 26-25: ULSTCHNGREQ (RW)
  - USB Link State Change Request

Bit 24: RO, reserved

Bits 23-20: RO (read-only status bits)

Bit 19: KEEP_CONNECT (RW)

Bit 18: RO, reserved

Bit 17: LW1:    NOT writable on this DWC3 v3.30a
Bit 16: SFTCONN: NOT writable on this DWC3 v3.30a (read-only status)
  - This bit shows the current soft connect state
  - It is NOT a control bit on DWC_usb3 v3.x
  - On older DWC3 (v1.x/v2.x), this was a control bit

Bit 15: RO, reserved

Bits 14-10: THRNLIMRXPKTCNT / various thresholds

Bit  9: IGNTMOUTINCR

Bit  8: RO

Bit  7: NOT SOFTDISCONNECT - this bit does NOT exist in DWC3!
  - In your code, DCTL_SOFTDISCONNECT is defined as BIT(7)
  - This is INCORRECT for DWC3 v3.x
  - DWC3 has NO SOFTDISCONNECT bit in DCTL
  - The D+ pull-up is controlled entirely by RUN_STOP

Bits 6-5: TRGTULST (RW)

Bits 4-0: RO / status
```

### Key Finding: No SOFTDISCONNECT in DWC3

DWC3 (unlike DWC2/musb) does NOT have a SOFTDISCONNECT bit. The D+ pull-up is controlled by:
- **RUN_STOP (bit 31)**: Setting this starts the device and asserts D+ pull-up
- The link state machine handles the rest

### Qualcomm QSCRATCH Wrapper

YES, there IS a Qualcomm-specific wrapper that needs configuration:

| Register | Offset | Purpose |
|----------|--------|---------|
| GENERAL_CFG | QSCRATCH + 0x08 | UTMI clock selection |
| HS_PHY_CTRL | QSCRATCH + 0x10 | VBUS override |
| SS_PHY_CTRL | QSCRATCH + 0x30 | SuperSpeed power |

The QSCRATCH wrapper must be configured BEFORE starting the DWC3 device:
1. **UTMI clock selection** (GENERAL_CFG): Select UTMI clock source for USB 2.0
2. **VBUS override** (HS_PHY_CTRL): Software-assert VBUS valid signals so DWC3 thinks VBUS is present

Without VBUS override, DWC3 will not transition from Powered state even when RUN_STOP is set.

---

## Question (c): QMP PHY Requirement

### Answer: QMP PHY is NOT needed for USB 2.0 HS operation.

The QMP USB3-DP combo PHY at 0x088E9000 handles:
- **USB 3.x SuperSpeed (5 Gbps)** - requires SerDes calibration, PCS configuration
- **DisplayPort (DP)** - alternate mode on the same pins

For USB 2.0 High-Speed (480 Mbps), only the **SNPS Femto HS PHY** at 0x088E3000 is needed. The D+ and D- pins for USB 2.0 are separate from the SuperSpeed TX/RX pins and are connected directly to the Femto PHY.

If you only need USB 2.0 gadget (CDC ACM serial), you can skip QMP PHY initialization entirely.

---

## Question (d): Type-C / CC Pin Configuration

### FUSB302 Type-C Controller

The Pixel 5 uses an FUSB302 Type-C port controller, typically on I2C at address 0x22.

For bare-metal USB gadget (UFP = Upstream Facing Port = device mode):

#### Is FUSB302 required?

**For a first attempt: NO.** You can work around it by:
1. Using a USB Type-A to Type-C cable with the correct orientation
2. Overriding VBUS detection in QSCRATCH (which we already do)

**For reliable operation: YES.** The FUSB302 handles:
- CC (Configuration Channel) pin pull-down resistors (Rd = 5.1k)
- Orientation detection (CC1 vs CC2)
- VBUS detection and threshold monitoring
- PD (Power Delivery) negotiation

#### Minimal FUSB302 Bare-Metal Init (UFP Gadget Mode)

```rust
// I2C address: 0x22 (7-bit)
const FUSB302_BASE: usize = 0x22; // I2C address

fn fusb302_init_gadget() -> bool {
    // 1. Reset
    i2c_write(FUSB302_BASE, 0x0C, 0x01);    // Reset: SW_RESET
    udelay(100);
    
    // 2. Power up all blocks
    i2c_write(FUSB302_BASE, 0x0B, 0x0F);    // Power: PWR_EN for all
    udelay(100);
    
    // 3. Configure as UFP (sink) with Rd pull-downs on both CC pins
    i2c_write(FUSB302_BASE, 0x02, 0x07);    // Switches0: PD1.0 enable + AUTO_CRC
    
    // 4. Set default Rd pull-down (5.1k) on CC pins
    //    Switches0[2:1] = CC pull-down configuration
    i2c_write(FUSB302_BASE, 0x02, 0x03);    // Switches0: enable CC1+CC2 Rd
    
    // 5. Enable VBUS detection
    i2c_write(FUSB302_BASE, 0x06, 0x07);    // Control0: enable all measurements
    
    // 6. Read Status0 to check VBUS
    let status0 = i2c_read(FUSB302_BASE, 0x0D);
    let vbus_ok = (status0 & 0x80) != 0;     // VBUSOK bit
    
    vbus_ok
}
```

#### CC Pin Behavior without FUSB302

Without FUSB302, the CC pins are floating. A USB host will NOT detect the device because:
- Host checks CC pins for Rd pull-downs to detect a UFP
- Without Rd, host sees nothing connected

**Workaround**: Use a USB-A to USB-C cable with a built-in 5.1k pull-down resistor on the CC pin. These cables are common and force the host to recognize a device.

---

## Question (e): ABL Bootloader USB Configuration

### ABL (Android Bootloader) USB Fastboot

ABL is Qualcomm's bootloader based on LK (Little Kernel). It uses USB gadget for fastboot.

#### What ABL does for USB:

1. **Minimal GCC clock setup**: Enables USB30 PRIM clocks with hardcoded values
2. **DWC3 peripheral mode**: Configures DWC3 in device-only mode via GCTL
3. **QSCRATCH VBUS override**: Sets software VBUS valid (since no real VBUS detection)
4. **SNPS Femto PHY init**: Basic PHY bringup (similar to Linux but simplified)
5. **USB gadget driver**: LK's built-in USB gadget stack for fastboot

#### Key insight for bare-metal:

When your bare-metal kernel takes over from ABL:
- ABL may have already configured GCC clocks and PHY
- Your GCTL read (0x00102000) shows ABL already set DEVICE mode
- Your DCTL read (0x00F00000) shows device is in a known initial state
- The QSCRATCH wrapper may NOT have VBUS override set (ABL might clear it on exit)

This means you should either:
1. **Full re-init**: Reset everything from scratch (recommended)
2. **Preserve ABL state**: Only configure what ABL didn't (risky)

---

## Complete Bare-Metal Init Sequence (Recommended)

This is the exact order that should work for your kernel:

```rust
pub fn init_usb_gadget() -> bool {
    // ========================================
    // Phase 1: GCC Clocks, Power, Resets
    // ========================================
    gcc::enable_usb30_clocks();  // Your existing function, already correct
    
    // ========================================
    // Phase 2: SNPS Femto HS PHY
    // ========================================
    init_hsphy();  // Your existing function, already correct
    
    // ========================================
    // Phase 3: QSCRATCH Wrapper (MISSING - this is likely the issue!)
    // ========================================
    
    // 3a. Select UTMI clock
    let qscratch = 0x0A6F_8800usize;
    
    // Disable PIPE clock
    setbits32(qscratch + 0x08, 1 << 8);     // GENERAL_CFG: PIPE_UTMI_CLK_DIS
    delay(1000);
    
    // Select UTMI clock source
    setbits32(qscratch + 0x08, (1 << 0) | (1 << 3)); // PIPE_UTMI_CLK_SEL | PIPE3_PHYSTATUS_SW
    delay(1000);
    
    // Re-enable clock
    clrbits32(qscratch + 0x08, 1 << 8);     // GENERAL_CFG: clear PIPE_UTMI_CLK_DIS
    delay(1000);
    
    // 3b. VBUS Override (CRITICAL - without this, DWC3 won't connect!)
    setbits32(qscratch + 0x30, 1 << 24);     // SS_PHY_CTRL: LANE0_PWR_PRESENT
    setbits32(qscratch + 0x10, (1 << 20) | (1 << 28)); // HS_PHY_CTRL: UTMI_OTG_VBUS_VALID | SW_SESSVLD_SEL
    
    // ========================================
    // Phase 4: DWC3 Core Init
    // ========================================
    
    // 4a. Verify DWC3 present
    let snpsid = read32(0x0A600000 + 0xC120);  // GSNPSID
    if snpsid == 0 || snpsid == 0xFFFFFFFF { return false; }
    
    // 4b. Core soft reset
    let mut dctl = read32(0x0A600000 + 0xC710);  // DCTL (NOTE: offset is 0xC710 in newer IP)
    dctl |= 1 << 30;    // CSFTRST
    dctl &= !(1 << 31); // Clear RUN_STOP
    write32(0x0A600000 + 0xC710, dctl);
    // Poll until CSFTRST self-clears
    for _ in 0..10000 {
        if read32(0x0A600000 + 0xC710) & (1 << 30) == 0 { break; }
        delay(100);
    }
    
    // 4c. Set device mode in GCTL
    let gctl = read32(0x0A600000 + 0xC110);
    let gctl = (gctl & !(0x3 << 12)) | (0x2 << 12); // PRTCAPDIR = DEVICE
    write32(0x0A600000 + 0xC110, gctl);
    
    // 4d. Clear SUSPHY in GUSB2PHYCFG before init
    let usb2phycfg = read32(0x0A600000 + 0xC200);
    write32(0x0A600000 + 0xC200, usb2phycfg & !(1 << 6)); // Clear SUSPHY
    
    // 4e. Setup event buffer
    let evnt_addr = &EVENT_BUF as *const _ as u32;
    write32(0x0A600000 + 0xC400, evnt_addr);    // GEVNTADRLO
    write32(0x0A600000 + 0xC404, 0);             // GEVNTADRH
    write32(0x0A600000 + 0xC408, 4096);          // GEVNTSIZ
    write32(0x0A600000 + 0xC40C, 0);             // GEVNTCOUNT: clear
    
    // 4f. Enable device events
    write32(0x0A600000 + 0xC708, 
        (1 << 0) |   // DISCONN
        (1 << 1) |   // USBRST
        (1 << 2) |   // CONNECT_DONE
        (1 << 3));   // ULSTCHNG
    
    // 4g. Configure DCFG for High Speed
    let dcfg = read32(0x0A600000 + 0xC700);
    write32(0x0A600000 + 0xC700, (dcfg & !0x7) | 0x0); // Speed = HS
    
    // 4h. Configure EP0
    // ... (your existing EP0 setup code)
    
    // 4i. Enable EP0
    write32(0x0A600000 + 0xC720, (1 << 0) | (1 << 1)); // DALEPENA
    
    // ========================================
    // Phase 5: Start Device (D+ Pull-up)
    // ========================================
    
    // Set RUN_STOP to assert D+ pull-up and start device
    // NOTE: Do NOT try to clear SOFTDISCONNECT - it doesn't exist in DWC3!
    let dctl = read32(0x0A600000 + 0xC710);
    write32(0x0A600000 + 0xC710, dctl | (1 << 31)); // RUN_STOP
    
    true
}
```

---

## Root Cause Analysis: Why Host Doesn't Detect Device

Based on the code in `usb_dwc3.rs`, the most likely issues are:

### Issue 1: Missing QSCRATCH VBUS Override (MOST LIKELY)

Your `init_v2()` function does NOT set the QSCRATCH HS_PHY_CTRL register. Without VBUS override, the DWC3 controller never sees a valid VBUS and will not transition from the Powered state, even when RUN_STOP is set.

**Fix**: Add VBUS override before starting the device:
```rust
// In init_v2(), before write32(DCTL, ...):
let qscratch = 0x0A6F_8800usize;
unsafe {
    // SS_PHY_CTRL: LANE0_PWR_PRESENT
    let ss = core::ptr::read_volatile((qscratch + 0x30) as *const u32);
    core::ptr::write_volatile((qscratch + 0x30) as *mut u32, ss | (1 << 24));
    // HS_PHY_CTRL: UTMI_OTG_VBUS_VALID | SW_SESSVLD_SEL
    let hs = core::ptr::read_volatile((qscratch + 0x10) as *const u32);
    core::ptr::write_volatile((qscratch + 0x10) as *mut u32, hs | (1 << 20) | (1 << 28));
}
```

### Issue 2: Missing UTMI Clock Selection

The QSCRATCH GENERAL_CFG needs UTMI clock selection for USB 2.0 operation. Without it, the PHY interface clock may not be routed correctly.

### Issue 3: Incorrect SOFTDISCONNECT Handling

Your code does:
```rust
write32(DCTL, (dctl_before | DCTL_RUN_STOP) & !DCTL_SOFTDISCONNECT);
```

Since DCTL_SOFTDISCONNECT (BIT(7)) is NOT a SOFTDISCONNECT bit in DWC3 v3.x, clearing it has no effect. However, it also doesn't hurt since it just clears a random bit. The real issue is the missing VBUS override.

### Issue 4: Possible DCTL Register Offset

Your code uses DCTL at offset 0xC704, but the DWC3 v3.30a header shows it should be at 0xC710. Check your readback values to confirm which offset works.

---

## DCTL Register Offset Verification

From the Linux kernel core.h:
```c
#define DWC3_DCTL       0xc704  // For DWC_usb3 (IP type 0x5533)
```

From your SNPSID = 0x5533330A, this is DWC_usb3, so 0xC704 is correct.

However, looking at your code you also reference 0xC710 (DGCMDPAR). Make sure you're using 0xC704 for DCTL reads/writes.

---

## Summary of Register Offsets for QSCRATCH

| Register | Offset from QSCRATCH base (0x0A6F8800) | Full Address | Bits |
|----------|----------------------------------------|--------------|------|
| GENERAL_CFG | 0x08 | 0x0A6F8808 | BIT(0)=PIPE_UTMI_CLK_SEL, BIT(3)=PIPE3_PHYSTATUS_SW, BIT(8)=PIPE_UTMI_CLK_DIS |
| HS_PHY_CTRL | 0x10 | 0x0A6F8810 | BIT(20)=UTMI_OTG_VBUS_VALID, BIT(28)=SW_SESSVLD_SEL |
| SS_PHY_CTRL | 0x30 | 0x0A6F8830 | BIT(24)=LANE0_PWR_PRESENT |

---

## GCC Register Offsets for USB

| Register | Offset from GCC base (0x00100000) | Full Address | Purpose |
|----------|-----------------------------------|--------------|---------|
| USB30_PRIM_GDSCR | 0x0F004 | 0x0010F004 | Power domain control |
| USB30_PRIM_BCR | 0x0F000 | 0x0010F000 | Block reset |
| QUSB2PHY_PRIM_BCR | 0x26000 | 0x00126000 | PHY reset |
| AHB2PHY_BCR | 0x6A000 | 0x0016A000 | AHB bridge reset |
| AHB2PHY_CLK | 0x6A004 | 0x0016A004 | AHB bridge clock |
| USB30_PRIM_MASTER_CLK | 0x0F010 | 0x0010F010 | Master clock |
| USB30_PRIM_SLEEP_CLK | 0x0F014 | 0x0010F014 | Sleep clock |
| USB30_PRIM_MOCK_UTMI_CLK | 0x0F018 | 0x0010F018 | UTMI clock |
| USB30_PRIM_PHY_AUX_CLK | 0x0F050 | 0x0010F050 | PHY aux clock |
| USB30_PRIM_PHY_COM_AUX_CLK | 0x0F054 | 0x0010F054 | PHY com aux clock |
| USB30_PRIM_CFG_NOC_AXI_CLK | 0x0502C | 0x0015020C | NOC AXI clock |
| USB30_PRIM_AGGR_NOC_AXI_CLK | 0x8201C | 0x0018201C | Aggregated NOC clock |
