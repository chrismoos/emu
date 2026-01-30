use std::{
    io::{Read, Seek},
    sync::Mutex,
};

use log::trace;

use crate::{errors::Error, targets::appleii::io::disks::SectorData};

// Maps physical sector to DOS sector
const PHYSICAL_TO_LOGICAL: &[usize] = &[
    0, 7, 0xe, 6, 0xd, 0x5, 0xc, 0x4, 0xb, 0x3, 0xa, 0x2, 0x9, 0x1, 0x8, 0xf,
];

// Maps physical sector to ProDOS sector
const PHYSICAL_TO_LOGICAL_PRODOS: &[usize] =
    &[0, 8, 1, 9, 2, 10, 3, 11, 4, 12, 5, 13, 6, 14, 7, 15];

pub enum SystemType {
    Dos,
    ProDos,
}

struct State<R> {
    reader: R,
    sector: usize,
}

pub struct DskFile<R> {
    sectors_per_track: usize,
    bytes_per_sector: usize,
    state: Mutex<State<R>>,
    tracks: usize,
    system_type: SystemType,
}

impl<R> DskFile<R>
where
    R: Read + Seek,
{
    pub fn new_16_sector(reader: R, system_type: SystemType) -> DskFile<R> {
        DskFile {
            state: Mutex::new(State {
                reader: reader,
                sector: 0,
            }),
            sectors_per_track: 16,
            bytes_per_sector: 256,
            tracks: 35,
            system_type,
        }
    }

    pub fn read(&self, track: usize) -> Result<SectorData, Error> {
        if track >= self.tracks {
            return Ok(SectorData {
                sector: 0,
                data: vec![0u8; 256],
            });
        }

        let mut state = self.state.lock().unwrap();
        let logical_sector = match self.system_type {
            SystemType::Dos => PHYSICAL_TO_LOGICAL[state.sector],
            SystemType::ProDos => PHYSICAL_TO_LOGICAL_PRODOS[state.sector],
        };

        let pos = (self.sectors_per_track * self.bytes_per_sector * track)
            + (logical_sector * self.bytes_per_sector);
        state.reader.seek(std::io::SeekFrom::Start(pos as u64))?;
        trace!(
            "Seek to position {} for Track {}, Logical Sector {} (mapped from Physical {})",
            pos, track, logical_sector, state.sector
        );

        let mut buf = vec![0u8; self.bytes_per_sector];
        state.reader.read_exact(&mut buf)?;

        let sector = state.sector;

        // wrap around
        state.sector += 1;
        if state.sector == self.sectors_per_track {
            state.sector = 0;
        }
        Ok(SectorData { sector, data: buf })
    }
}
