use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use std::io::Error as IoError;
use std::io::{Cursor, Read};

/*
 *  This module translates each TI boolotader commands into a type, allowing for serialize/deserialize
 *  It's my personal experiment in macros
 */

pub trait CommandDef: Sized {
    const BASE_PACKET_SIZE: u8 = 3;
    const CMD: u8;
    const MIN_LEN: u8;
    const MAX_LEN: u8;
    // NULL_BYTES are clocked out so as to receive the ACK (and response payload as well)
    // note: some commands require a delay, so NULL_BYTES may be 0 and instead the parent bootloader module handles the delay
    const NULL_BYTES: usize;
    fn into_payload(self) -> Result<Option<Vec<u8>>, Error>;
}

#[derive(Debug)]
pub enum Error {
    MaxPayloadExceeded,
    MinPayloadNotMet,
    IO(IoError),
    NoAck,
    Nack,
    BadChecksum,
    BadCmdByte,
    PacketTooShort,
    InvalidCmdStatus,
    InvalidStatusCode,
}

impl From<IoError> for Error {
    fn from(err: IoError) -> Error {
        Error::IO(err)
    }
}

pub fn check_ack(from_bus: Vec<u8>) -> Result<Cursor<Vec<u8>>, Error> {
    const ACK_BYTE: u8 = 0xCC;
    const NACK_BYTE: u8 = 0x33;
    // search for checksum
    let mut rdr = Cursor::new(from_bus);
    loop {
        if let Ok(cur) = rdr.read_u8() {
            if cur == ACK_BYTE {
                break;
            } else if cur == NACK_BYTE {
                return Err(Error::Nack);
            }
        }
        // if we did not read a value, we got to end with NoAck
        else {
            return Err(Error::NoAck);
        }
    }
    Ok(rdr)
}

pub trait Command: CommandDef {
    const PACKET_SIZE_INDEX: usize = 0;
    const CHECKSUM_INDEX: usize = 1;

    fn serialize(self) -> Result<Vec<u8>, Error> {
        // serializes everything after the CMD byte
        let payload = self.into_payload()?;

        // calculate the checksum over CMD and payload
        let mut checksum = Self::CMD;
        let mut size = Self::BASE_PACKET_SIZE;
        if let Some(ref payload) = payload {
            for i in payload {
                checksum = ((checksum as usize) + (*i as usize)) as u8;
            }
            size += payload.len() as u8;
        }

        // create the packet with header
        // byte[0] = packet size
        // byte[1] = packet checksum
        // byte[2] = cmd
        // byte[3..N] = Option<payload>
        let mut output = vec![size, checksum, Self::CMD];
        if let Some(mut payload) = payload {
            output.append(&mut payload);
        }

        output.resize(size as usize + Self::NULL_BYTES, 0);
        Ok(output)
    }

    fn read_header(from_bus: Vec<u8>) -> Result<(Vec<u8>), Error> {
        // create the packet with header
        // byte[0] = packet size
        // byte[1] = packet checksum
        // byte[2..N] = Option<payload>
        // NOTE: no command byte

        // helper verifies ACK byte and returns reader after it
        let mut rdr = check_ack(from_bus)?;

        // first byte is packet size
        let length = rdr.read_u8()? as usize;
        // second byte is checksum
        let checksum = rdr.read_u8()?;

        if length < (Self::MIN_LEN as usize - 1) {
            return Err(Error::MinPayloadNotMet);
        } else if length > (Self::MAX_LEN as usize - 1) {
            return Err(Error::MaxPayloadExceeded);
        }

        const BYTES_NIBBLED: usize = 2;
        let mut payload = vec![0; length - BYTES_NIBBLED];
        let bytes_read = rdr.read(payload.as_mut_slice())?;
        if bytes_read != length - BYTES_NIBBLED {
            return Err(Error::PacketTooShort);
        }

        // initialize checksum calculation with CMD byte
        let mut checksum_calc = 0;
        for i in payload.as_slice() {
            checksum_calc = ((checksum_calc as usize) + (*i as usize)) as u8;
        }
        if checksum_calc != checksum {
            return Err(Error::BadChecksum);
        }
        Ok(payload)
    }
}

