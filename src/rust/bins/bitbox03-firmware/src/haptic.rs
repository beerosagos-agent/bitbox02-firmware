// SPDX-License-Identifier: Apache-2.0

// BitBoxV3.0 wiring to the DRV2603 (U8):
// PG10 -> MOTOR_PWM, driven by TIM15 channel 1 through alternate function 14.
// PG12 -> MOTOR_EN, active high.
// PG9  -> LRA/ERM, externally strapped high by R25 to select LRA mode.
// U8's differential outputs drive the motor through spring contacts J7/J8.
// PWM duty and braking behavior: https://www.ti.com/lit/ds/symlink/drv2603.pdf

// Reuse the board's C HAL bindings for clock queries and delays, and the Rust
// peripheral access crate (PAC) for direct GPIO, power, and timer register access.
use crate::board::ffi;
use bitbox_mcu_stm32u5::pac;

pub fn test() {
    // Claim the peripheral handles once. This startup stub assumes no earlier Rust
    // code has taken them; a second take would return None and panic here.
    let peripherals = pac::Peripherals::take().unwrap();
    let rcc = peripherals.RCC;

    // Enable access to the power controller and GPIOG. The readbacks provide the
    // required delay after enabling each clock before accessing its peripheral.
    rcc.ahb3enr().modify(|_, w| w.pwren().set_bit());
    let _ = rcc.ahb3enr().read();
    rcc.ahb2enr1().modify(|_, w| w.gpiogen().set_bit());
    let _ = rcc.ahb2enr1().read();

    let gpio = peripherals.GPIOG;
    // Preload PWM and EN low before switching the pins to output mode, so the
    // driver stays disabled while its pins and the timer are being configured.
    gpio.bsrr().write(|w| w.br10().set_bit().br12().set_bit());
    // Use no internal pull-ups or pull-downs on these three signals.
    gpio.pupdr()
        .modify(|_, w| w.pupd9().floating().pupd10().floating().pupd12().floating());
    // R25 straps PG9 to 3.3 V through 0 ohms; never drive it, even high (VDDIO2 is 1.8 V).
    gpio.moder()
        .modify(|_, w| w.mode9().input().mode12().output());
    // Push-pull actively drives both logic levels. Low slew speed is sufficient
    // for the 20 kHz control signal; PG10 will be connected to TIM15 below.
    gpio.otyper()
        .modify(|_, w| w.ot10().push_pull().ot12().push_pull());
    gpio.ospeedr()
        .modify(|_, w| w.ospeed10().low_speed().ospeed12().low_speed());
    // PG9/10/12 use the separate VDDIO2 supply. IO2SV declares that supply valid
    // and removes the I/O isolation; enabling the GPIO clock alone is insufficient.
    peripherals.PWR.svmcr().modify(|_, w| w.io2sv().set_bit());

    // Enable TIM15, wait for its clock, then reset it to a known configuration.
    rcc.apb2enr().modify(|_, w| w.tim15en().set_bit());
    let _ = rcc.apb2enr().read();
    rcc.apb2rstr().modify(|_, w| w.tim15rst().set_bit());
    rcc.apb2rstr().modify(|_, w| w.tim15rst().clear_bit());

    // TIM15 runs at PCLK2 when APB2 is undivided, or twice PCLK2 when prescaled.
    // PPRE2 encodings 0..=3 mean divide-by-one; 4..=7 select a divider of 2 or more.
    let mut clock_hz = unsafe { ffi::HAL_RCC_GetPCLK2Freq() };
    if rcc.cfgr2().read().ppre2().bits() >= 4 {
        clock_hz *= 2;
    }
    // Require an exact 20 MHz counter clock before enabling motor drive.
    // The hardware divides by PSC + 1, hence the subtraction below.
    assert!(clock_hz >= 20_000_000 && clock_hz.is_multiple_of(20_000_000));
    let prescaler = u16::try_from(clock_hz / 20_000_000 - 1).unwrap();
    let pwm = peripherals.TIM15;
    pwm.psc().write(|w| unsafe { w.psc().bits(prescaler) });
    // Count from 0 through 999: 1,000 ticks at 20 MHz give a 20 kHz PWM signal.
    // This is the driver's control frequency; U8 generates the motor's resonant drive.
    pwm.arr().write(|w| unsafe { w.arr().bits(999) });
    // DRV2603 LRA half strength is 75% duty; 0% requests active braking.
    // Its positive drive range is 50..100% duty, so 750/1,000 requests half the
    // full-scale output amplitude. This does not guarantee half the perceived vibration.
    pwm.ccr1().write(|w| unsafe { w.ccr().bits(750) });
    // PWM mode 1 holds the output high while the counter is below CCR1.
    // OC1PE buffers compare changes; an update event transfers them to the timer.
    pwm.ccmr1_output()
        .write(|w| w.oc1m().pwm_mode1().oc1pe().enabled());
    // Force an update to load the prescaler and compare value, then clear status flags.
    pwm.egr().write(|w| w.ug().set_bit());
    pwm.sr().reset();
    // CC1E enables channel 1; MOE opens TIM15's separate main output gate.
    pwm.ccer().write(|w| w.cc1e().set_bit());
    pwm.bdtr().write(|w| w.moe().set_bit());
    // ARPE buffers future period changes; CEN starts the counter.
    pwm.cr1().write(|w| w.arpe().set_bit().cen().set_bit());

    // Route TIM15_CH1 to PG10 using AF14. PWM now runs independently in hardware.
    gpio.afrh().modify(|_, w| w.afrel10().af14());
    gpio.moder().modify(|_, w| w.mode10().alternate());
    // Drive phase: enable U8 for nominally 100 ms at 75% duty. HAL_Delay blocks
    // this function using the existing system tick while TIM15 continues generating PWM.
    gpio.bsrr().write(|w| w.bs12().set_bit());
    unsafe { ffi::HAL_Delay(100) };
    // Brake phase: load 0% duty immediately with an update event. Keep EN high
    // for 50 ms because active braking requires the driver to remain enabled.
    pwm.ccr1().reset();
    pwm.egr().write(|w| w.ug().set_bit());
    unsafe { ffi::HAL_Delay(50) };
    // End the effect: disable U8, stop the counter, and close the timer output gate.
    gpio.bsrr().write(|w| w.br12().set_bit());
    pwm.cr1().modify(|_, w| w.cen().clear_bit());
    pwm.bdtr().modify(|_, w| w.moe().clear_bit());
    // Disconnect PG10 from TIM15 and leave it as an ordinary output held low.
    gpio.bsrr().write(|w| w.br10().set_bit());
    gpio.moder().modify(|_, w| w.mode10().output());
}

pub fn disable() {
    // The panic handler cannot borrow the peripheral; BSRR disables EN atomically.
    // Access the registers directly because the test may already own the PAC handles.
    let rcc = unsafe { &*pac::RCC::PTR };
    // A panic may happen before GPIO initialization, so check its clock first.
    if rcc.ahb2enr1().read().gpiogen().bit_is_set() {
        let gpio = unsafe { &*pac::GPIOG::PTR };
        // Clear only PG12 without a read-modify-write of the other outputs.
        // This removes motor drive immediately; it does not wait for active braking.
        gpio.bsrr().write(|w| w.br12().set_bit());
    }
}
