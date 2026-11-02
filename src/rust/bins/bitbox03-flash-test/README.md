# BitBox03 external memory test

For the custom STM32U5G9ZJT6Q board in `BitBoxV3.0.zip` (schematic V1.0):

- U11 IS25WP512M-JLLE: 64 MiB flash, OCTOSPI1 on PE10 (clock), PE11 (CS),
  PE12..15 (IO0..3). IO2/3 also serve as WP/RESET in single-line mode.
- U9 W958D8NBYA5I: 32 MiB HyperRAM, HSPI1 on PH9 (CS), PH10..15 (DQ0..5),
  PI0/1 (DQ6/7), PI2 (RWDS), PI3 (clock), PD1 (RESET).

The assembled board reports HyperRAM IDs `0x0c83 / 0x0001`, matching the ISSI
IS66/67WVH8M8F family: **8 MiB (64 Mbit)**. The test recognizes this pair and
the archived Winbond part's `0x0e86 / 0x0001`, configures the controller for the
detected capacity, and tests that entire range. Other IDs stop the test.

Both interfaces use a nominal **40 MHz clock**, derived from HSI16 through an
80 MHz system clock. Flash data transfers use quad SDR; HyperRAM uses eight-bit
DDR. This standalone diagnostic runs entirely from internal SRAM and does not
program internal flash or option bytes. No production firmware changes are needed.

The test starts at a breakpoint so RTT can be connected, then waits for a command.
**It does not automatically erase memory.** Failure prints `FAIL` with the
address/register and stops at a breakpoint. Successful commands return to the
menu. Peripheral waits and flash busy polling are bounded.

| Command | Coverage | Contents affected |
| --- | --- | --- |
| `s` | Walking ones/zeros, `00/ff/55/aa`, quad program, single/quad readback, address aliasing, unaligned/page/bank-boundary reads, erase verification, flash RESET wire | 20 selected 4 KiB sectors (80 KiB), including address zero and the last sector |
| `f` | Erase/check, program **every page**, verify **every byte**, then repeat with the complemented pattern | Entire 64 MiB flash; leaves the second pattern for retention checks |
| `v` | Verify every byte of the pattern left by `f`, without erasing or programming the array | None |
| `r` | HyperRAM IDs, RESET wire, walking data bits, two full address-pattern passes, idle refresh check | Entire detected HyperRAM: 8 MiB ISSI or 32 MiB Winbond |
| `t` | Repeat `v` for read stress and scope measurements; `q` stops after the current pass | None; requires successful `f` first |
| `h` | Show commands | None |

Enter one command at a time, followed by Enter. Full tests can take several
minutes. Flash progress is reported per 8 MiB during erase/program; `v` and `t`
also report verification progress. RTT output is nonblocking, so keep the reader
connected. Only a final `PASS` establishes completion.

Patterns depend on the complete word address. All locations are programmed
before verification so upper address aliasing cannot pass through immediate
write/readback. Complementary passes exercise both values of every storage bit;
erasing between them also proves erase works on programmed data. `s` samples
sector-address bits 12..25 and both sides of 16, 32 and 48 MiB boundaries.
It leaves the sampled sectors programmed. After a power cycle, run `v` before
`s` or `f`: rewriting any tested data would invalidate that retention check.

Initialization checks the flash ID at 1 MHz and 40 MHz, enables its nonvolatile
QE bit if needed, and sets volatile read parameters. Protection bits are
preserved. `s` temporarily clears QE to test RESET on IO3: it sets WEL, pulses
the pin, and requires WEL to clear before enabling quad mode and rereading data.
The flash remains quad-enabled when the test finishes.

HyperRAM uses indirect four-byte transactions so the FIFO never needs CPU
refilling while CS is low; transactions stay below the device's refresh limit.
Each full fill is followed by a 100 ms idle and a complete readback. These are
functional checks at 40 MHz, not sustained DMA or memory-mapped throughput tests.

Build from the repository root:

```sh
./scripts/bootstrap-cargo-config
cd src/rust
cargo build -p bitbox03-flash-test --target thumbv8m.main-none-eabihf --locked --offline
cd ../..
```

