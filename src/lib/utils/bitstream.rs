#[derive(Default)]
pub struct Bitstream {
    buf: Vec<u8>,
    bits: usize,
}

impl Bitstream {
    pub fn new() -> Bitstream {
        Self::default()
    }

    pub fn packed(&self) -> &[u8] {
        &self.buf
    }

    pub fn write_byte(&mut self, byte: u8) {
        for x in 0..8 {
            self.write_bit((byte >> (7 - x)) & 1);
        }
    }

    pub fn write_bytes(&mut self, byte: &[u8]) {
        byte.iter().for_each(|b| self.write_byte(*b));
    }

    pub fn write_bit(&mut self, bit: u8) {
        if self.bits % 8 == 0 {
            self.buf.push(0);
        }
        let index = self.buf.len() - 1;
        self.buf[index] |= (bit & 1) << (7 - (self.bits % 8));
        self.bits += 1;
    }

    pub fn read_bit(&self, bit: usize) -> u8 {
        if bit >= self.bits {
            return 0;
        }
        (self.buf[bit / 8] >> (7 - (bit % 8))) & 1
    }

    pub fn num_bits(&self) -> usize {
        self.bits
    }
}
