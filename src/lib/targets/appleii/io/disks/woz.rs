use std::{
    collections::HashMap,
    fmt::Debug,
    io::{self, BufRead, Cursor, ErrorKind, Read, Seek},
    sync::Mutex,
};

use crate::{
    errors::Error,
    targets::appleii::io::disks::{self, FloppyDiskReader, crc::crc32, dsk::DskFile},
    utils::bitstream::Bitstream,
};

use byteorder::{LittleEndian, ReadBytesExt};
use log::trace;
use rand::Rng;

const MAX_CHUNK_SIZE: usize = 1024 * 1024 * 8;

struct WozDiskTrack {
    total_bits: usize,
    bit_index: usize,
    sr: u8,
    random: [bool; 32],
    random_index: usize,
    data: Vec<u8>,
    zeros: usize,
}

impl Debug for WozDiskTrack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WozDiskTrack")
            .field("total_bits", &self.total_bits)
            .field("bit_index", &self.bit_index)
            .field("sr", &self.sr)
            .field("random", &self.random)
            .field("random_index", &self.random_index)
            .field("data len", &self.data.len())
            .field("zeros", &self.zeros)
            .finish()
    }
}

impl WozDiskTrack {
    pub fn new_v2(track_data: &TrackDataV2) -> WozDiskTrack {
        let mut rng = rand::rng();
        let mut random = [false; 32];
        rng.fill(&mut random);

        WozDiskTrack {
            random_index: 0,
            random: random.clone(),
            total_bits: track_data.num_bits,
            bit_index: 0,
            sr: 0,
            zeros: 0,
            data: track_data.data.clone(),
        }
    }

    pub fn new(track_data: &TrackData) -> WozDiskTrack {
        let mut rng = rand::rng();
        let mut random = [false; 32];
        rng.fill(&mut random);

        WozDiskTrack {
            random_index: 0,
            random: random.clone(),
            total_bits: track_data.num_bits,
            bit_index: 0,
            sr: 0,
            zeros: 0,
            data: track_data.bitstream.clone(),
        }
    }
    fn next_bit(&mut self) -> u8 {
        let byte = self.data[self.bit_index / 8];
        let bit = (byte >> (7 - (self.bit_index % 8))) & 1;

        self.bit_index += 1;
        if self.bit_index == self.total_bits {
            self.bit_index = 0;
        }

        if bit == 1 {
            self.zeros = 0;
        } else {
            self.zeros += 1;
        }

        if self.zeros > 3 {
            let bit = self.random[self.random_index];
            self.random_index += 1;
            if self.random_index == self.random.len() {
                self.random_index = 0;
            }
            if bit { 1 } else { 0 }
        } else {
            bit
        }
    }

    pub fn reset(&mut self, bit_index: usize) {
        if bit_index >= self.total_bits {
            self.bit_index = 0;
        } else {
            self.bit_index = bit_index;
        }
        //self.sr = 0;
    }

    pub fn write_bit(&mut self, data: u8) -> usize {
        let byte_index = self.bit_index / 8;

        // clear the bit
        self.data[byte_index] &= !(1 << (7 - (self.bit_index % 8)));

        // or the bit
        self.data[byte_index] |= (data & 1) << (7 - (self.bit_index % 8));

        self.bit_index += 1;
        if self.bit_index == self.total_bits {
            self.bit_index = 0;
        }
        self.bit_index
    }

    pub fn next_byte(&mut self) -> (u8, usize) {
        if (self.sr >> 7) == 1 {
            self.sr = 0;
        }
        let bit = self.next_bit();
        self.sr <<= 1;
        self.sr |= bit;
        (self.sr, self.bit_index)
    }
}

#[derive(Debug)]
struct WozDiskState {
    pub current_track_index: usize,
    pub tracks: Vec<WozDiskTrack>,
}

#[derive(Debug)]
pub struct WozDisk {
    pub info_chunk: InfoChunk,
    pub chunks: Vec<Chunk>,
    state: Mutex<WozDiskState>,
}

