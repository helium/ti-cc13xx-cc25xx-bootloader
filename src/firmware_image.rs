use std::fs::File;
use std::io::Error as ioError;
use std::io::Read;
use std::path::Path;

use bincode::{deserialize, serialize, ErrorKind};
use crc::crc32;
use ihex::reader::ReaderError;
use ihex::record::Record;
use std::iter::Iterator;

#[derive(Debug)]
pub enum Error {
    IO(ioError),
    EndOfFileInMiddleOfFile,
}

impl From<ioError> for Error {
    fn from(err: ioError) -> Error {
        Error::IO(err)
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Segment {
    pub start: usize,
    pub data: Vec<u8>,
    pub crc: u32,
}

impl Segment {
    fn new(start: usize, init_data: &mut Vec<u8>) -> Segment {
        let mut data = Vec::new();
        data.append(init_data);

        Segment {
            start,
            data,
            crc: 0,
        }
    }
}
#[derive(Serialize, Deserialize, Debug)]
pub struct FirmwareImage {
    pub segments: Vec<Segment>,
}

impl FirmwareImage {
    pub fn from_records(mut records: Vec<Record>) -> Result<FirmwareImage, Error> {
        let mut segments = Vec::new();
        let mut ext_addr: usize = 0;
        let mut current_segment = Segment {
            start: 0x00,
            data: Vec::new(),
            crc: 0,
        };
        let mut hit_eof = false;
        loop {
            match records.pop().unwrap() {
                Record::Data { offset, mut value } => {
                    if hit_eof {
                        return Err(Error::EndOfFileInMiddleOfFile);
                    }
                    let new_loc = offset as usize | ext_addr;
                    if current_segment.start + current_segment.data.len() != new_loc {
                        let crc_calc = crc32::checksum_ieee(&current_segment.data);
                        current_segment.crc = crc_calc;
                        segments.push(current_segment);
                        current_segment = Segment::new(new_loc, &mut value);
                    } else {
                        current_segment.data.append(&mut value);
                    }
                }
                Record::ExtendedSegmentAddress(val) => ext_addr = (val as usize) << 4,
                Record::ExtendedLinearAddress(val) => ext_addr = (val as usize) << 16,
                Record::EndOfFile => {
                    if hit_eof {
                        segments.push(current_segment);
                        break;
                    } else {
                        hit_eof = true;
                    }
                }
                Record::StartSegmentAddress { .. } => {}
                _ => assert!(false, "Unhandled iHex record type!"),
            }
        }
        segments.reverse();
        Ok(FirmwareImage { segments })
    }

    pub fn from_path(path: &Path) -> Result<FirmwareImage, Error> {
        let mut file = File::open(path).expect("Firmware path invalid");
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        Self::new(&contents)
    }

    pub fn new(file: &str) -> Result<FirmwareImage, Error> {
        let split = file.split("\r\n").map(|line| {
            let record_result = Record::from_record_string(line);
            match record_result {
                Ok(record) => record,
                Err(e) => {
                    match e {
                        // this allows us to handle untreated hex output from compilation
                        // as last line has \r\n folowed by no start code
                        // integrity check in from_records verifies multiple EOF only exist at EOF
                        ReaderError::MissingStartCode => Record::EndOfFile,
                        _ => {
                            panic!("RecordReader Error: {:}", e);
                        }
                    }
                }
            }
        });
        let mut records: Vec<Record> = split.collect();
        records.reverse();
        FirmwareImage::from_records(records)
    }

    pub fn serialize(self) -> Result<Vec<u8>, Box<ErrorKind>> {
        serialize(&self)
    }

    pub fn deserialize(encoded: &[u8]) -> Result<FirmwareImage, Box<ErrorKind>> {
        deserialize(encoded)
    }
}

#[test]
fn test_read_record_from_hex() {
    const FW_FILE: &'static str = include_str!("firmware/test_parsing.ihex");
    let mut firmware = FirmwareImage::new(FW_FILE).unwrap();

    if let Some(current_segment) = firmware.segments.pop() {
        // check the first segment
        assert_eq!(current_segment.start, 0);
        assert_eq!(current_segment.data.len(), 60);
    }
}

#[test]
fn test_serialize_deserialize() {
    const FW_FILE: &'static str = include_str!("firmware/test_parsing.ihex");
    let firmware = FirmwareImage::new(FW_FILE).unwrap();

    let mut encoded = firmware.serialize().unwrap();
    let mut decoded = FirmwareImage::deserialize(&encoded.as_mut_slice()).unwrap();

    if let Some(current_segment) = decoded.segments.pop() {
        assert_eq!(current_segment.start, 0);
        assert_eq!(current_segment.data.len(), 60);
    }
}

#[test]
fn test_deserialize_from_include() {
    const FW_SERIALIZED: &'static [u8] = include_bytes!("firmware/firmware.bincode");
    let mut decoded = FirmwareImage::deserialize(&FW_SERIALIZED).unwrap();

    if let Some(current_segment) = decoded.segments.pop() {
        assert_eq!(current_segment.start, 0);
        assert_eq!(current_segment.data.len(), 60);
    }
}
