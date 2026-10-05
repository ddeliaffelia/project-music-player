#![no_std]
#![no_main]

use embassy_embedded_hal::shared_bus::blocking::spi::SpiDeviceWithConfig;
use embassy_executor::Spawner;
use embassy_net::StackResources;
use embassy_rp::{adc::{Adc, Channel, Config}, bind_interrupts, gpio::{Input, Level, Output, Pull}, i2c::Blocking as I2cBlocking, peripherals::{I2C1, SPI1}, spi::{self, Spi, Blocking}};
use embassy_sync::blocking_mutex::{raw::NoopRawMutex, Mutex};
use embassy_time::{Delay, Duration, Timer};
use embedded_hal_1::spi::{Operation, SpiDevice};
use embedded_sdmmc::{File, Mode, SdCard, TimeSource, Timestamp, VolumeIdx, VolumeManager};
use irqs::Irqs;
use port_expander::Pcf8574;
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};
use defmt::*;

//I2C
use embassy_rp::i2c::{I2c, InterruptHandler as I2CInterruptHandler, Config as I2cConfig};
use embedded_hal_async::i2c::{Error, I2c as _};
// use embassy_rp::peripherals::I2C0;
use ag_lcd::{Cursor, LcdDisplay};
use itoa;

use embassy_futures::select::{select, Either};

mod irqs;

/// https://github.com/rp-rs/rp-hal-boards/blob/main/boards/rp-pico/examples/pico_spi_sd_card.rs
#[derive(Default)]
pub struct DummyTimesource();

impl TimeSource for DummyTimesource {
    fn get_timestamp(&self) -> Timestamp {
        Timestamp {
            year_since_1970: 0,
            zero_indexed_month: 0,
            zero_indexed_day: 0,
            hours: 0,
            minutes: 0,
            seconds: 0,
        }
    }
}

struct DummyTime;
impl TimeSource for DummyTime {
    fn get_timestamp(&self) -> Timestamp {
        Timestamp { year_since_1970: 54, zero_indexed_month: 0, zero_indexed_day: 1,
                    hours: 0, minutes: 0, seconds: 0 }
    }
}

type SpiBus    = Spi<'static, SPI1, Blocking>;
type SpiDev    = SpiDeviceWithConfig<'static, NoopRawMutex, SpiBus, Output<'static>>;
type SdCardDev = SdCard<SpiDev, Delay>;
type FsMgr     = VolumeManager<SdCardDev, DummyTime>;

static BUS: StaticCell<Mutex<NoopRawMutex, core::cell::RefCell<SpiBus>>> = StaticCell::new();
static FS : StaticCell<FsMgr> = StaticCell::new();

//it should be for the Play/Pause button, but i could not finish the code for it
enum PlayerState {
    Playing,
    Paused,
    Stopped,
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let peripherals = embassy_rp::init(Default::default());
    let mut adc = Adc::new(peripherals.ADC, Irqs, Config::default());

    let mut config = spi::Config::default();
    config.frequency = 1_000_000;
    config.phase = spi::Phase::CaptureOnFirstTransition;
    config.polarity = spi::Polarity::IdleLow;

    //Buttons pins:
    let mut prev_btn = Input::new(peripherals.PIN_8, Pull::Up);
    let mut play_btn = Input::new(peripherals.PIN_7, Pull::Up);
    let mut skip_btn = Input::new(peripherals.PIN_6, Pull::Up);

    //Potentiometer pin:
    let mut volume = Channel::new_pin(peripherals.PIN_26, Pull::Up);
    
    //SD card pins:
    let miso_sd = peripherals.PIN_12;
    let mosi_sd = peripherals.PIN_11;
    let clk_sd = peripherals.PIN_10;
    let mut cs_sd = Output::new(peripherals.PIN_13, Level::High);
    
    let mut spi_sd = Spi::new_blocking(peripherals.SPI1, clk_sd, mosi_sd, miso_sd, config.clone());
    let spi_bus = BUS.init(Mutex::new(core::cell::RefCell::new(spi_sd)));
    let spi_sd_dev = SpiDeviceWithConfig::new(spi_bus, cs_sd, config.clone());
    
    //DAC pins:
    let sdi_dac = peripherals.PIN_19;
    let clk_dac = peripherals.PIN_18;
    let mut cs_dac = Output::new(peripherals.PIN_17, Level::High);
    // let mut spi_amp = Spi::new(peripherals.SPI0, clk_dac, sdi_dac, peripherals.DMA_CH0, peripherals.DMA_CH1, config);

    //LCD 1602 pins:
    let sda = peripherals.PIN_2;
    let scl = peripherals.PIN_3;

    //LCD - working:
    let i2c: I2c<'_, I2C1, I2cBlocking> = I2c::new_blocking(peripherals.I2C1, scl, sda, I2cConfig::default());
    let mut expander = Pcf8574::new(i2c, true, true, true);
    let mut lcd = LcdDisplay::new_pcf8574(&mut expander, Delay) 
        .with_cursor(Cursor::Off)
        .with_reliable_init(10_000)
        .build();

    //SD - file reading setup:
    let delay_sd = Delay;
    let sdcard = SdCard::new(spi_sd_dev, delay_sd);
    let fs = FS.init(VolumeManager::new(sdcard, DummyTime));

    match fs.device().num_bytes() {
        Ok(sz) => info!("Card OK {} bytes", sz),
        Err(_e) => (),
    }

