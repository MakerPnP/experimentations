//! Implementation of the flash-algorithm for the MakerPnPControl-CORE board
//! PCB Revision: RevA1
//!
//! Run with this environment setup to see more log output
//! ```
//! RUST_LOG=trace,nusb=info,probe_rs::probe=info
//! ```
#![no_std]
#![no_main]

use defmt::{info, trace};
use embassy_executor::Spawner;
use {defmt_rtt as _, panic_probe as _};

use embassy_stm32::gpio::{Input, Level, Output, Pull, Speed};
use embassy_time::{block_for, Duration};

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("START");
    let p = rcc_setup::stm32h735g_init();
    info!("INIT COMPLETE");

    let mut fpga_creset_b = Output::new(p.PF15, Level::Low, Speed::Low);

    info!("Input pin PG5 (TIM1_ETR, J408:20)");
    let button = Input::new(p.PG5, Pull::Up);

    loop {
        info!("Holding FPGA in reset");
        // hold FPGA in RESET mode
        fpga_creset_b.set_low();

        info!("Press and release button to start FPGA");
        wait_for_button_press_release(&button);

        info!("Releasing FPGA reset");
        fpga_creset_b.set_high();

        info!("Press and release button to stop FPGA");
        wait_for_button_press_release(&button);
    }
}

/// Polarity: active LOW
/// Debounce time: 100us
fn wait_for_button_press_release(button: &Input) {
    loop {
        if button.is_low() {
            trace!("Waiting for stable LOW input signal");
            // debounce
            block_for(Duration::from_micros(100));
            if button.is_low() {
                info!("Release button");
                loop {
                    if button.is_high() {
                        trace!("Waiting for stable HIGH input signal");
                        // debounce
                        block_for(Duration::from_micros(100));
                        if button.is_high() {
                            return;
                        }
                    }
                }
            }
        }
    }
}

mod rcc_setup {

    use embassy_stm32::rcc::mux::Fmcsel;
    use embassy_stm32::rcc::{Hse, HseMode, *};
    use embassy_stm32::time::Hertz;
    use embassy_stm32::{Config, Peripherals};

    /// Sets up clocks for the stm32h735g mcu
    /// change this if you plan to use a different microcontroller
    pub fn stm32h735g_init() -> Peripherals {
        // setup power and clocks for an stm32h735g-dk run from an external 25 Mhz external oscillator
        let mut config = Config::default();
        config.rcc.hse = Some(Hse {
            freq: Hertz::mhz(50),
            mode: HseMode::Oscillator,
        });
        config.rcc.hsi = None;
        config.rcc.csi = false;
        config.rcc.pll1 = Some(Pll {
            source: PllSource::Hse,
            prediv: PllPreDiv::Div4,  // 12.5Mhz
            mul: PllMul::Mul44,       // 550Mhz
            divp: Some(PllDiv::Div1), // 550Mhz
            divq: Some(PllDiv::Div4), // 110Mhz
            divr: Some(PllDiv::Div2), // 275Mhz
        });
        config.rcc.pll2 = Some(Pll {
            source: PllSource::Hse,
            prediv: PllPreDiv::Div5,  // 10Mhz
            mul: PllMul::Mul40,       // 400Mhz
            divp: Some(PllDiv::Div5), // 80Mhz
            divq: Some(PllDiv::Div2), // 200Mhz
            divr: Some(PllDiv::Div3), // 133.33Mhz (for OSPI)
        });
        config.rcc.pll3 = Some(Pll {
            source: PllSource::Hse,
            prediv: PllPreDiv::Div25, // 2Mhz
            mul: PllMul::Mul96,       // 192Mhz
            divp: Some(PllDiv::Div1), // 192Mhz
            divq: Some(PllDiv::Div8), // 24Mhz
            divr: Some(PllDiv::Div4), // 48Mhz
        });
        config.rcc.voltage_scale = VoltageScale::Scale0;
        config.rcc.supply_config = SupplyConfig::DirectSMPS;
        config.rcc.sys = Sysclk::Pll1P; // 550Mhz
        config.rcc.d1c_pre = AHBPrescaler::Div1; // 550Mhz
        config.rcc.ahb_pre = AHBPrescaler::Div2; // 275Mhz
        config.rcc.apb1_pre = APBPrescaler::Div2; // 137.5Mhz
        config.rcc.apb2_pre = APBPrescaler::Div2; // 137.5Mhz
        config.rcc.apb3_pre = APBPrescaler::Div2; // 137.5Mhz
        config.rcc.apb4_pre = APBPrescaler::Div2; // 137.5Mhz

        config.rcc.mux.octospisel = Fmcsel::Pll2R; // 133.33Mhz

        embassy_stm32::init(config)
    }
}
