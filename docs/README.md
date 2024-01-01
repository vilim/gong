# Simple ESP32 remote-controled mechanical gong

## Hardware
Just an [ESP32 C6 dev board](https://docs.espressif.com/projects/espressif-esp-dev-kits/en/latest/esp32c6/esp32-c6-devkitc-1/index.html), another one would do as well and a servo motor directly connected to the dev board (as the layout of the GND, 5V and a GPIO pin allows for that). It can be powered by any 5V USB power supply.
The rest is a wooden holder for the gong, a mallet directly attached to the servo (this part will require improvements). The onboard LED just indicates WiFi status (yellow - connecting, green - connected)

## Software
Hacked together from different ESP32 Rust examples, mainly [this one](https://github.com/ivmarkov/rust-esp32-std-demo), the [servo one](https://github.com/flyaruu/rust-on-esp32) and from the the addressable LED (ws2812-esp32-rmt-driver) crate.
After installing the toolchain with `espup`, the environment variables for WiFi have to be set and then uploading should work directly with `cargo run` (when espflash works).