// SPDX-License-Identifier: Apache-2.0

use core::ptr::{read_volatile, write_volatile};

// STM32U5G9 non-secure register addresses; TrustZone must be disabled.
const RCC: usize = 0x4602_0c00;
const PWR: usize = 0x4602_0800;
pub const GPIOD: usize = 0x4202_0c00;
pub const GPIOE: usize = 0x4202_1000;
pub const GPIOH: usize = 0x4202_1c00;
pub const GPIOI: usize = 0x4202_2000;
pub const OCTOSPI: usize = 0x420d_1400;
pub const HSPI: usize = 0x420d_3400;

// Common OCTOSPI/HSPI register offsets.
pub const CR: usize = 0x000;
pub const DCR1: usize = 0x008;
pub const DCR2: usize = 0x00c;
pub const SR: usize = 0x020;
const FCR: usize = 0x024;
pub const DLR: usize = 0x040;
pub const AR: usize = 0x048;
pub const DR: usize = 0x050;
pub const CCR: usize = 0x100;
pub const TCR: usize = 0x108;
pub const IR: usize = 0x110;
pub const HLCR: usize = 0x200;

pub const BUSY: u32 = 1 << 5;
const TCF: u32 = 1 << 1; // Transfer complete.
const FTF: u32 = 1 << 2; // FIFO has reached the programmed threshold.
const CPU_MHZ: u32 = 80;

#[inline]
pub fn read(address: usize) -> u32 {
    // All callers use aligned peripheral registers, never cached external memory.
    unsafe { read_volatile(address as *const u32) }
}

#[inline]
pub fn write(address: usize, value: u32) {
    unsafe { write_volatile(address as *mut u32, value) }
}

pub fn modify(address: usize, mask: u32, value: u32) {
    write(address, (read(address) & !mask) | value);
}

pub fn cycles() -> u32 {
    cortex_m::peripheral::DWT::cycle_count()
}

pub fn delay_us(us: u32) {
    let start = cycles();
    while cycles().wrapping_sub(start) < us * CPU_MHZ {}
}

pub fn wait(address: usize, mask: u32, value: u32) {
    // Use a bounded iteration count even while the system clock is changing.
    for _ in 0..1_000_000 {
        if read(address) & mask == value {
            return;
        }
    }
    panic!("register {:#x} timeout: {:#x}", address, read(address));
}

pub fn pin(port: usize, pin: u32, high: bool) {
    // BSRR: low 16 bits set outputs; high 16 bits clear them atomically.
    write(port + 0x18, 1 << (pin + if high { 0 } else { 16 }));
}

pub fn gpio(port: usize, pin: u32, alternate: Option<u32>) {
    modify(port + 0x08, 3 << (2 * pin), 3 << (2 * pin)); // Very high output speed.
    modify(port + 0x0c, 3 << (2 * pin), 0); // No internal pull resistors.
    if let Some(af) = alternate {
        let shift = 4 * (pin % 8);
        modify(
            port + 0x20 + 4 * (pin / 8) as usize,
            15 << shift,
            af << shift,
        );
    }
    // MODER: 01 = GPIO output, 10 = alternate function.
    modify(
        port,
        3 << (2 * pin),
        if alternate.is_some() { 2 } else { 1 } << (2 * pin),
    );
}

