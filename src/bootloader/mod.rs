mod commands;
use bootloader::commands::Error as BlPkError;
use bootloader::commands::*;

use firmware_image::Segment;
use std::io;
use std::{thread, time};

use Cc131x;
pub struct Bootloader;

/*
 *  The responsbility of this library is to exercise the commands module and provide a high level bootloader interface
 *  It handles delays required between commands on a more or less case-by-case basis.
 *  All the timings were empirically determined at 4Mhz
 */

#[derive(Debug)]
pub enum Error {
    IO(io::Error),
    BOOTLOADER(BlPkError),
}

impl From<BlPkError> for Error {
    fn from(err: BlPkError) -> Error {
        Error::BOOTLOADER(err)
    }
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Error {
        Error::IO(err)
    }
}

impl Bootloader {
    fn ack(io: &Cc131x) -> Result<(), Error> {
        let packet = [0xCC];
        io.write(&packet)?;
        Ok(())
    }

    fn get_status(io: &Cc131x) -> Result<StatusValue, Error> {
        let packet = GetStatus::new().serialize()?;
        let resp = io.write(&packet)?;
        let status = CommandStatus::from_payload(resp)?;
        Self::ack(&io)?;
        Ok(status.value)
    }

    pub fn initialize(io: &Cc131x) -> Result<(), Error> {
        const CC1310_CHIP_ID: u32 = 0x2002_8000;

        let packet = Ping::new().serialize()?;
        let resp = io.write(&packet)?;
        check_ack(resp)?;

        let packet = GetChipId::new().serialize()?;
        let response = io.write(&packet)?;
        let chip_id = ChipId::from_payload(response)?;
        Bootloader::ack(io)?;
        assert_eq!(chip_id.value, CC1310_CHIP_ID);
        Ok(())
    }

    pub fn erase_sector(io: &Cc131x, sector: u32) -> Result<(), Error> {
        let packet = SectorErase::new(sector).serialize()?;
        io.write(&packet)?;

        let delay = time::Duration::from_millis(10);
        thread::sleep(delay);
        let mut response = vec![0; 28];
        io.read(&mut response.as_mut_slice())?;
        check_ack(response)?;

        let status = Self::get_status(&io)?;
        assert_eq!(status, StatusValue::Success, "Failed to Erase Sector");
        Ok(())
    }

    pub fn erase_chip(io: &Cc131x) -> Result<(), Error> {
        let packet = BankErase::new().serialize()?;
        io.write(&packet)?;

        let delay = time::Duration::from_millis(25);
        thread::sleep(delay);
        let mut response = vec![0; 28];
        io.read(&mut response.as_mut_slice())?;
        check_ack(response)?;

        let status = Self::get_status(&io)?;
        assert_eq!(status, StatusValue::Success, "Failed to Erase Sector");
        Ok(())
    }

    fn write_payload(io: &Cc131x, payload: Vec<u8>) -> Result<(), Error> {
        let len = payload.len() as u32;
        let packet = SendData::new(payload).serialize()?;
        io.write(&packet)?;

        let delay = time::Duration::new(0, len * 6500);

        thread::sleep(delay);

        let mut response = vec![0; 32];
        io.read(&mut response.as_mut_slice())?;
        check_ack(response)?;
        Ok(())
    }

    pub fn get_crc(io: &Cc131x, addr: u32, size: u32) -> Result<u32, Error> {
        let packet = Crc32::new(addr, size, 0).serialize().unwrap();
        io.write(&packet).unwrap();

        let delay = time::Duration::new(0, size * 500);
        thread::sleep(delay);

        let mut response = vec![0; 16];
        io.read(&mut response.as_mut_slice())?;
        let crc32_checksum = Crc32Response::from_payload(response).unwrap();
        Bootloader::ack(io)?;
        Ok(crc32_checksum.value)
    }

    pub fn system_reset(io: &Cc131x) -> Result<(), Error> {
        let packet = Reset::new().serialize().unwrap();
        let response = io.write(&packet).unwrap();
        check_ack(response)?;
        let delay = time::Duration::from_millis(20);
        thread::sleep(delay);
        Ok(())
    }