Power-cycle the board before the first run so both memories start in their
power-on interface modes. The MCU must have TrustZone disabled, matching
`Project 2.ioc`. With OpenOCD already serving GDB on port 3333:

```sh
arm-none-eabi-gdb -q src/rust/target/thumbv8m.main-none-eabihf/debug/bitbox03-flash-test \
  -ex 'set $vectors = 0x20010000' -x scripts/openocd.gdb
```

The startup breakpoint is intentional. At the GDB prompt, start RTT:

```gdb
monitor rtt setup 0x20000200 0x1000 "SEGGER RTT"
monitor rtt start
monitor rtt server start 19021 0
```

Connect `telnet localhost 19021` in another terminal, then enter `continue` in
GDB. The memory test menu will appear in telnet. Use RTT port 19021 for test
commands; OpenOCD's port 4444 accepts debugger commands. The existing `rtt_start`
GDB helper also opens a second channel which this binary does not provide, so
use the explicit commands above.

Alternatively, use J-Link's GDB server:

```sh
JLinkGDBServer -device STM32U5G9ZJ -if SWD -speed 4000 -port 2331 -RTTTelnetPort 19021
arm-none-eabi-gdb -q src/rust/target/thumbv8m.main-none-eabihf/debug/bitbox03-flash-test \
  -ex 'set $vectors = 0x20010000' -x scripts/jlink.gdb
```

Connect telnet to port 19021 and continue from the startup breakpoint.

An initial reset halt with `pc: 0xfffffffe msp: 0xfffffffc` is consistent with
an erased boot vector. `flash probe 0` probes internal flash and does not load
this test. The GDB scripts reset the MCU, load the ELF into SRAM, and set VTOR,
SP and PC. Do not reset again after loading: reset returns to the configured
boot address instead of starting the SRAM test.

OpenOCD's STM32U5 `reset init` also selects a 160 MHz PLL clock. The diagnostic
switches to HSI before stopping and reconfiguring PLL1 for its 80 MHz system
clock; it does not require the reset-default MSI clock on entry.

For board validation, run this sequence:

1. Run `s`, then `r`, and require each final `PASS`.
2. Run `f` and wait for its final `PASS` before removing power.
3. Remove board power completely, including any supply or back-power path through
   the debug probe. Restore power and reload the same ELF with the GDB script.
4. Reconnect RTT, continue, and run **`v` first**. Do not run `s` or `f` before
   this verification. Repeat cold starts and `v` as needed. HyperRAM is volatile
   and is not expected to retain data across power loss.
5. Run `t` for repeated read stress, and observe the erase/program stages of `f`
   for write load. Measure the supply at the memory devices and probe clock,
   CS, data, and RAM RWDS for voltage levels, ringing and timing margin. A
   software `PASS` cannot establish those electrical margins.
6. Repeat on several boards and at the intended supply/temperature corners,
   within all component ratings. Repeat at the eventual production clock and
   with production DMA/memory-mapped traffic before treating the design as
   validated for that operating point.

Register definitions follow ST's [STM32U5G9 CMSIS header](https://github.com/STMicroelectronics/cmsis-device-u5/blob/main/Include/stm32u5g9xx.h)
and [XSPI HAL](https://github.com/STMicroelectronics/stm32u5xx-hal-driver/blob/main/Src/stm32u5xx_hal_xspi.c).
Interface timing follows [ST AN5050](https://www.st.com/resource/en/application_note/dm00407776-octospi-interface-on-stm32-microcontrollers-stmicroelectronics.pdf).
Memory commands and timing follow the [ISSI flash datasheet](https://www.issi.com/WW/pdf/25LP-WP512M.pdf),
[ISSI HyperRAM datasheet](https://www.issi.com/WW/pdf/66-67WVH8M8FALL-BLL.pdf),
and [Winbond W958D8NBYA datasheet](https://www.mouser.com/datasheet/2/949/W958D8NBYA_256Mb_HyperRAM_TFBGA24_SDP_85C_datashee-2997645.pdf).