use num_traits::FromPrimitive;
#[derive(Primitive, Debug, PartialEq)]
pub enum StatusValue {
    Default = 0,
    Success = 0x40,
    UnknownCmd = 0x41,
    InvalidCmd = 0x42,
    InvalidAddr = 0x43,
    FlashFail = 0x44,
}

impl Default for StatusValue {
    fn default() -> StatusValue {
        StatusValue::Default
    }
}

enum CommandFields {
    Sma(u8),
    Med(u16),
    Big(u32),
    Vector(Vec<u8>),
    StatusValue(StatusValue),
}

macro_rules! commandFieldFrom {
    ($id:ident, $type:ty) => {
        impl From<$type> for CommandFields {
            fn from(thing: $type) -> CommandFields {
                CommandFields::$id(thing)
            }
        }
        impl From<CommandFields> for $type {
            fn from(field: CommandFields) -> $type {
                if let CommandFields::$id(v) = field {
                    v
                } else {
                    panic!("Failed at unwrapping CommandField enum");
                }
            }
        }
    };
}

commandFieldFrom!(Sma, u8);
commandFieldFrom!(Med, u16);
commandFieldFrom!(Big, u32);
commandFieldFrom!(Vector, Vec<u8>);
commandFieldFrom!(StatusValue, StatusValue);

fn serializer(vec: &mut Vec<u8>, input: &CommandFields) -> Result<(), Error> {
    use self::CommandFields::*;
    match *input {
        Sma(u) => vec.push(u),
        Med(u) => vec.write_u16::<BigEndian>(u)?,
        Big(u) => vec.write_u32::<BigEndian>(u)?,
        Vector(ref v) => {
            let mut vec_clone = v.clone();
            vec.append(&mut vec_clone);
        }
        _ => assert!(false, "Status is never seralized"),
    }
    Ok(())
}

fn deserializer(
    rdr: &mut Cursor<Vec<u8>>,
    input: &mut CommandFields,
    count: usize,
) -> Result<(), Error> {
    use self::CommandFields::*;
    use self::StatusValue;

    match *input {
        Sma(ref mut u) => *u = rdr.read_u8()?,
        Med(ref mut u) => *u = rdr.read_u16::<BigEndian>()?,
        Big(ref mut u) => *u = rdr.read_u32::<BigEndian>()?,
        Vector(ref mut v) => {
            v.resize(count, 0);
            rdr.read_exact(&mut v.as_mut_slice())?;
        }
        StatusValue(ref mut s) => {
            let status_byte = rdr.read_u8()?;
            let status_value = StatusValue::from_u8(status_byte);
            match status_value {
                Some(v) => *s = v,
                None => return Err(Error::InvalidStatusCode),
            }
            //*s = status_value.into() ;
        }
    }
    Ok(())
}

