// SPDX-License-Identifier: Apache-2.0

use crate::hardware::{self as hw, DCR2, GPIOE, OCTOSPI};
use rtt_target::rprintln;

const SIZE: u32 = 64 * 1024 * 1024;
const SECTOR_SIZE: u32 = 4096; // Smallest independently erasable unit.
const BLOCK_SIZE: u32 = 64 * 1024; // Faster full-device erase, using 4BER64.
const PAGE_SIZE: usize = 256; // Program operations must not cross a page.
const QE: u8 = 1 << 6; // Non-volatile quad enable in the status register.
// Exercise every sector-address bit, both sides of 16 MiB boundaries, and the last byte.
const SECTORS: [u32; 20] = [
    0,
    1 << 12,
    1 << 13,
    1 << 14,
    1 << 15,
    1 << 16,
    1 << 17,
    1 << 18,
    1 << 19,
    1 << 20,
    1 << 21,
    1 << 22,
    1 << 23,
    1 << 24,
    1 << 25,
    0x00ff_f000,
    0x01ff_f000,
    0x02ff_f000,
    0x0300_0000,
    SIZE - SECTOR_SIZE,
];

fn command(opcode: u8, data: &mut [u8], reading: bool) {
    hw::flash_command(opcode, None, data, reading, false);
}

fn status() -> u8 {
    let mut value = [0];
    command(0x05, &mut value, true); // RDSR: read status register.
    value[0]
}

fn wait_ready() {
    // Ten seconds covers the datasheet's worst-case 64 KiB erase; each operation is bounded.
    let start = hw::cycles();
    while status() & 1 != 0 {
        assert!(
            hw::cycles().wrapping_sub(start) < 800_000_000,
            "flash busy timeout"
        );
    }
}

fn write_enable() {
    // WEL clears after each program/erase, so enable each operation separately.
    command(0x06, &mut [], false);
    assert_eq!(status() & 3, 2, "write enable"); // WEL = 1, WIP = 0.
}

fn set_quad(enabled: bool) {
    let old = status();
    let new = if enabled { old | QE } else { old & !QE };
    if old != new {
        write_enable();
        command(0x01, &mut [new & !3], false); // WRSR; preserve all protection bits.
        wait_ready();
        assert_eq!(status() & !3, new & !3, "quad enable status");
    }
    for pin in [14, 15] {
        hw::pin(GPIOE, pin, true);
        hw::gpio(GPIOE, pin, if enabled { Some(10) } else { None });
    }
}

fn read_parameters() {
    // SRPV: volatile only. Select RESET on IO3, eight dummy clocks, no burst wrapping.
    command(0xc0, &mut [0xc0], false);
}

fn identify() {
    let mut id = [0; 3];
    command(0x9f, &mut id, true);
    rprintln!("JEDEC ID: {:02x?}", id);
    // ISSI, IS25WP family, 512 Mbit. Check before any destructive operation.
    assert_eq!(id, [0x9d, 0x70, 0x1a], "IS25WP512M ID");
}

pub fn init() {
    command(0xab, &mut [], false); // Release deep power-down.
    hw::delay_us(1000);
    wait_ready();
    command(0x66, &mut [], false); // Enable software reset.
    command(0x99, &mut [], false);
    hw::delay_us(1000);
    identify();
    read_parameters();
    set_quad(true);
    hw::write(OCTOSPI + DCR2, 1); // 80 MHz kernel / 2 = 40 MHz serial clock.
    identify();
    rprintln!("Flash: hardware quad SPI at 40 MHz");
}

fn reset_test() {
    set_quad(false); // IO3 is a reset input only when QE = 0.
    read_parameters();
    write_enable();
    hw::pin(GPIOE, 15, false);
    hw::delay_us(10); // RESET pulse minimum is 100 ns.
    hw::pin(GPIOE, 15, true);
    hw::delay_us(1000); // Allow reset recovery before polling.
    assert_eq!(status() & 3, 0, "hardware RESET did not clear WEL");
    identify();
    read_parameters();
    set_quad(true);
}

fn erase(address: u32, block: bool) {
    write_enable();
    // Four-byte-address 64 KiB block erase or 4 KiB sector erase.
    hw::flash_command(
        if block { 0xdc } else { 0x21 },
        Some(address),
        &mut [],
        false,
        false,
    );
    wait_ready();
}

fn program(address: u32, data: &mut [u8]) {
    assert!(!data.is_empty() && (address as usize % PAGE_SIZE) + data.len() <= PAGE_SIZE);
    write_enable();
    hw::flash_command(0x34, Some(address), data, false, true); // 4PPQ: quad page program.
    wait_ready();
}

fn read(address: u32, data: &mut [u8], quad: bool) {
    hw::flash_command(
        if quad { 0x6c } else { 0x13 },
        Some(address),
        data,
        true,
        quad,
    );
}

