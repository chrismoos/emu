use crate::{errors::Error, utils::bitstream::Bitstream};

pub mod crc;
pub mod dsk;
pub mod woz;

pub trait FloppyDiskReader {
    fn seek_track(&self, track: f32) -> Result<(), Error>;

    // Read next bit from the disk
    fn read(&self) -> Result<u8, Error>;

    // Write next bit from the disk
    fn write(&self, bit: u8) -> Result<(), Error>;

    fn reset(&self) -> Result<(), Error>;
}

pub struct SectorData {
    pub sector: usize,
    pub data: Vec<u8>,
}

const LOOKUP_TABLE_62: [u8; 64] = [
    0x96, 0x97, 0x9a, 0x9b, 0x9d, 0x9e, 0x9f, 0xa6, 0xa7, 0xab, 0xac, 0xad, 0xae, 0xaf, 0xb2, 0xb3,
    0xb4, 0xb5, 0xb6, 0xb7, 0xb9, 0xba, 0xbb, 0xbc, 0xbd, 0xbe, 0xbf, 0xcb, 0xcd, 0xce, 0xcf, 0xd3,
    0xd6, 0xd7, 0xd9, 0xda, 0xdb, 0xdc, 0xdd, 0xde, 0xdf, 0xe5, 0xe6, 0xe7, 0xe9, 0xea, 0xeb, 0xec,
    0xed, 0xee, 0xef, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf9, 0xfa, 0xfb, 0xfc, 0xfd, 0xfe, 0xff,
];

fn encode_62(data: &[u8; 256]) -> (Vec<u8>, u8) {
    let mut buf = vec![];
    let mut previous_byte = 0;

    let mut packed_twos = vec![0u8; 86];
    for x in 0..256 {
        let idx = x % 86;
        packed_twos[idx] |= (((data[x] >> 1) & 1) | ((data[x] & 1) << 1)) << ((x / 86) * 2);
    }

    for byte in packed_twos {
        buf.push(LOOKUP_TABLE_62[(byte ^ previous_byte) as usize]);
        previous_byte = byte;
    }

    for byte in data {
        let b = *byte >> 2;
        buf.push(LOOKUP_TABLE_62[(b ^ previous_byte) as usize]);
        previous_byte = b;
    }

    (buf, LOOKUP_TABLE_62[previous_byte as usize])
}

fn encode_44(byte: u8) -> [u8; 2] {
    let x = 0b10101010 | (byte >> 1);
    let y = 0b10101010 | byte;

    return [x, y];
}

fn write_sync(bs: &mut Bitstream, num: usize) {
    for _ in 0..num {
        for _ in 0..8 {
            bs.write_bit(1);
        }
        for _ in 0..2 {
            bs.write_bit(0);
        }
    }
}

pub fn encode_sector(track: usize, sector: usize, data: &[u8]) -> Result<Bitstream, Error> {
    let mut bitstream = Bitstream::new();

    write_sync(&mut bitstream, 16);
    // address field
    let volume = 254;
    bitstream.write_bytes(&[0xd5, 0xaa, 0x96]);
    bitstream.write_bytes(&encode_44(volume as u8));
    bitstream.write_bytes(&encode_44(track as u8));
    bitstream.write_bytes(&encode_44(sector as u8));
    bitstream.write_bytes(&encode_44((volume ^ track ^ sector) as u8));
    bitstream.write_bytes(&[0xde, 0xaa, 0xeb]);

    // gap -- add in woz
    write_sync(&mut bitstream, 7);

    // data field
    bitstream.write_bytes(&[0xd5, 0xaa, 0xad]);

    let (encoded_data, checksum) = encode_62(&data.try_into()?);
    bitstream.write_bytes(&encoded_data);
    bitstream.write_byte(checksum);
    bitstream.write_bytes(&[0xde, 0xaa, 0xeb]);

    Ok(bitstream)
}