impl<R> TryFrom<DskFile<R>> for WozDisk
where
    R: Read + Seek,
{
    type Error = Error;

    fn try_from(value: DskFile<R>) -> Result<Self, Self::Error> {
        let mut chunks = vec![];
        let mut tracks = vec![];
        let mut indexes = [0xffu8; 160];
        let mut current_index = 0;

        for track in 0..35 {
            let mut bitstream = Bitstream::new();
            for sector in 0..16 {
                let sector_data = value.read(track)?;
                let sector_data_encoded = disks::encode_sector(track, sector, &sector_data.data)?;
                for b in 0..sector_data_encoded.num_bits() {
                    bitstream.write_bit(sector_data_encoded.read_bit(b));
                }
            }

            tracks.push(TrackData {
                bitstream: bitstream.packed().to_vec(),
                num_bytes: bitstream.packed().len(),
                num_bits: bitstream.num_bits(),
                splice_point: 0,
                splice_nibble: 0,
                splice_bit_count: 0,
            });

            for _ in 0..3 {
                indexes[current_index] = (current_index / 4) as u8;
            }
            current_index += 4;
        }

        chunks.push(Chunk::TrackMap525(TrackMap525Chunk { indexes }));
        chunks.push(Chunk::Tracks(TracksChunk { tracks }));

        let tracks = chunks
            .iter()
            .flat_map(|c| match c {
                Chunk::Tracks(tracks_chunk) => tracks_chunk
                    .tracks
                    .iter()
                    .map(|t| WozDiskTrack::new(t))
                    .collect::<Vec<_>>(),
                _ => vec![],
            })
            .collect::<Vec<_>>();

        let woz_disk = WozDisk {
            info_chunk: InfoChunk {
                version: WozVersion::V1 as u8,
                disk_type: DiskType::Disk525,
                write_protected: false,
                synchronized: false,
                cleaned: false,
                creator: "emu".to_owned(),
            },
            state: Mutex::new(WozDiskState {
                current_track_index: 0,
                tracks,
            }),
            chunks,
        };
        Ok(woz_disk)
    }
}

impl WozDisk {
    pub fn parse<R>(reader: R) -> Result<WozDisk, Error>
    where
        R: Read,
    {
        WozDiskParser::parse(reader)
    }

    fn get_track_data_at_index(&self, index: usize) -> Option<&TrackData> {
        self.chunks.iter().find_map(|c| match c {
            Chunk::Tracks(tracks_chunk) => Some(&tracks_chunk.tracks[index]),
            _ => None,
        })
    }

    pub fn seek_track(&self, track: f32) -> Result<(), Error> {
        match self.info_chunk.disk_type {
            DiskType::Disk525 => {
                match self.chunks.iter().find_map(|c| match c {
                    Chunk::TrackMap525(track_map525_chunk) => Some(track_map525_chunk),
                    _ => None,
                }) {
                    Some(tm) => {
                        let index = (track as usize) % 40;
                        let subindex = ((track - (track as usize as f32)) * 4.0) as usize;
                        trace!("get track at {}.{}", index, (subindex as f32 / 4.0) * 100.0);
                        let track_index = tm.indexes[(index * 4) + subindex] as usize;
                        self.state.lock().unwrap().current_track_index = track_index;
                        Ok(())
                    }
                    None => return Err("track not found".into()),
                }
            }
            DiskType::Disk35 => return Err("3.5in not supported!".into()),
        }
    }

    pub fn write_bit(&self, index: usize, data: u8) -> Result<usize, Error> {
        let mut state = self.state.lock().unwrap();
        let track_index = state.current_track_index;
        if track_index >= state.tracks.len() {
            return Ok(0);
        }

        state.tracks[track_index].reset(index);
        let index = state.tracks[track_index].write_bit(data);
        Ok(index)
    }

    pub fn get_byte(&self, index: usize) -> (Option<u8>, usize) {
        let mut state = self.state.lock().unwrap();
        let track_index = state.current_track_index;
        if track_index >= state.tracks.len() {
            return (None, 0);
        }

        state.tracks[track_index].reset(index);
        let (byte, index) = state.tracks[track_index].next_byte();
        (Some(byte), index)
    }