pub fn init() {
    modify(RCC, 0, 1 << 8);
    wait(RCC, 1 << 10, 1 << 10);
    // OpenOCD's reset-init selects PLL1. Switch to HSI before stopping the PLL or lowering VOS.
    modify(RCC + 0x1c, 3, 1);
    wait(RCC + 0x1c, 3 << 2, 1 << 2);
    modify(RCC, 1 << 24, 0);
    wait(RCC, 1 << 25, 0);
    modify(RCC + 0x20, 15, 0); // HCLK = SYSCLK; cycle-counter delays assume 80 MHz.
    // HSI16 / M=1 * N=10 / R=2 = 80 MHz. PLLMBOOST /1 supplies the EPOD booster.
    write(RCC + 0x28, 2 | (3 << 2) | (1 << 18));
    write(RCC + 0x34, 9 | (1 << 9) | (1 << 16) | (1 << 24));
    modify(RCC + 0x94, 0, 1 << 2); // PWR clock.
    let _ = read(RCC + 0x94);
    modify(PWR + 0x0c, 3 << 16, (2 << 16) | (1 << 18)); // VOS2 + booster, up to 100 MHz.
    wait(PWR + 0x0c, 1 << 15, 1 << 15);
    wait(PWR + 0x3c, 1 << 15, 1 << 15);
    modify(0x4002_2000, 15, 4); // FLASH_ACR latency, before increasing HCLK.
    modify(RCC, 0, 1 << 24);
    wait(RCC, 1 << 25, 1 << 25);
    modify(RCC + 0x1c, 3, 3);
    wait(RCC + 0x1c, 3 << 2, 3 << 2);

    let mut core = cortex_m::Peripherals::take().unwrap();
    core.DCB.enable_trace();
    core.DWT.enable_cycle_counter();
    // GPIO D/E/H/I, OCTOSPIM, OCTOSPI1 and HSPI1 clocks; both memory kernels use SYSCLK.
    modify(
        RCC + 0x8c,
        0,
        (1 << 3) | (1 << 4) | (1 << 7) | (1 << 8) | (1 << 21),
    );
    modify(RCC + 0x90, 0, (1 << 4) | (1 << 12));
    let _ = read(RCC + 0x90);
    modify(RCC + 0xe4, 15 << 20, 0);
    // Route OCTOSPI1 clock, CS and IO[3:0] to OCTOSPIM port 1.
    write(0x420c_4004, 1 | (1 << 8) | (1 << 16));
    // Preload idle levels before connecting the flash. IO2/3 stay high in single SPI.
    for pin_number in [11, 14, 15] {
        pin(GPIOE, pin_number, true);
    }
    for pin_number in 10..=15 {
        gpio(
            GPIOE,
            pin_number,
            if pin_number < 14 { Some(10) } else { None },
        );
    }
    // 64 MiB, 8 clocks CS-high, delay block bypassed for SDR. Start at 1 MHz.
    write(OCTOSPI + DCR1, (25 << 16) | (7 << 8) | (1 << 3));
    write(OCTOSPI + DCR2, 79);
    write(OCTOSPI + CR, 1);
    delay_us(1000);
}

pub fn begin(base: usize, reading: bool) {
    wait(base + SR, BUSY, 0);
    write(base + FCR, 0x1b); // Clear transfer/error/status/timeout flags.
    modify(base + CR, 3 << 28, u32::from(reading) << 28);
}

pub fn finish(base: usize) {
    let start = cycles();
    loop {
        let status = read(base + SR);
        assert_eq!(status & 1, 0, "XSPI {:#x} transfer error", base);
        if status & TCF != 0 {
            break;
        }
        assert!(
            cycles().wrapping_sub(start) < CPU_MHZ * 100_000,
            "XSPI {:#x} timeout",
            base
        );
    }
    wait(base + SR, BUSY, 0);
    write(base + FCR, TCF);
}

pub fn flash_command(opcode: u8, address: Option<u32>, data: &mut [u8], reading: bool, quad: bool) {
    begin(OCTOSPI, reading);
    // Dedicated four-byte opcodes reach beyond 16 MiB without changing address mode.
    let address_mode = if address.is_some() {
        (1 << 8) | (3 << 12)
    } else {
        0
    };
    let data_mode = if data.is_empty() {
        0
    } else if quad {
        3 << 24
    } else {
        1 << 24
    };
    write(OCTOSPI + DLR, data.len().saturating_sub(1) as u32);
    // Sample SDR half a cycle later; 4FRQO uses eight explicitly configured dummy cycles.
    write(
        OCTOSPI + TCR,
        (1 << 30) | if reading && quad { 8 } else { 0 },
    );
    write(OCTOSPI + CCR, 1 | address_mode | data_mode);
    write(OCTOSPI + IR, opcode.into());
    if let Some(address) = address {
        write(OCTOSPI + AR, address);
    }
    let start = cycles();
    for byte in data {
        loop {
            let status = read(OCTOSPI + SR);
            assert_eq!(status & 1, 0, "OCTOSPI transfer error");
            if status & FTF != 0 || (reading && status & TCF != 0) {
                break;
            }
            assert!(
                cycles().wrapping_sub(start) < CPU_MHZ * 100_000,
                "OCTOSPI FIFO timeout"
            );
        }
        unsafe {
            if reading {
                *byte = read_volatile((OCTOSPI + DR) as *const u8);
            } else {
                write_volatile((OCTOSPI + DR) as *mut u8, *byte);
            }
        }
    }
    finish(OCTOSPI);
}