    pub fn write_segment(io: &Cc131x, segment: &Segment) -> Result<(), Error> {
        const MAX_PAYLOAD: usize = 252;

        #[derive(Debug)]
        struct S {
            address: u32,
            size: u32,
        }
        let s = S {
            address: segment.start as u32,
            size: segment.data.len() as u32,
        };
        // prepare chip for download of segment
        let start_segment_download = Download::new(s.address, s.size).serialize()?;
        let resp = io.write(&start_segment_download)?;
        check_ack(resp)?;

        let mut data = segment.data.clone();
        // send the whole segment chunk by chunk
        loop {
            let len = data.len();
            if len <= MAX_PAYLOAD {
                break;
            }
            let mut payload = data;
            data = payload.split_off(MAX_PAYLOAD);
            Self::write_payload(io, payload)?;
        }
        Self::write_payload(io, data)?;

        let status = Self::get_status(&io)?;
        assert_eq!(status, StatusValue::Success, "Failed to Send Data");

        let crc_read = Self::get_crc(&io, s.address, s.size)?;
        assert_eq!(segment.crc, crc_read);

        let status = Self::get_status(&io)?;
        assert_eq!(status, StatusValue::Success, "Failed to Read CRC");

        Ok(())
    }

    pub fn flash_firmware(io: &Cc131x, firmware: &FirmwareImage, sram: usize) -> Result<(), Error> {
        Bootloader::initialize(&io)?;
        Bootloader::erase_chip(&io)?;
        for segment in &firmware.segments {
            // throw away hex segments writing to SRAM
            if (segment.start & sram) == 0 {
                Bootloader::write_segment(&io, segment)?;
            }
        }
        Bootloader::system_reset(&io)?;
        Ok(())
    }

    pub fn firmware_match(
        io: &Cc131x,
        firmware: &FirmwareImage,
        sram: usize,
    ) -> Result<bool, Error> {
        Bootloader::initialize(&io)?;
        for segment in &firmware.segments {
            // throw away hex segments writing to SRAM
            if (segment.start & sram) == 0 {
                let crc =
                    Bootloader::get_crc(&io, segment.start as u32, segment.data.len() as u32)?;
                if crc != segment.crc {
                    Bootloader::system_reset(&io)?;

                    return Ok(false);
                }
            }
        }
        Bootloader::system_reset(&io)?;
        Ok(true)
    }
}

#[test]
fn test_enter_bootloader_and_get_ack() {
    // instantiate Lms6002 device with the mock registers rather than Spidev
    // P9_15 <=> GPIO 48, P9_23 <=> GPIO 49
    let io = Cc131x::new("/dev/spidev1.0", 60, 115, 49, 48).unwrap();
    io.enter_bootloader().unwrap();

    //Bootloader::poll_until_ready(&io);
    let packet = Ping::new().serialize().unwrap();
    let resp = io.write(&packet).unwrap();
    check_ack(resp).unwrap();
}

//#[cfg(test)]
use firmware_image::FirmwareImage;
#[test]
fn test_write_memory_location() {
    let io = Cc131x::new("/dev/spidev1.0", 60, 115, 49, 48).unwrap();
    io.enter_bootloader().unwrap();

    Bootloader::initialize(&io).unwrap();
    Bootloader::erase_sector(&io, 0).unwrap();

    const FW_FILE: &'static str = include_str!("../../src/firmware/test_parsing.ihex");
    let mut firmware = FirmwareImage::new(FW_FILE);
    if let Some(segment) = firmware.segments.pop() {
        Bootloader::write_segment(&io, &segment).unwrap();
    }
}

#[test]
fn test_write_whole_memory() {
    let io = Cc131x::new("/dev/spidev1.0", 60, 115, 49, 48).unwrap();
    io.enter_bootloader().unwrap();
    const FW_SERIALIZED: &'static [u8] = include_bytes!("../firmware/firmware.bincode");
    let firmware = FirmwareImage::deserialize(FW_SERIALIZED).unwrap();
    const SRAM_START: usize = 0x20000000;

    Bootloader::flash_firmware(&io, &firmware, SRAM_START).unwrap();
}

#[test]
fn test_verify_whole_memory() {
    let io = Cc131x::new("/dev/spidev1.0", 60, 115, 49, 48).unwrap();
    io.enter_bootloader().unwrap();
    const FW_SERIALIZED: &'static [u8] = include_bytes!("../firmware/firmware.bincode");
    let firmware = FirmwareImage::deserialize(FW_SERIALIZED).unwrap();
    const SRAM_START: usize = 0x20000000;
    let firmware_match = Bootloader::firmware_match(&io, &firmware, SRAM_START).unwrap();
    if !firmware_match {
        assert!(false, "Firmware mismatch");
    }
}