    pub fn get_track_data(&self, track: f32) -> Option<&TrackData> {
        match self.info_chunk.disk_type {
            DiskType::Disk525 => {
                match self.chunks.iter().find_map(|c| match c {
                    Chunk::TrackMap525(track_map525_chunk) => Some(track_map525_chunk),
                    _ => None,
                }) {
                    Some(tm) => {
                        let index = (track as usize) % 40;
                        let subindex = ((track - (track as usize as f32)) * 100.0) as usize;
                        let track_index = tm.indexes[(index * 4) + subindex] as usize;
                        trace!("get track at {}.{}", index, subindex);
                        self.get_track_data_at_index(track_index)
                    }
                    None => return None,
                }
            }
            DiskType::Disk35 => None,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum DiskType {
    Disk525,
    Disk35,
}

impl TryFrom<u8> for DiskType {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(DiskType::Disk525),
            2 => Ok(DiskType::Disk35),
            _ => Err(format!("unknown disk type {}", value).into()),
        }
    }
}

#[derive(Debug)]
pub struct TrackMap35Chunk {
    pub indexes: [[u8; 80]; 3], // two sides, each 80 tracks
}

impl TrackMap35Chunk {
    pub fn parse(buf: &[u8]) -> Result<TrackMap35Chunk, Error> {
        if buf.len() != 160 {
            return Err("invalid size for TMAP chunk".into());
        }

        let mut tm = TrackMap35Chunk {
            indexes: [[0u8; 80]; 3],
        };

        tm.indexes[0].copy_from_slice(&buf[0..80]);
        tm.indexes[1].copy_from_slice(&buf[80..]);

        Ok(tm)
    }
}

#[derive(Debug)]
pub struct TrackMap525Chunk {
    pub indexes: [u8; 160], // 0.25 increments
}

impl TrackMap525Chunk {
    pub fn parse(buf: &[u8]) -> Result<TrackMap525Chunk, Error> {
        if buf.len() != 160 {
            return Err("invalid size for TMAP chunk".into());
        }

        let mut tm = TrackMap525Chunk {
            indexes: [0u8; 160],
        };
        tm.indexes.copy_from_slice(buf);
        Ok(tm)
    }
}

#[derive(Debug)]
pub struct InfoChunk {
    pub version: u8,
    pub disk_type: DiskType,
    pub write_protected: bool,
    pub synchronized: bool,
    pub cleaned: bool,
    pub creator: String,
}

impl InfoChunk {
    pub fn parse(data: &[u8]) -> Result<InfoChunk, Error> {
        if data.len() != 60 {
            return Err("invalid size when reading info chunk".into());
        }

        let mut cursor = Cursor::new(data);

        let info = InfoChunk {
            version: cursor.read_u8()?,
            disk_type: DiskType::try_from(cursor.read_u8()?)?,
            write_protected: cursor.read_u8()? == 1,
            synchronized: cursor.read_u8()? == 1,
            cleaned: cursor.read_u8()? == 1,
            creator: Self::parse_info_creator(cursor)?,
        };

        Ok(info)
    }

    fn parse_info_creator<B>(mut reader: B) -> Result<String, Error>
    where
        B: Read,
    {
        let mut creator = vec![0u8; 32];
        reader.read_exact(&mut creator)?;

        Ok(String::from_utf8(creator)?.trim().to_string())
    }
}

#[derive(Clone)]
pub struct TrackDataV2 {
    starting_block: u16,
    block_count: u16,
    num_bits: usize,
    data: Vec<u8>,
}

impl Debug for TrackDataV2 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrackDataV2")
            .field("starting_block", &self.starting_block)
            .field("block_count", &self.block_count)
            .field("num_bits", &self.num_bits)
            .field("data len", &self.data.len())
            .finish()
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct TrackData {
    bitstream: Vec<u8>,
    num_bytes: usize,
    num_bits: usize,
    splice_point: usize,
    splice_nibble: u8,
    splice_bit_count: usize,
}

impl TrackData {
    pub fn parse(buf: &[u8]) -> Result<TrackData, Error> {
        let mut cursor = Cursor::new(buf);
        let mut bitstream = vec![0u8; 6646];
        cursor.read_exact(&mut bitstream)?;

        Ok(TrackData {
            bitstream,
            num_bytes: cursor.read_u16::<LittleEndian>()? as usize,
            num_bits: cursor.read_u16::<LittleEndian>()? as usize,
            splice_point: cursor.read_u16::<LittleEndian>()? as usize,
            splice_nibble: cursor.read_u8()?,
            splice_bit_count: cursor.read_u8()? as usize,
        })
    }
}

#[derive(Debug)]
pub struct MetaChunk {
    pub metadata: HashMap<String, String>,
}

impl MetaChunk {
    pub fn parse(data: &[u8]) -> Result<MetaChunk, Error> {
        let mut cursor = Cursor::new(data);
        let mut chunk = MetaChunk {
            metadata: HashMap::new(),
        };
        loop {
            let mut line = String::new();
            match cursor.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    if let Some((p, s)) = line.trim().split_once("\t") {
                        chunk.metadata.insert(p.to_owned(), s.to_owned());
                    }
                }
                Err(e) => return Err(e.into()),
            }
        }
        Ok(chunk)
    }
}

