use crate::targets::appleii::io::video::CharacterGenerator;

pub struct CharacterROM {
    data_primary: Vec<u8>,
    data_alternate: Vec<u8>,
}

impl CharacterROM {
    #[allow(clippy::useless_vec)]
    pub fn new(data: &[u8], reverse_bits: bool, invert_bits: bool, enhanced: bool) -> CharacterROM {
        let mut data_primary: Vec<u8> = match enhanced {
            false => data.to_vec(),
            true => vec![
                &data[0..0x40 * 8],
                &data[0x80 * 8..((0x80 * 8) + (0x40 * 8))],
                &data[0x80 * 8..],
            ]
            .concat(),
        }
        .iter()
        .map(|s| if reverse_bits { s.reverse_bits() } else { *s })
        .map(|s| if invert_bits { !s } else { s })
        .map(|s| if enhanced { s >> 1 } else { s })
        .collect();

        let mut data_alternate: Vec<u8> = data
            .iter()
            .map(|s| if reverse_bits { s.reverse_bits() } else { *s })
            .map(|s| if invert_bits { !s } else { s })
            .map(|s| if enhanced { s >> 1 } else { s })
            .collect();

        // on original apple ii video rom, there is no inversion in the character rom, so apply that here to be consistent with newer ones
        if !enhanced {
            for x in 0..0x40 * 8 {
                data_primary[x] = !data_primary[x];
                data_alternate[x] = !data_alternate[x];
            }
        }

        CharacterROM {
            data_primary,
            data_alternate,
        }
    }
}

impl CharacterGenerator for CharacterROM {
    fn get_character(&self, code: u8, alternate: bool) -> &[u8] {
        let start = (code as usize) * 8;
        if alternate {
            return &self.data_alternate[start..start + 8];
        } else {
            return &self.data_primary[start..start + 8];
        }
    }
}