    fs.device().spi(|s| {
        let mut fast = spi::Config::default();
        fast.frequency = 8_000_000;
        fast.phase     = spi::Phase::CaptureOnFirstTransition;
        fast.polarity  = spi::Polarity::IdleLow;
        s.set_config(fast);

        let tx = [0xFF; 10];
        let mut rx = [0u8; 10];
        let mut ops = [Operation::Transfer(&mut rx, &tx)];
        if s.transaction(&mut ops).is_ok() {
            info!("Dummy transaction OK");
        } else {
            error!("Dummy transaction failed !!!!!");
        }
    });

    //DAC setup:
    let mut spi_dac = Spi::new_blocking(
        peripherals.SPI0, 
        clk_dac,
        sdi_dac,
        peripherals.PIN_16, //dummy
        config.clone(),
    );

    cs_dac.set_high();
    Timer::after_millis(10).await;

    //Unfinished audio, so the setup is useless :<


    //Files:
    let mut vol  = fs.open_volume(VolumeIdx(0)).unwrap();
    let mut root = vol.open_root_dir().unwrap();

    let mut file_array: [Option<File<'_, SdCardDev, DummyTime, 4, 4, 1>>; 10] = [const { None }; 10];
    let song_names = ["song1.wav", "song2.wav", "song3.wav", "song4.wav"];
    
    if let Some(file) = &mut file_array[0] {
        let mut buf = [0u8; 64];
        let n = file.read(&mut buf).unwrap();
        info!("Read, {}", n);
    }

    if let Some(file) = file_array[0].take(){
        file.close().unwrap();
        info!("Read and done!");
    }

    let mut player_state = PlayerState::Stopped;
    let mut index = 0; //index for going through songs
    loop {
        match select(
        prev_btn.wait_for_low(),
        select(
            skip_btn.wait_for_low(),
            select(
                play_btn.wait_for_low(),
                Timer::after_secs(5)
            )
        )
        ).await {
            Either::First(_) => { //previous btn
                info!("go back");
                if index != 0 { 
                    index -= 1;
                }

                root.open_file_in_dir(song_names[index], Mode::ReadOnly).unwrap();

                lcd.clear();
                lcd.print("Song ");
                let mut buf = itoa::Buffer::new();
                lcd.print(buf.format(index + 1));

                prev_btn.wait_for_rising_edge().await;
            },
            
            Either::Second(Either::First(_)) => { //skip btn
                info!("skip");
                if index != 3 { 
                    index += 1;
                }

                root.open_file_in_dir(song_names[index], Mode::ReadOnly).unwrap();
                
                lcd.clear();
                lcd.print("Song ");
                let mut buf = itoa::Buffer::new();
                lcd.print(buf.format(index + 1));
                
                skip_btn.wait_for_rising_edge().await;
            },

            Either::Second(Either::Second(Either::First(_))) => { //play/pause btn
                info!("pause/play");
                match player_state {
                    PlayerState::Playing => {
                        player_state = PlayerState::Paused;
                        lcd.clear();
                        lcd.print("Paused");
                    },
                    PlayerState::Paused | PlayerState::Stopped => {
                        player_state = PlayerState::Playing;
                        lcd.clear();
                        lcd.print("Playing ");
                        let mut buf = itoa::Buffer::new();
                        lcd.print(buf.format(index + 1));
                    }
                }
                play_btn.wait_for_rising_edge().await;
            },

            Either::Second(Either::Second(Either::Second(_))) => { // vol check (every 5 secs?)
                let level = adc.read(&mut volume).await.unwrap();
                println!("Volume level: {}", level);
            }
        }
        Timer::after_secs(1).await;
    }
}

//help for fixing buttons:
//https://stackoverflow.com/questions/69173586/either-type-a-or-b-in-rust

//OLD LOOP CODE:

        // root.open_file_in_dir(song_names[index], Mode::ReadOnly).unwrap();
            
        //     lcd.clear();
        //     lcd.print("Song ");
        //     let mut buf = itoa::Buffer::new();
        //     lcd.print(buf.format(index + 1));

        // //Buttons:
        // if prev_btn.is_low(){
        //     println!("Previous song plays");
        //     if index != 0 {
        //         index -= 1;
        //     }
        //     root.open_file_in_dir(song_names[index], Mode::ReadOnly).unwrap();
        //     //update display:
        //     lcd.clear();
        //     lcd.print("Song ");
        //     let mut buf = itoa::Buffer::new();
        //     lcd.print(buf.format(index + 1));    

        //     prev_btn.wait_for_rising_edge().await;
        // }
        // if play_btn.is_low(){
        //     println!("o~o ?");
        //     play_btn.wait_for_rising_edge().await;
        // }
        // if skip_btn.is_low(){
        //     println!("Next song plays");
        //     if index != 3 {
        //         index += 1;
        //     }
        //     root.open_file_in_dir(song_names[index], Mode::ReadOnly).unwrap();
        //     //update display:
        //     lcd.clear();
        //     lcd.print("Song ");
        //     let mut buf = itoa::Buffer::new();
        //     lcd.print(buf.format(index + 1));

        //     skip_btn.wait_for_rising_edge().await;
        // }

        // //Potentiometer:
        // static mut LAST_VOLUME_CHECK: u64 = 0;
        // let now = embassy_time::Instant::now().as_secs();
        // unsafe {
        //     if now - LAST_VOLUME_CHECK >= 5 {
        //         let level = adc.read(&mut volume).await.unwrap();
        //         println!("Volume level: {}", level);
        //         LAST_VOLUME_CHECK = now;
        //     }
        // } 
