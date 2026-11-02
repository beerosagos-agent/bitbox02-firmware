// SPDX-License-Identifier: Apache-2.0

use crate::hardware::{
    self as hw, AR, BUSY, CCR, CR, DCR1, DCR2, DLR, DR, GPIOD, GPIOH, GPIOI, HLCR, HSPI, SR, TCR,
};
use core::ptr::{read_volatile, write_volatile};
use rtt_target::rprintln;

const CONFIG0: u32 = 0x1000; // Byte address in HyperBus register space.
const DEFAULT_CONFIG0: u16 = 0x8f2f; // Seven-clock latency, fixed 2x latency, 34-ohm drive.

fn reset() {
    hw::pin(GPIOD, 1, false);
    hw::delay_us(10);
    hw::pin(GPIOD, 1, true);
    hw::delay_us(1000); // Exceeds the 150 us power-up/reset recovery requirement.
}

fn start(address: u32, register: bool, reading: bool) {
    hw::begin(HSPI, reading);
    hw::modify(HSPI + DCR1, 1 << 24, u32::from(register) << 24);
    // WZL selects zero-latency register writes; the controller leaves RWDS undriven for them.
    hw::write(
        HSPI + HLCR,
        (3 << 16) | (7 << 8) | 1 | if register { 2 } else { 0 },
    );
    // Eight address/data lines, DDR, 32-bit address, RWDS input for reads.
    hw::write(
        HSPI + CCR,
        (4 << 8) | (1 << 11) | (3 << 12) | (4 << 24) | (1 << 27) | (1 << 29),
    );
    // A single FIFO word avoids software-induced clock stalls and stays below tCSM = 4 us.
    hw::write(HSPI + DLR, if register { 1 } else { 3 });
    hw::write(HSPI + AR, address);
}

fn read_register(address: u32) -> u16 {
    start(address, true, true);
    hw::wait(HSPI + SR, 0x7f << 8, 2 << 8); // Wait for both FIFO bytes before a halfword read.
    let value = unsafe { read_volatile((HSPI + DR) as *const u16) };
    hw::finish(HSPI);
    value
}

fn write_register(address: u32, value: u16) {
    start(address, true, false);
    unsafe { write_volatile((HSPI + DR) as *mut u16, value) };
    hw::finish(HSPI);
}

fn read_word(address: u32) -> u32 {
    start(address, false, true);
    hw::wait(HSPI + SR, 0x7f << 8, 4 << 8);
    let value = hw::read(HSPI + DR);
    hw::finish(HSPI);
    value
}

fn write_word(address: u32, value: u32) {
    start(address, false, false);
    hw::write(HSPI + DR, value);
    hw::finish(HSPI);
}

fn init() -> u32 {
    hw::pin(GPIOD, 1, true);
    hw::gpio(GPIOD, 1, None); // PD1: RAM_RESET, active low.
    hw::pin(GPIOH, 9, true);
    // PH9 CS, PH10..15 DQ0..5; PI0/1 DQ6/7, PI2 RWDS, PI3 clock. All AF8.
    for pin in 9..=15 {
        hw::gpio(GPIOH, pin, Some(8));
    }
    for pin in 0..=3 {
        hw::gpio(GPIOI, pin, Some(8));
    }
    hw::write(HSPI + CR, 0);
    hw::write(HSPI + DCR1, (4 << 24) | (24 << 16) | (7 << 8));
    hw::write(HSPI + DCR2, 1); // 80 MHz kernel / 2 = 40 MHz clock, DDR data.
    hw::wait(HSPI + SR, BUSY, 0); // Prescaler writes trigger automatic HSPI calibration.
    hw::write(HSPI + TCR, 1 << 28); // Quarter-cycle output hold, no SDR sample shifting.
    hw::write(HSPI + CR, 1);
    reset();
    let id0 = read_register(0);
    let id1 = read_register(2);
    rprintln!("HyperRAM ID: {:#06x} {:#06x}", id0, id1);
    // Both supported HyperRAM 2.0 families reset to DEFAULT_CONFIG0.
    let (manufacturer, size): (&str, u32) = match (id0, id1) {
        (0x0c83, 0x0001) => ("ISSI", 8 * 1024 * 1024), // IS66/67WVH8M8F: 64 Mbit.
        (0x0e86, 0x0001) => ("Winbond", 32 * 1024 * 1024), // W958D8NBYA: 256 Mbit.
        _ => panic!("unsupported HyperRAM ID"),
    };
    rprintln!("HyperRAM: {}, {} MiB", manufacturer, size / (1024 * 1024));
    // DEVSIZE encodes the byte capacity as log2(size) - 1.
    hw::modify(HSPI + DCR1, 0x1f << 16, (size.ilog2() - 1) << 16);
    assert_eq!(
        read_register(CONFIG0),
        DEFAULT_CONFIG0,
        "HyperRAM CR0 after reset"
    );
    // Change only the drive strength, then prove that the external reset wire restores it.
    write_register(CONFIG0, DEFAULT_CONFIG0 | (1 << 12));
    assert_eq!(
        read_register(CONFIG0),
        DEFAULT_CONFIG0 | (1 << 12),
        "HyperRAM CR0 write"
    );
    reset();
    assert_eq!(
        read_register(CONFIG0),
        DEFAULT_CONFIG0,
        "HyperRAM hardware RESET"
    );
    size
}

pub fn test() {
    rprintln!("HyperRAM: 40 MHz, destructive full memory test");
    let size = init();
    for bit in 0..32 {
        for value in [1 << bit, !(1 << bit)] {
            write_word(0, value);
            assert_eq!(read_word(0), value, "HyperRAM walking bit {}", bit);
        }
    }
    for inverted in [false, true] {
        rprintln!("HyperRAM: fill all words, inverted={}", inverted);
        for address in (0..size).step_by(4) {
            write_word(address, crate::pattern(address, inverted));
            if (address + 4).is_multiple_of(8 * 1024 * 1024) {
                rprintln!(
                    "HyperRAM filled {} / {} MiB",
                    (address + 4) / (1024 * 1024),
                    size / (1024 * 1024)
                );
            }
        }
        // Idle longer than a complete refresh interval before checking the entire array.
        hw::delay_us(100_000);
        rprintln!("HyperRAM: verify all words after idle");
        for address in (0..size).step_by(4) {
            assert_eq!(
                read_word(address),
                crate::pattern(address, inverted),
                "HyperRAM {:#010x}",
                address
            );
            if (address + 4).is_multiple_of(8 * 1024 * 1024) {
                rprintln!(
                    "HyperRAM verified {} / {} MiB",
                    (address + 4) / (1024 * 1024),
                    size / (1024 * 1024)
                );
            }
        }
    }
    rprintln!("PASS: HyperRAM ID, reset, data bits, full address range, refresh");
}