#[derive(Debug)]
pub struct TracksChunkV2 {
    pub tracks: Vec<TrackDataV2>,
}

impl TracksChunkV2 {
    pub fn parse(data: &[u8]) -> Result<TracksChunkV2, Error> {
        let mut cursor = Cursor::new(data);
        let mut tracks = vec![];
        for _ in 0..160 {
            let starting_block = cursor.read_u16::<LittleEndian>()?;
            let block_count = cursor.read_u16::<LittleEndian>()?;
            let num_bits = cursor.read_u32::<LittleEndian>()? as usize;

            if starting_block == 0 || block_count == 0 {
                continue;
            }

            let start_offset = 1280 + ((starting_block as usize - 3) * 512);
            let block_data = &data[start_offset..start_offset + (512 * block_count as usize)];
            let td = TrackDataV2 {
                starting_block,
                block_count,
                num_bits,
                data: block_data.to_vec(),
            };

            tracks.push(td);
        }

        Ok(TracksChunkV2 { tracks })
    }
}

#[derive(Debug)]
pub struct TracksChunk {
    pub tracks: Vec<TrackData>,
}

impl TracksChunk {
    pub fn parse(data: &[u8]) -> Result<TracksChunk, Error> {
        if data.len() % 6656 != 0 {
            return Err("invalid size for tracks data".into());
        }

        Ok(TracksChunk {
            tracks: data
                .chunks_exact(6656)
                .map(|d| TrackData::parse(d))
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

#[derive(Debug)]
pub enum Chunk {
    Info(InfoChunk),
    TrackMap525(TrackMap525Chunk),
    TrackMap35(TrackMap35Chunk),
    Tracks(TracksChunk),
    TracksV2(TracksChunkV2),
    Meta(MetaChunk),
    Unknown(u32),
}

struct WozDiskParser {
    reader: Cursor<Vec<u8>>,
}

impl WozDiskParser {
    pub fn parse<R>(mut reader: R) -> Result<WozDisk, Error>
    where
        R: Read,
    {
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf)?;

        let crcval = crc32(0, &buf[12..]);

        let mut parser = WozDiskParser {
            reader: Cursor::new(buf),
        };
        let header = parser.parse_header()?;

        let info_chunk = parser.read_info_chunk()?;
        let chunks = parser.read_chunks(&header, &info_chunk)?;

        if header.crc != 0x00 && crcval != header.crc {
            return Err("invalid CRC, bad file".into());
        }

        let tracks = chunks
            .iter()
            .flat_map(|c| match c {
                Chunk::Tracks(tracks_chunk) => tracks_chunk
                    .tracks
                    .iter()
                    .map(|t| WozDiskTrack::new(t))
                    .collect::<Vec<_>>(),
                Chunk::TracksV2(tracks_chunk) => tracks_chunk
                    .tracks
                    .iter()
                    .map(|t| WozDiskTrack::new_v2(t))
                    .collect::<Vec<_>>(),
                _ => vec![],
            })
            .collect::<Vec<_>>();

        if tracks.len() == 0 {
            return Err("missing tracks".into());
        }

        Ok(WozDisk {
            info_chunk,
            chunks,
            state: Mutex::new(WozDiskState {
                current_track_index: 0,
                tracks,
            }),
        })
    }

    fn read_chunks(&mut self, header: &Header, info: &InfoChunk) -> Result<Vec<Chunk>, Error> {
        let mut chunks = vec![];
        loop {
            match self.read_chunk(header, info)? {
                Some(chunk) => {
                    chunks.push(chunk);
                }
                None => {
                    break;
                }
            }
        }
        Ok(chunks)
    }

    fn read_info_chunk(&mut self) -> Result<InfoChunk, Error> {
        if self.reader.read_u32::<LittleEndian>()? != 0x4F464E49 {
            return Err("invalid chunk id when looking for INFO".into());
        }

        let chunk_size = self.reader.read_u32::<LittleEndian>()? as usize;
        if chunk_size != 60 {
            return Err("invalid chunk size for INFO".into());
        }

        let mut data = vec![0u8; chunk_size];
        self.reader.read_exact(&mut data)?;
        InfoChunk::parse(&data)
    }

    fn read_chunk(&mut self, header: &Header, info: &InfoChunk) -> Result<Option<Chunk>, Error> {
        let chunk_id = match self.reader.read_u32::<LittleEndian>() {
            Ok(val) => val,
            Err(e) => {
                let ioerr = io::Error::from(e);
                if ioerr.kind() == ErrorKind::UnexpectedEof {
                    return Ok(None);
                } else {
                    return Err(ioerr.into_inner().ok_or("other error")?);
                }
            }
        };

        let chunk_size = self.reader.read_u32::<LittleEndian>()? as usize;
        if chunk_size > MAX_CHUNK_SIZE {
            return Err(format!(
                "chunk size {} larger than max allowed {}",
                chunk_size, MAX_CHUNK_SIZE
            )
            .into());
        }

        let mut data = vec![0u8; chunk_size];
        self.reader.read_exact(&mut data)?;

        Ok(Some(match chunk_id {
            0x50414D54 => match info.disk_type {
                DiskType::Disk525 => Chunk::TrackMap525(TrackMap525Chunk::parse(&data)?),
                DiskType::Disk35 => Chunk::TrackMap35(TrackMap35Chunk::parse(&data)?),
            },
            0x4154454D => Chunk::Meta(MetaChunk::parse(&data)?),
            0x534B5254 => match header.version {
                WozVersion::V1 => Chunk::Tracks(TracksChunk::parse(&data)?),
                WozVersion::V2 => Chunk::TracksV2(TracksChunkV2::parse(&data)?),
            },
            _ => Chunk::Unknown(chunk_id),
        }))
    }

    fn parse_header(&mut self) -> Result<Header, Error> {
        let version = match self.reader.read_u32::<LittleEndian>()? {
            0x315A4F57 => WozVersion::V1,
            0x325A4F57 => WozVersion::V2,
            _ => return Err("invalid signature".into()),
        };

        if self.reader.read_u8()? != 0xff {
            return Err("expected 0xff in header".into());
        }

        let mut lfcrlf = [0u8; 3];
        self.reader.read_exact(&mut lfcrlf)?;
        if lfcrlf != [0x0a, 0x0d, 0x0a] {
            return Err("invalid LF CR LF sequence in header".into());
        }

        Ok(Header {
            crc: self.reader.read_u32::<LittleEndian>()?,
            version,
        })
    }
}

#[derive(Debug)]
enum WozVersion {
    V1,
    V2,
}

struct Header {
    crc: u32,
    version: WozVersion,
}

#[derive(Default)]
struct State {
    bit_index: usize,
}

pub struct WozDiskReader {
    disk: WozDisk,
    state: Mutex<State>,
}

impl WozDiskReader {
    pub fn new(disk: WozDisk) -> WozDiskReader {
        let mut rng = rand::rng();
        let mut random = [false; 32];
        rng.fill(&mut random);
        WozDiskReader {
            disk,
            state: Mutex::new(State::default()),
        }
    }
}

impl FloppyDiskReader for WozDiskReader {
    fn seek_track(&self, track: f32) -> Result<(), Error> {
        self.disk.seek_track(track)?;
        Ok(())
    }

    fn read(&self) -> Result<u8, Error> {
        let mut state = self.state.lock().unwrap();
        let (data, next_index) = self.disk.get_byte(state.bit_index);
        state.bit_index = next_index;

        let byte = data.unwrap_or(0);
        Ok(byte)
    }

    fn write(&self, bit: u8) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();
        let next_index = self.disk.write_bit(state.bit_index, bit)?;
        state.bit_index = next_index;
        Ok(())
    }

    fn reset(&self) -> Result<(), Error> {
        *self.state.lock().unwrap() = State::default();
        Ok(())
    }
}
