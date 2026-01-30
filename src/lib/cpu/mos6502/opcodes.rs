use paste::paste;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum AddressingMode {
    Immediate,
    Absolute,
    ZeroPage,
    Accum,
    Implied,
    IndexX,
    IndexY,
    ZeroPageX,
    ZeroPageY,
    ZeroPageIndirect, // 65c02
    ZeroPageRel,      // wdc 65c02
    AbsX,
    AbsXIndirect,
    AbsY,
    Relative,
    Indirect,
    Nop2, // 65c02
    Nop3, // 65c02
}

macro_rules! opcodes {
    ($(
        ($name:ident,
            [
                $(
                ($mode:ident, $value:expr, $cycles:expr, $instruction_bytes:expr)
                ),+
            ]
        )
    ),+) => {
        paste! {
            #[derive(Debug)]
            pub enum Opcodes {
                $(
                    $(
                        [<$name $mode>],
                    )*
                )*
            }

            impl std::fmt::Display for Opcodes {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
                    f.write_fmt(format_args!("{}, {:?}", self.opcode().name, self.opcode().mode))
                }
            }
        }

        paste! {
            #[derive(Debug)]
            pub enum Instruction {
                $(
                    $name,
                )*
            }
        }

        #[derive(Debug)]
        pub struct Opcode {
            pub name: &'static str,
            pub instruction: Instruction,
            pub value: u8,
            pub mode: AddressingMode,
            pub instruction_bytes: usize,
            pub cycles: usize,
        }

        paste! {
            impl Opcodes {
                pub fn opcode(&self) -> Opcode {
                    match self {
                        $(
                            $(
                                Opcodes::[<$name $mode>] => Opcode {
                                    instruction: Instruction::$name,
                                    name: stringify!([<$name:lower>]),
                                    value: $value,
                                    mode: AddressingMode::$mode,
                                    instruction_bytes: $instruction_bytes,
                                    cycles: $cycles,
                                },
                            )*
                        )*
                    }
                }
            }
        }

        paste! {
            pub fn decode_instruction(instruction: u8) -> Option<Opcodes> {
                match instruction {
                    $(
                        $(
                            $value => Some(Opcodes::[<$name $mode>]),
                        )*
                    )*
                }
            }
        }
    };
}