macro_rules! command {
    ($i:ident, $e:expr, $null:expr, $min:expr, $max:expr; $($arg_name:ident, $arg_type:ty),*) => {
        pub struct $i {
            $(pub $arg_name: $arg_type),*
        }

        impl CommandDef for $i {
            const CMD: u8 = $e;
            const NULL_BYTES: usize = $null;
            const MIN_LEN: u8 = $min;
            const MAX_LEN: u8 = $max;
            fn into_payload(self) -> Result<Option<Vec<u8>>, Error> {
                // macros are kind of dumb
                #[allow(unused_mut)]
                let mut payload: Vec<u8> = Vec::new();
                $(serializer(&mut payload, &self.$arg_name.into())?;)*
                let len = payload.len();
                if len + 3 < (Self::MIN_LEN as usize) {
                    return Err(Error::MinPayloadNotMet);
                }
                else if len + 3 > Self::MAX_LEN as usize {
                    return Err(Error::MaxPayloadExceeded);
                }
                if len == 0 {
                    return Ok(None);
                }
                else{
                    Ok(Some(payload))
                }
            }
        }
        impl Command for $i {}
        impl $i {
            #[allow(dead_code)] // macros like to complain about unused code that is used
            pub fn new($($arg_name: $arg_type),*) -> $i {
                $i { $($arg_name),*}
            }
            #[allow(dead_code)]
            #[allow(unused_mut)]
            pub fn from_payload(mut from_bus: Vec<u8>) -> Result<$i, Error> {
                let payload = Self::read_header(from_bus)?;
                $(let mut $arg_name: $arg_type = Default::default();)*
                #[allow(unused_variables)] // macros like to complain about unused code that is used
                let len = payload.len();
                #[allow(unused_variables)] // macros like to complain about unused code that is used
                let mut rdr = Cursor::new(payload);
                $(
                    let pos = rdr.position() as usize;
                    let mut tmp = $arg_name.into();
                    deserializer(&mut rdr, &mut tmp, len - pos)?;
                    $arg_name = tmp.into();
                )*
                return Ok(
                        $i {
                    $($arg_name),*
                            }
                )
            }
        }
    };
    ($i:ident, $e:expr, $null:expr, $fix:expr; $($arg_name:ident, $arg_type:ty),*) =>{
        command![$i, $e, $null, $fix, $fix; $($arg_name, $arg_type),*];
    };
    ($i:ident, $e:expr, $null:expr) => {
        command![$i, $e, $null, 3, 3; ];
    };
}

command!(Ping, 0x20, 36);
command!(
    Download,
    0x21,       // command byte
    24,          // num null bytes
    11;         // fixed payload size
    address,   // serializer arg 1
    u32,        // serializer type arg1
    size,     // serializer arg 2
    u32         // serializer type arg2
);
command!(GetStatus, 0x23, 32);
command!(
    SendData,
    0x24,   // command byte
    0,     // num null bytes
    4,      // min payload
    255;    // max payload
    data,   // serializer arg 1
    Vec<u8> // serializer type 1
    );
command!(Reset, 0x25, 32);
command!(
    SectorErase,
    0x26,
    0,
    7;
    address,
    u32
);
command!(
    Crc32,
    0x27,
    0,
    15;
    address,
    u32,
    size,
    u32,
    repeat,
    u32
    );
command!(ChipId, 0x20, 0, 7; value, u32);
command!(GetChipId, 0x28, 42);
command!(
    MemoryRead,
    0x2A,
    272,
    9;
    address,
    u32,
    access_type,
    u8,
    size,
    u8
    );
command!(
    MemoryWrite,
    0x2B,
    50,
    9,
    255;
    address,
    u32,
    size,
    u32
);
command!(
    BankErase,
    0x2C,
    0,
    3;
);
command!(
    Crc32Response,
    0x00,
    0,
    7;
    value,
    u32
);
command!(
    CommandStatus,
    0x00,
    0,
    4;
    value,
    StatusValue
);

#[test]
fn test_bl_packet_serializer() {
    let cmd = Crc32::new(0x3030, 0xABAB, 0);

    let packet: Vec<u8> = cmd.serialize().unwrap();
    let checksum = (0x27 + 0x30 + 0x30 + 0xAB + 0xAB) & 0xFF;
    assert_eq!(
        packet.as_slice(),
        [
            15, // packet length
            checksum as u8,
            0x27, // command byte
            0x00, // MSB mem location
            0x00,
            0x30,
            0x30, // LSB mem location
            0x00, // MSB length
            0x00,
            0xAB,
            0xAB, // LSB length
            0,
            0,
            0,
            0
        ]
    );
}

#[test]
fn test_bl_packet_deserializer() {
    let checksum = (0x30 + 0x30 + 0xAB + 0xAB) & 0xFF;
    let data_from_bus = vec![
        0,
        0,
        0xCC,
        14, // packet length
        checksum as u8,
        0x00, // MSB mem location
        0x00,
        0x30,
        0x30, // LSB mem location
        0x00, // MSB length
        0x00,
        0xAB,
        0xAB,
        0x00,
        0x00,
        0x00,
        0x00,
    ];
    let response = Crc32::from_payload(data_from_bus).unwrap();
    assert_eq!(response.address, 0x3030);
    assert_eq!(response.size, 0xABAB);
}
