// SPDX-License-Identifier: Apache-2.0

#![no_std]
#![no_main]

mod flash;
mod hardware;
mod ram;

use cortex_m_rt::entry;
use rtt_target::rprintln;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    rprintln!("FAIL: {}", info);
    cortex_m::asm::bkpt();
    loop {
        cortex_m::asm::wfi();
    }
}

// Odd multiplication is bijective on u32: every aligned address gets a distinct word.
// Complementing the second pass makes every storage bit take both values.
fn pattern(address: u32, inverted: bool) -> u32 {
    let word = (address / 4).wrapping_mul(0x9e37_79b1) ^ 0xa5c3_69f0;
    if inverted { !word } else { word }
}

fn menu() {
    rprintln!("40 MHz memory tests; commands followed by Enter:");
    rprintln!("s: DESTRUCTIVE sampled flash + reset test");
    rprintln!("f: DESTRUCTIVE full 64 MiB flash, two passes");
    rprintln!("v: read-only full flash verification after f / power cycle");
    rprintln!("r: DESTRUCTIVE full HyperRAM test (size from ID)");
    rprintln!("t: repeat full flash verification for scope/stress; q stops");
    rprintln!("h: help");
}

#[entry]
fn main() -> ! {
    let channels = rtt_target::rtt_init! {
        up: { 0: { size: 1024, mode: rtt_target::ChannelMode::NoBlockSkip, name: "Terminal" } }
        down: { 0: { size: 32, name: "Commands" } }
        section_cb: ".segger_rtt"
    };
    rtt_target::set_print_channel(channels.up.0);
    let mut input = channels.down.0;
    // Let the debugger attach the RTT server before initialization or any test output.
    cortex_m::asm::bkpt();
    hardware::init();
    flash::init();
    menu();
    let mut command = [0];
    loop {
        if input.read(&mut command) == 0 {
            continue;
        }
        match command[0] {
            b's' => flash::sampled_test(),
            b'f' => flash::full_test(),
            b'v' => {
                flash::verify_all();
                rprintln!("PASS: full flash retention pattern");
            }
            b'r' => ram::test(),
            b't' => {
                rprintln!("Repeated reads; send q to stop after the current pass");
                for pass in 1u32.. {
                    flash::verify_all();
                    rprintln!("PASS: read stress {}", pass);
                    let mut stop = false;
                    while input.read(&mut command) != 0 {
                        stop |= command[0] == b'q';
                    }
                    if stop {
                        break;
                    }
                }
            }
            b'h' => menu(),
            _ => continue,
        }
        rprintln!("Ready (h for help)");
    }
}
