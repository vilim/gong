use std::{
    str::from_utf8,
    sync::{Arc, Mutex},
    thread::sleep,
    time::Duration,
};

use esp_idf_svc::hal::{
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
    hal::units::*,
    netif::{EspNetif, NetifConfiguration},
};

use esp_idf_svc::{
    http::Method::Post,
    wifi::{AuthMethod, ClientConfiguration, Configuration},
};
use log::*;

use ws2812_esp32_rmt_driver::Ws2812Esp32RmtDriver;

use esp_idf_svc::ipv4::{
    ClientConfiguration as IpClientConfiguration, ClientSettings as IpClientSettings,
    Configuration as IpConfiguration, Ipv4Addr, Mask, Subnet,
};

// Set these env variables in a config file not commited to git
// e.g. ~/.cargo/config.toml
const SSID: &str = env!("ESP32_WIFI_SSID");
const PASS: &str = env!("ESP32_WIFI_PWD");
const STATIC_IP: &str = env!("ESP32_STATIC_IP");
const GATEWAY_IP: &str = env!("ESP32_GATEWAY_IP");

const YELLOW: [u8; 3] = [120, 120, 0];
const GREEN: [u8; 3] = [120, 0, 10];

fn main() {
    // It is necessary to call this function once. Otherwise some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_svc::sys::link_patches();

    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();

    // Set up the LED
    let mut ws2812 = Ws2812Esp32RmtDriver::new(0, 8).unwrap();

    // The LED glows yellow while WiFi is connecting
    ws2812.write(&YELLOW).unwrap();

    // Set up WiFi
    let peripherals = Peripherals::take().unwrap();
    let sysloop = EspSystemEventLoop::take().unwrap();
    let timer_service = EspTaskTimerService::new().unwrap();
    let _wifi = wifi(
        peripherals.modem,
        sysloop,
        Some(EspDefaultNvsPartition::take().unwrap()),
        timer_service,
    )
    .unwrap();

    // Then the LED turns green
    ws2812.write(&GREEN).unwrap();

    // Set up the server to recive POST requests
    let mut server = EspHttpServer::new(&Default::default()).unwrap();

    // Set up the servo motor
    // the servo code is adapted from
    // https://github.com/flyaruu/rust-on-esp32/tree/5c66fb73a0369ca8b04d0fa4e7d1af330acefb53
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

            // Parse the request of the form ({angle},{pause},)*{angle}
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

    // Setting up static IP configuration
    // perhaps an easier solution is possible,
    // this seemed to be the simplest one with the high-level driver is possible

    let ipconfig = IpConfiguration::Client(IpClientConfiguration::Fixed(IpClientSettings {
        ip: Ipv4Addr::from(parse_ip(STATIC_IP)),
        subnet: Subnet {
            gateway: Ipv4Addr::from(parse_ip(GATEWAY_IP)),
            mask: Mask(24),
        },
        dns: None,
        secondary_dns: None,
    }));

    let netif_config = NetifConfiguration {
        ip_configuration: ipconfig,
        key: "StaticClient".into(),
        ..NetifConfiguration::wifi_default_client()
    };

    let netif_ap_config = NetifConfiguration {
        key: "StaticAP".into(),
        ..NetifConfiguration::wifi_default_router()
    };

    wifi.wifi_mut()
        .swap_netif(
            EspNetif::new_with_conf(&netif_config).unwrap(),
            EspNetif::new_with_conf(&netif_ap_config).unwrap(),
        )
        .unwrap();

    wifi.start().await?;
    info!("Wifi started");

    wifi.connect().await?;
    info!("Wifi connected");

    wifi.wait_netif_up().await?;
    info!("Wifi netif up");

    Ok(())
}

fn parse_ip(ip: &str) -> [u8; 4] {
    let mut result = [0u8; 4];
    for (idx, octet) in ip.split(".").into_iter().enumerate() {
        result[idx] = u8::from_str_radix(octet, 10).unwrap();
    }
    result
}