fn expected(address: u32, inverted: bool) -> u8 {
    (crate::pattern(address & !3, inverted) >> (8 * (address % 4))) as u8
}

fn verify(address: u32, length: u32, inverted: Option<bool>, quad: bool) {
    let mut buf = [0; PAGE_SIZE];
    for offset in (0..length).step_by(PAGE_SIZE) {
        let count = (length - offset).min(PAGE_SIZE as u32) as usize;
        read(address + offset, &mut buf[..count], quad);
        for (i, &actual) in buf[..count].iter().enumerate() {
            let a = address + offset + i as u32;
            let value = inverted.map_or(0xff, |inverted| expected(a, inverted));
            assert_eq!(actual, value, "flash address {:#010x}, quad={}", a, quad);
        }
    }
}

fn fill(address: u32, length: u32, inverted: bool) {
    let mut buf = [0; PAGE_SIZE];
    for offset in (0..length).step_by(PAGE_SIZE) {
        for (i, byte) in buf.iter_mut().enumerate() {
            *byte = expected(address + offset + i as u32, inverted);
        }
        program(address + offset, &mut buf);
    }
}

pub fn sampled_test() {
    rprintln!("Erasing 20 sampled sectors (80 KiB total)");
    // Walking ones/zeros plus checkerboards exercise each data wire in both directions.
    erase(0, false);
    verify(0, SECTOR_SIZE, None, true);
    let mut data = [0; 20];
    for bit in 0..8 {
        data[bit] = 1 << bit;
        data[8 + bit] = !(1 << bit);
    }
    data[16..].copy_from_slice(&[0x00, 0xff, 0x55, 0xaa]);
    let expected = data;
    program(0, &mut data);
    for quad in [false, true] {
        read(0, &mut data, quad);
        assert_eq!(data, expected, "walking data bits, quad={}", quad);
    }
    for inverted in [false, true] {
        for sector in SECTORS {
            erase(sector, false);
            // Erase programmed data: an initially blank sector alone cannot prove erase works.
            verify(sector, SECTOR_SIZE, None, true);
        }
        // Write ALL regions before reading any: immediate readback would miss address aliasing.
        for sector in SECTORS {
            fill(sector, SECTOR_SIZE, inverted);
        }
        for sector in SECTORS {
            verify(sector, SECTOR_SIZE, Some(inverted), true);
            verify(sector, SECTOR_SIZE, Some(inverted), false);
            // Unaligned reads straddle page boundaries; both adjacent pages hold distinct data.
            verify(sector + PAGE_SIZE as u32 - 3, 11, Some(inverted), true);
        }
        for boundary in [0x0100_0000, 0x0200_0000, 0x0300_0000] {
            verify(boundary - 3, 11, Some(inverted), true);
        }
    }
    reset_test();
    for sector in SECTORS {
        verify(sector, SECTOR_SIZE, Some(true), true);
    }
    rprintln!("PASS: quad/single SPI, data bits, addresses, erase/program, hardware reset");
}

pub fn verify_all() {
    for address in (0..SIZE).step_by(BLOCK_SIZE as usize) {
        verify(address, BLOCK_SIZE, Some(true), true);
        if (address + BLOCK_SIZE).is_multiple_of(8 * 1024 * 1024) {
            rprintln!(
                "Verified {} / 64 MiB",
                (address + BLOCK_SIZE) / (1024 * 1024)
            );
        }
    }
}

pub fn full_test() {
    for inverted in [false, true] {
        rprintln!("Full flash: erase/check, inverted={}", inverted);
        for address in (0..SIZE).step_by(BLOCK_SIZE as usize) {
            erase(address, true);
            verify(address, BLOCK_SIZE, None, true);
            if (address + BLOCK_SIZE).is_multiple_of(8 * 1024 * 1024) {
                rprintln!("Erased {} / 64 MiB", (address + BLOCK_SIZE) / (1024 * 1024));
            }
        }
        rprintln!("Full flash: program all pages");
        for address in (0..SIZE).step_by(BLOCK_SIZE as usize) {
            fill(address, BLOCK_SIZE, inverted);
            if (address + BLOCK_SIZE).is_multiple_of(8 * 1024 * 1024) {
                rprintln!(
                    "Programmed {} / 64 MiB",
                    (address + BLOCK_SIZE) / (1024 * 1024)
                );
            }
        }
        rprintln!("Full flash: verify all pages");
        for address in (0..SIZE).step_by(BLOCK_SIZE as usize) {
            verify(address, BLOCK_SIZE, Some(inverted), true);
        }
    }
    rprintln!("PASS: full 64 MiB flash, two patterns; power-cycle, reload, then v");
}
