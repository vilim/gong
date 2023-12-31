use std::{
    str::from_utf8,
    sync::{Arc, Mutex},
    thread::sleep,
    time::Duration,
};

use esp_idf_svc::hal::units::*;
use esp_idf_svc::hal::{
    gpio::PinDriver,
    ledc::{config::TimerConfig, LedcDriver, LedcTimerDriver},
    peripheral::Peripheral,
    prelude::Peripherals,
};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    http::server::EspHttpServer,
    nvs::{EspDefaultNvsPartition, EspNvsPartition, NvsDefault},
    ping::EspPing,
    timer::{EspTaskTimerService, EspTimerService, Task},
    wifi::{AsyncWifi, EspWifi},
};

use esp_idf_svc::{
    http::Method::Post,
    io::Read,
    wifi::{AuthMethod, ClientConfiguration, Configuration},
};
use log::*;

use ws2812_esp32_rmt_driver::Ws2812Esp32RmtDriver;
use ws2812_esp32_rmt_driver::RGB8;

const SSID: &str = env!("ESP32_WIFI_SSID");
const PASS: &str = env!("ESP32_WIFI_PWD");

fn main() {
    // It is necessary to call this function once. Otherwise some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_svc::sys::link_patches();

    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();

    let peripherals = Peripherals::take().unwrap();
    let sysloop = EspSystemEventLoop::take().unwrap();
    let timer_service = EspTaskTimerService::new().unwrap();
    const yellow: [u8; 3] = [120,120,0];
    const green: [u8; 3] = [120,0,50];
    const blue: [u8; 3] = [120,0,255];
    let mut ws2812 = Ws2812Esp32RmtDriver::new(0, 8).unwrap();
    ws2812.write(&yellow).unwrap();
    let _wifi = wifi(
        peripherals.modem,
        sysloop,
        Some(EspDefaultNvsPartition::take().unwrap()),
        timer_service,
    )
    .unwrap();
    ws2812.write(&green).unwrap();

    let mut server = EspHttpServer::new(&Default::default()).unwrap();

    let servo_timer = peripherals.ledc.timer1;
    let servo_driver = LedcTimerDriver::new(
        servo_timer,
        &TimerConfig::new()
            .frequency(50.Hz())
            .resolution(esp_idf_svc::hal::ledc::Resolution::Bits14),
    )
    .unwrap();
    let servo = Arc::new(Mutex::new(
        LedcDriver::new(
            peripherals.ledc.channel3,
            servo_driver,
            peripherals.pins.gpio3,
        )
        .unwrap(),
    ));

    let max_duty = servo.lock().unwrap().get_max_duty();

    let min = max_duty / 40;
    let max = max_duty / 8;

    fn interpolate(angle: u32, min: u32, max: u32) -> u32 {
        angle * (max - min) / 180 + min
    }

    server
        .fn_handler("/servo", Post, move |mut req| {
            let mut buffer = [0_u8; 1024];
            let bytes_read = req.read(&mut buffer).unwrap();
            let angle_string = from_utf8(&buffer[0..bytes_read]).unwrap();
            let times_angles: Vec<u32> = angle_string
                .split(",")
                .map(|s| s.parse::<u32>().unwrap())
                .collect();
            servo
                .lock()
                .unwrap()
                .set_duty(interpolate(times_angles[0] as u32, min, max))
                .unwrap();
            info!("Set servo to {}", times_angles[0]);
            for i in 0..(times_angles.len() - 1) / 2 {
                let wait_time = times_angles[i * 2 + 1];
                info!("Wait {}", wait_time);
                sleep(Duration::from_millis(wait_time as u64));
                let servo_angle = times_angles[i * 2 + 2];
                info!("Set servo to {}", servo_angle);
                servo
                    .lock()
                    .unwrap()
                    .set_duty(interpolate(servo_angle as u32, min, max))
                    .unwrap();
            }
            Ok(())
        })
        .unwrap();

    loop {
        sleep(Duration::from_secs(1));
    }
}

pub fn wifi(
    modem: impl Peripheral<P = esp_idf_svc::hal::modem::Modem> + 'static,
    sysloop: EspSystemEventLoop,
    nvs: Option<EspNvsPartition<NvsDefault>>,
    timer_service: EspTimerService<Task>,
) -> anyhow::Result<AsyncWifi<EspWifi<'static>>> {
    use futures::executor::block_on;
    let mut wifi = AsyncWifi::wrap(
        EspWifi::new(modem, sysloop.clone(), nvs)?,
        sysloop,
        timer_service.clone(),
    )?;

    block_on(connect_wifi(&mut wifi))?;

    let ip_info = wifi.wifi().sta_netif().get_ip_info()?;

    println!("Wifi DHCP info: {:?}", ip_info);

    EspPing::default().ping(
        ip_info.subnet.gateway,
        &esp_idf_svc::ping::Configuration::default(),
    )?;
    Ok(wifi)
}

async fn connect_wifi(wifi: &mut AsyncWifi<EspWifi<'static>>) -> anyhow::Result<()> {
    let wifi_configuration: Configuration = Configuration::Client(ClientConfiguration {
        ssid: SSID.into(),
        bssid: None,
        auth_method: AuthMethod::WPA2Personal,
        password: PASS.into(),
        channel: None,
    });

    wifi.set_configuration(&wifi_configuration)?;

    wifi.start().await?;
    info!("Wifi started");

    wifi.connect().await?;
    info!("Wifi connected");

    wifi.wait_netif_up().await?;
    info!("Wifi netif up");

    Ok(())
}
