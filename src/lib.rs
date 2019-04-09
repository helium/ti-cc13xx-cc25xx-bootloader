use byteorder::ByteOrder;
use std::io;
use std::path::Path;
use std::result::Result;
use std::time::Duration;
use std::{thread, time};

extern crate sysfs_gpio;
use sysfs_gpio::{Direction, Pin};

extern crate spidev;
use spidev::{Spidev, SpidevOptions, SpidevTransfer, SPI_MODE_3};

extern crate byteorder;
use byteorder::BigEndian;

extern crate crc;
extern crate ihex;
#[macro_use]
extern crate enum_primitive_derive;
extern crate num_traits;

#[macro_use]
extern crate serde_derive;
extern crate bincode;
extern crate serde;

pub mod bootloader;
pub mod firmware_image;

use bootloader::Bootloader;
use firmware_image::FirmwareImage;

pub struct Cc131x {
    pub io: Spidev,
    pub reset: Pin,
    pub bootloader_en: Pin,
    pub slave_ready: Pin,
    pub slave_tx_req: Pin,
}

#[derive(Debug)]
pub enum Error {
    IO(std::io::Error),
    GPIO(sysfs_gpio::Error),
    BOOTLOADER(bootloader::Error),
    DESER(bincode::Error),
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Error {
        Error::IO(err)
    }
}

impl From<sysfs_gpio::Error> for Error {
    fn from(err: sysfs_gpio::Error) -> Error {
        Error::GPIO(err)
    }
}

impl From<bootloader::Error> for Error {
    fn from(err: bootloader::Error) -> Error {
        Error::BOOTLOADER(err)
    }
}

impl From<bincode::Error> for Error {
    fn from(err: bincode::Error) -> Error {
        Error::DESER(err)
    }
}

const SRAM_START: usize = 0x2000_0000;
// this is where the TI linker puts it, but it gets copied over
const CCFG: usize = 0x1FFA8;
const BL_CONFIG_OFFSET: usize = 12 * 4;
const BL_CONFIG_REG: usize = CCFG | BL_CONFIG_OFFSET;
const BL_EXPECT: u32 = 0xC507_FEC5;

impl Cc131x {
    // causes panic if firmware is invalid
    pub fn assert_if_invalid(firmware: &FirmwareImage) {
        for segment in &firmware.segments {
            let range = (segment.start, segment.start + segment.data.len());
            // find segment with the CCFG
            if BL_CONFIG_REG >= range.0 && BL_CONFIG_REG <= range.1 {
                // split it to the location of interest
                let (_, data) = segment.data.as_slice().split_at(BL_CONFIG_OFFSET);
                let value = BigEndian::read_u32(data);
                // use the format macro so that errors print in hex
                assert_eq!(
                    format!("{:X}", BL_EXPECT),
                    format!("{:X}", value),
                    "BL Config Register has changed!"
                );
            }
        }
    }

    pub fn new<P: AsRef<Path>>(
        path: P,
        reset: u16,
        bootloader_en: u16,
        slave_ready: u16,
        slave_tx_req: u16,
    ) -> Result<Cc131x, Error> {
        // BL_ON is active low for BL, keep as input
        let bootloader_en = Pin::new(bootloader_en.into());

        // TODO: remove this workaround
        // for some reason, setting direction before unexport/export gave
        // " sh: write error: Input/output error " on Hotspot Rev3
        bootloader_en.unexport()?;
        bootloader_en.export()?;

        // reset the CC131x to put it in a known state
        let reset = Pin::new(reset.into());

        let spidev = Cc131x::init(path)?;
        let ret = Cc131x {
            io: spidev,
            reset,
            bootloader_en,
            slave_ready: Pin::new(slave_ready.into()),
            slave_tx_req: Pin::new(slave_tx_req.into()),
        };

        Ok(ret)
    }

    fn reset(&self) -> Result<(), Error> {
        self.reset.set_direction(Direction::Out)?;
        let low_delay = Duration::from_millis(15);
        self.reset.set_value(0)?;
        thread::sleep(low_delay);
        let start_delay = Duration::from_millis(35);
        self.reset.set_value(1)?;
        thread::sleep(start_delay);
        Ok(())
    }

    // a helper for the constructor
    fn init<P: AsRef<Path>>(path: P) -> io::Result<Spidev> {
        let mut spi = Spidev::open(path)?;
        let options = SpidevOptions::new()
            .bits_per_word(8)
            .max_speed_hz(4_000_000)
            // SPI_MODE_3 is picked to match built-in bootloader on CC131x
            .mode(SPI_MODE_3)
            .build();
        spi.configure(&options)?;
        Ok(spi)
    }

    pub fn write_wait_read(&self, input_buf: &[u8], wait: u32) -> io::Result<(Vec<u8>)> {
        let mut rx_buf = vec![0; input_buf.len()];
        {
            let mut transfer = SpidevTransfer::read_write(input_buf, &mut rx_buf);
            self.io.transfer(&mut transfer)?;
        }

        let delay = Duration::new(0, wait);

        thread::sleep(delay);

        let tx_buf = vec![0; 255];
        let mut rx_buf = vec![0; 255];
        {
            let mut transfer = SpidevTransfer::read_write(&tx_buf, &mut rx_buf);
            self.io.transfer(&mut transfer)?;
        }
        Ok(rx_buf)
    }

    pub fn write(&self, input_buf: &[u8]) -> io::Result<(Vec<u8>)> {
        let mut rx_buf = vec![0; input_buf.len()];
        {
            let mut transfer = SpidevTransfer::read_write(input_buf, &mut rx_buf);
            self.io.transfer(&mut transfer)?;
        }
        Ok(rx_buf)
    }

    pub fn read(&self, rec_buf: &mut [u8]) -> io::Result<()> {
        let tx_buf = vec![0; rec_buf.len()];
        {
            let mut transfer = SpidevTransfer::read_write(tx_buf.as_slice(), rec_buf);
            self.io.transfer(&mut transfer)?;
        }
        Ok(())
    }

    pub fn enter_bootloader(&self) -> Result<(), Error> {
        self.bootloader_en
            .set_direction(Direction::Out)
            .expect("Cannot configure bootloader pin as output!");
        self.bootloader_en.set_value(0)?;

        self.reset()?;

        let output = [0x00];
        self.write(&output)?;
        let low_delay = time::Duration::from_millis(20);
        thread::sleep(low_delay);
        self.bootloader_en.set_value(1)?;

        Ok(())
    }

    pub fn flash_firmware(&self, firmware: &FirmwareImage) -> Result<(), Error> {
        self.enter_bootloader()?;
        Bootloader::flash_firmware(&self, firmware, SRAM_START)?;
        Ok(())
    }

    pub fn need_to_update_firmware(&self, firmware: &FirmwareImage) -> Result<bool, Error> {
        self.enter_bootloader().expect("Enter bootloader fail!");
        let firmware_match = Bootloader::firmware_match(&self, firmware, SRAM_START)?;
        if firmware_match {
            return Ok(false);
        }
        Ok(true)
    }
}
