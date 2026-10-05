# Music Player

## Description
Simple, compact music player built using a Raspberry Pi Pico W, programmed in Rust. It will allow users to play, pause, and skip songs stored on an SD card through a speaker, whose volume can be controlled using a potentiometer. The LCD display is meant to show the playing song's title.

### Functionality
The core piece, a **Raspberry Pi Pico 2W**, gets its digital audio play-back from an **SD card** inserted in a module, interfaced via SPI connection. Audio should be output through a speaker, powered by an **amplifier**. The amplifier functions with analog signal, which is why a **Digital-Analog converter** is needed, powered by a 5V **power supply** with a 9V alkaline **battery**. The **LCD** displays via I2C protocol the currently playing song's title. Sound's volume is controlled by a **potentiometer**. Three **buttons** are meant to play, pause and navigate through the songs.
In Rust, the most difficult part 

### Hardware
- Raspberry Pi Pico 2W
- 1602 I2C LCD
- PAM8403D amplifier
- 2 W speaker
- SD card module
- MCP4821-E/P DAC
- Power supply
- 9V alkaline battery

### Software
- Development environment: Visual Studio Code
- probe-rs - embedded toolkit
- crates - embassy-rp. embassy-executor, defmt, cortex-m, ag-lcd, port-expander, embedded-sdmmc, embassy-time

### Design
![kicad.svg](kicad.svg)