opcodes!(
    (
        Adc,
        [
            (Immediate, 0x69, 2, 2),
            (Absolute, 0x6d, 4, 3),
            (ZeroPage, 0x65, 3, 2),
            (ZeroPageIndirect, 0x72, 5, 2),
            (IndexX, 0x61, 6, 2),
            (IndexY, 0x71, 5, 2),
            (ZeroPageX, 0x75, 4, 2),
            (AbsX, 0x7d, 4, 3),
            (AbsY, 0x79, 4, 3)
        ]
    ),
    (
        And,
        [
            (Immediate, 0x29, 2, 2),
            (Absolute, 0x2d, 4, 3),
            (ZeroPage, 0x25, 3, 2),
            (ZeroPageIndirect, 0x32, 5, 2),
            (IndexX, 0x21, 6, 2),
            (IndexY, 0x31, 5, 2),
            (ZeroPageX, 0x35, 4, 2),
            (AbsX, 0x3d, 4, 3),
            (AbsY, 0x39, 4, 3)
        ]
    ),
    (
        Asl,
        [
            (Absolute, 0x0e, 6, 3),
            (ZeroPage, 0x06, 5, 2),
            (Accum, 0x0a, 2, 1),
            (ZeroPageX, 0x16, 6, 2),
            (AbsX, 0x1e, 7, 3)
        ]
    ),
    (Bcc, [(Relative, 0x90, 2, 2)]),
    (Bcs, [(Relative, 0xb0, 2, 2)]),
    (Beq, [(Relative, 0xf0, 2, 2)]),
    (
        Bit,
        [
            (Absolute, 0x2c, 4, 3),
            (ZeroPage, 0x24, 3, 2),
            (ZeroPageX, 0x34, 3, 2),
            (AbsX, 0x3c, 4, 3),
            (Immediate, 0x89, 2, 2)
        ]
    ),
    (Bmi, [(Relative, 0x30, 2, 2)]),
    (Bne, [(Relative, 0xd0, 2, 2)]),
    (Bpl, [(Relative, 0x10, 2, 2)]),
    (Bra, [(Relative, 0x80, 3, 2)]),
    (Brk, [(Implied, 0x00, 7, 1)]),
    (Bbr0, [(ZeroPageRel, 0x0f, 5, 3)]),
    (Bbr1, [(ZeroPageRel, 0x1f, 5, 3)]),
    (Bbr2, [(ZeroPageRel, 0x2f, 5, 3)]),
    (Bbr3, [(ZeroPageRel, 0x3f, 5, 3)]),
    (Bbr4, [(ZeroPageRel, 0x4f, 5, 3)]),
    (Bbr5, [(ZeroPageRel, 0x5f, 5, 3)]),
    (Bbr6, [(ZeroPageRel, 0x6f, 5, 3)]),
    (Bbr7, [(ZeroPageRel, 0x7f, 5, 3)]),
    (Bbs0, [(ZeroPageRel, 0x8f, 5, 3)]),
    (Bbs1, [(ZeroPageRel, 0x9f, 5, 3)]),
    (Bbs2, [(ZeroPageRel, 0xaf, 5, 3)]),
    (Bbs3, [(ZeroPageRel, 0xbf, 5, 3)]),
    (Bbs4, [(ZeroPageRel, 0xcf, 5, 3)]),
    (Bbs5, [(ZeroPageRel, 0xdf, 5, 3)]),
    (Bbs6, [(ZeroPageRel, 0xef, 5, 3)]),
    (Bbs7, [(ZeroPageRel, 0xff, 5, 3)]),
    (Bvc, [(Relative, 0x50, 2, 2)]),
    (Bvs, [(Relative, 0x70, 2, 2)]),
    (Clc, [(Implied, 0x18, 2, 1)]),
    (Cld, [(Implied, 0xd8, 2, 1)]),
    (Cli, [(Implied, 0x58, 2, 1)]),
    (Clv, [(Implied, 0xb8, 2, 1)]),
    (
        Cmp,
        [
            (Immediate, 0xc9, 2, 2),
            (Absolute, 0xcd, 4, 3),
            (ZeroPage, 0xc5, 3, 2),
            (ZeroPageIndirect, 0xd2, 5, 2),
            (IndexX, 0xc1, 6, 2),
            (IndexY, 0xd1, 5, 2),
            (ZeroPageX, 0xd5, 4, 2),
            (AbsX, 0xdd, 4, 3),
            (AbsY, 0xd9, 4, 3)
        ]
    ),
    (
        Cpx,
        [
            (Immediate, 0xe0, 2, 2),
            (Absolute, 0xec, 4, 3),
            (ZeroPage, 0xe4, 3, 2)
        ]
    ),
    (
        Cpy,
        [
            (Immediate, 0xc0, 2, 2),
            (Absolute, 0xcc, 4, 3),
            (ZeroPage, 0xc4, 3, 2)
        ]
    ),
    (
        Dec,
        [
            (Absolute, 0xce, 6, 3),
            (ZeroPage, 0xc6, 5, 2),
            (ZeroPageX, 0xd6, 6, 2),
            (Accum, 0x3a, 2, 1),
            (AbsX, 0xde, 7, 3)
        ]
    ),
    (Dex, [(Implied, 0xca, 2, 1)]),
    (Dey, [(Implied, 0x88, 2, 1)]),
    (
        Eor,
        [
            (Immediate, 0x49, 2, 2),
            (Absolute, 0x4d, 4, 3),
            (ZeroPage, 0x45, 3, 2),
            (ZeroPageIndirect, 0x52, 5, 2),
            (IndexX, 0x41, 6, 2),
            (IndexY, 0x51, 5, 2),
            (ZeroPageX, 0x55, 4, 2),
            (AbsX, 0x5d, 4, 3),
            (AbsY, 0x59, 4, 3)
        ]
    ),
    (
        Inc,
        [
            (Absolute, 0xee, 6, 3),
            (ZeroPage, 0xe6, 5, 2),
            (ZeroPageX, 0xf6, 6, 2),
            (Accum, 0x1a, 2, 1),
            (AbsX, 0xfe, 7, 3)
        ]
    ),
    (Inx, [(Implied, 0xe8, 2, 1)]),
    (Iny, [(Implied, 0xc8, 2, 1)]),
    (
        Jmp,
        [
            (Absolute, 0x4c, 3, 3),
            (Indirect, 0x6c, 5, 3),
            (AbsXIndirect, 0x7c, 6, 3)
        ]
    ),
    (Jsr, [(Absolute, 0x20, 6, 3)]),
    (
        Lda,
        [
            (Immediate, 0xa9, 2, 2),
            (Absolute, 0xad, 4, 3),
            (ZeroPage, 0xa5, 3, 2),
            (ZeroPageIndirect, 0xb2, 5, 2),
            (IndexX, 0xa1, 6, 2),
            (IndexY, 0xb1, 5, 2),
            (ZeroPageX, 0xb5, 4, 2),
            (AbsX, 0xbd, 4, 3),
            (AbsY, 0xb9, 4, 3)
        ]
    ),
    (
        Ldx,
        [
            (Immediate, 0xa2, 2, 2),
            (Absolute, 0xae, 4, 3),
            (ZeroPage, 0xa6, 3, 2),
            (AbsY, 0xbe, 4, 3),
            (ZeroPageY, 0xb6, 4, 2)
        ]
    ),
    (
        Ldy,
        [
            (Immediate, 0xa0, 2, 2),
            (Absolute, 0xac, 4, 3),
            (ZeroPage, 0xa4, 3, 2),
            (ZeroPageX, 0xb4, 4, 2),
            (AbsX, 0xbc, 4, 3)
        ]
    ),
    (
        Lsr,
        [
            (Absolute, 0x4e, 6, 3),
            (ZeroPage, 0x46, 5, 2),
            (Accum, 0x4a, 2, 1),
            (ZeroPageX, 0x56, 6, 2),
            (AbsX, 0x5e, 7, 3)
        ]
    ),
    (Nop, [(Implied, 0xea, 2, 1)]),
    (NopC2, [(Nop2, 0xc2, 2, 2)]), // REP #, need to NOP for machine identification on ProDOS?
    // 65c02 NOPs
    (Nop03, [(Implied, 0x03, 1, 1)]),
    (Nop13, [(Implied, 0x13, 1, 1)]),
    (Nop23, [(Implied, 0x23, 1, 1)]),
    (Nop33, [(Implied, 0x33, 1, 1)]),
    (Nop43, [(Implied, 0x43, 1, 1)]),
    (Nop53, [(Implied, 0x53, 1, 1)]),
    (Nop63, [(Implied, 0x63, 1, 1)]),
    (Nop73, [(Implied, 0x73, 1, 1)]),
    (Nop83, [(Implied, 0x83, 1, 1)]),
    (Nop93, [(Implied, 0x93, 1, 1)]),
    (NopA3, [(Implied, 0xA3, 1, 1)]),
    (NopB3, [(Implied, 0xb3, 1, 1)]),
    (NopC3, [(Implied, 0xc3, 1, 1)]),
    (NopD3, [(Implied, 0xd3, 1, 1)]),
    (NopE3, [(Implied, 0xe3, 1, 1)]),
    (NopF3, [(Implied, 0xf3, 1, 1)]),
    (Nop0B, [(Implied, 0x0b, 1, 1)]),
    (Nop1B, [(Implied, 0x1b, 1, 1)]),
    (Nop2B, [(Implied, 0x2b, 1, 1)]),
    (Nop3B, [(Implied, 0x3b, 1, 1)]),
    (Nop4B, [(Implied, 0x4b, 1, 1)]),
    (Nop5B, [(Implied, 0x5b, 1, 1)]),
    (Nop6B, [(Implied, 0x6b, 1, 1)]),
    (Nop7B, [(Implied, 0x7b, 1, 1)]),
    (Nop8B, [(Implied, 0x8b, 1, 1)]),
    (Nop9B, [(Implied, 0x9b, 1, 1)]),
    (NopAB, [(Implied, 0xab, 1, 1)]),
    (NopBB, [(Implied, 0xbb, 1, 1)]),
    (NopEB, [(Implied, 0xeb, 1, 1)]),
    (NopFB, [(Implied, 0xfb, 1, 1)]),
    (Nop02, [(Nop2, 0x02, 2, 2)]),
    (Nop22, [(Nop2, 0x22, 2, 2)]),
    (Nop42, [(Nop2, 0x42, 2, 2)]),
    (Nop62, [(Nop2, 0x62, 2, 2)]),
    (Nop82, [(Nop2, 0x82, 2, 2)]),
    (Nope2, [(Nop2, 0xe2, 2, 2)]),
    (Nop44, [(ZeroPage, 0x44, 3, 2)]),
    (Nop54, [(ZeroPageX, 0x54, 4, 2)]),
    (NopD4, [(ZeroPageX, 0xd4, 4, 2)]),
    (Nopf4, [(ZeroPageX, 0xf4, 4, 2)]),
    (NopDc, [(Absolute, 0xdc, 4, 3)]),
    (NopFc, [(Absolute, 0xfc, 4, 3)]),
    (Nop5c, [(Absolute, 0x5c, 8, 3)]),
    (
        Ora,
        [
            (Immediate, 0x09, 2, 2),
            (Absolute, 0x0d, 4, 3),
            (ZeroPage, 0x05, 3, 2),
            (ZeroPageIndirect, 0x12, 5, 2),
            (IndexX, 0x01, 6, 2),
            (IndexY, 0x11, 5, 2),
            (ZeroPageX, 0x15, 4, 2),
            (AbsX, 0x1d, 4, 3),
            (AbsY, 0x19, 4, 3)
        ]
    ),
    (Pha, [(Implied, 0x48, 3, 1)]),
    (Php, [(Implied, 0x08, 3, 1)]),
    (Phx, [(Implied, 0xda, 3, 1)]),
    (Phy, [(Implied, 0x5a, 3, 1)]),
    (Plx, [(Implied, 0xfa, 4, 1)]),
    (Ply, [(Implied, 0x7a, 4, 1)]),
    (Pla, [(Implied, 0x68, 4, 1)]),
    (Plp, [(Implied, 0x28, 4, 1)]),
    (
        Rol,
        [
            (Absolute, 0x2e, 6, 3),
            (ZeroPage, 0x26, 5, 2),
            (Accum, 0x2a, 2, 1),
            (ZeroPageX, 0x36, 6, 2),
            (AbsX, 0x3e, 7, 3)
        ]
    ),
    (
        Ror,
        [
            (Absolute, 0x6e, 6, 3),
            (ZeroPage, 0x66, 5, 2),
            (Accum, 0x6a, 2, 1),
            (ZeroPageX, 0x76, 6, 2),
            (AbsX, 0x7e, 7, 3)
        ]
    ),
    (Rmb0, [(ZeroPage, 0x07, 5, 2)]),
    (Rmb1, [(ZeroPage, 0x17, 5, 2)]),
    (Rmb2, [(ZeroPage, 0x27, 5, 2)]),
    (Rmb3, [(ZeroPage, 0x37, 5, 2)]),
    (Rmb4, [(ZeroPage, 0x47, 5, 2)]),
    (Rmb5, [(ZeroPage, 0x57, 5, 2)]),
    (Rmb6, [(ZeroPage, 0x67, 5, 2)]),
    (Rmb7, [(ZeroPage, 0x77, 5, 2)]),
    (Smb0, [(ZeroPage, 0x87, 5, 2)]),
    (Smb1, [(ZeroPage, 0x97, 5, 2)]),
    (Smb2, [(ZeroPage, 0xA7, 5, 2)]),
    (Smb3, [(ZeroPage, 0xB7, 5, 2)]),
    (Smb4, [(ZeroPage, 0xC7, 5, 2)]),
    (Smb5, [(ZeroPage, 0xD7, 5, 2)]),
    (Smb6, [(ZeroPage, 0xE7, 5, 2)]),
    (Smb7, [(ZeroPage, 0xF7, 5, 2)]),
    (Stp, [(Implied, 0xdb, 3, 1)]),
    (Rti, [(Implied, 0x40, 6, 1)]),
    (Rts, [(Implied, 0x60, 6, 1)]),
    (
        Sbc,
        [
            (Immediate, 0xe9, 2, 2),
            (Absolute, 0xed, 4, 3),
            (ZeroPage, 0xe5, 3, 2),
            (ZeroPageIndirect, 0xf2, 5, 2),
            (IndexX, 0xe1, 6, 2),
            (IndexY, 0xf1, 5, 2),
            (ZeroPageX, 0xf5, 4, 2),
            (AbsX, 0xfd, 4, 3),
            (AbsY, 0xf9, 4, 3)
        ]
    ),
    (Sec, [(Implied, 0x38, 2, 1)]),
    (Sed, [(Implied, 0xf8, 2, 1)]),
    (Sei, [(Implied, 0x78, 2, 1)]),
    (
        Sta,
        [
            (Absolute, 0x8d, 4, 3),
            (ZeroPage, 0x85, 3, 2),
            (ZeroPageIndirect, 0x92, 5, 2),
            (IndexX, 0x81, 6, 2),
            (IndexY, 0x91, 6, 2),
            (ZeroPageX, 0x95, 4, 2),
            (AbsX, 0x9d, 5, 3),
            (AbsY, 0x99, 5, 3)
        ]
    ),
    (
        Stx,
        [
            (Absolute, 0x8e, 4, 3),
            (ZeroPage, 0x86, 3, 2),
            (ZeroPageY, 0x96, 4, 2)
        ]
    ),
    (
        Sty,
        [
            (Absolute, 0x8c, 4, 3),
            (ZeroPage, 0x84, 3, 2),
            (ZeroPageX, 0x94, 4, 2)
        ]
    ),
    (
        Stz,
        [
            (ZeroPage, 0x64, 3, 2),
            (ZeroPageX, 0x74, 4, 2),
            (Absolute, 0x9c, 4, 3),
            (AbsX, 0x9e, 5, 3)
        ]
    ),
    (Tay, [(Implied, 0xa8, 2, 1)]),
    (Tax, [(Implied, 0xaa, 2, 1)]),
    (Trb, [(ZeroPage, 0x14, 5, 2), (Absolute, 0x1c, 6, 3)]),
    (Tsb, [(ZeroPage, 0x04, 5, 2), (Absolute, 0x0c, 6, 3)]),
    (Tsx, [(Implied, 0xba, 2, 1)]),
    (Txa, [(Implied, 0x8a, 2, 1)]),
    (Txs, [(Implied, 0x9a, 2, 1)]),
    (Tya, [(Implied, 0x98, 2, 1)]),
    (Wai, [(Implied, 0xcb, 3, 1)])
);
