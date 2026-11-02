/* Both regions are internal SRAM; load this test with a debugger after reset. */
MEMORY
{
    /* Leave the first 512 bytes for boot arguments; use this region for data and stack. */
    RAM (xrw)   : ORIGIN = 0x20000200, LENGTH = 64K - 512
    /* cortex-m-rt calls the code region FLASH, but this address is internal SRAM. */
    FLASH (rx)  : ORIGIN = 0x20010000, LENGTH = 64K
}

SECTIONS
{
    /* Fix the RTT control block address for the debugger; rtt_init! initializes it. */
    .segger_rtt ORIGIN(RAM) (NOLOAD) : ALIGN(4)
    {
        KEEP(*(.segger_rtt));
    } > RAM
} INSERT BEFORE .data;
