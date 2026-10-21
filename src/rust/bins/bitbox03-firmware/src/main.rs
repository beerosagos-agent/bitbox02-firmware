// SPDX-License-Identifier: Apache-2.0

#![no_std]
#![no_main]

mod haptic;

#[cfg(feature = "board-stm32u5a9j-dk")]
use bitbox_board_stm32u5a9j_dk as board;
use bitbox_platform_stm32u5 as _;
use core::panic::PanicInfo;
use cortex_m_rt::entry;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    // Remove motor drive before logging and stopping, even if the test panics mid-pulse.
    haptic::disable();
    log::error!("{info}");
    halt()
}

fn halt() -> ! {
    // Stop at a debugger breakpoint, then wait indefinitely if execution is resumed.
    cortex_m::asm::bkpt();
    loop {
        cortex_m::asm::wfe();
    }
}

#[entry]
fn main() -> ! {
    // The existing board initialization sets up the clocks and HAL system tick used below.
    board::init();
    bitbox_debug::rtt_logger_init!();
    log::info!("BitBox03 firmware stub");
    // Run one blocking motor pulse per startup, then stop. Movement must be checked on
    // the board: this test does not read back whether the motor actually vibrated.
    haptic::test();
    halt()
}
