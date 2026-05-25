#![no_std]
#![no_main]

use defmt::{error, info};
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::{
    clock::CpuClock,
};
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::{interrupt::software::SoftwareInterruptControl, timer::timg::TimerGroup};

esp_bootloader_esp_idf::esp_app_desc!();
#[panic_handler]
fn panic(panic_info: &core::panic::PanicInfo) -> ! {
    error!("{}", panic_info);
    loop {}
}

#[esp_rtos::main]
async fn main(_spawner: Spawner) {
    rtt_target::rtt_init_defmt!();
    defmt::println!("Init!");

    let peripherals = esp_hal::init(esp_hal::Config::default()
        .with_cpu_clock(CpuClock::max())
    );

    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    //let timer0 = esp_hal::timer::systimer::SystemTimer::new(peripherals.SYSTIMER);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);

    let mut led = Output::new(peripherals.GPIO0, Level::Low, OutputConfig::default());

    let mut counter = 0;
    loop {
        if counter == 10 {
            panic!("test panic");
        }
        defmt::println!("test println");
        led.toggle();
        info!("test info");

        Timer::after(Duration::from_millis(250)).await;

        //block_for(Duration::from_millis(250));
        error!("test error");
        counter += 1;
    }
}