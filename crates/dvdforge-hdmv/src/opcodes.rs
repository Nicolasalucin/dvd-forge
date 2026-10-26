//! HDMV virtual-machine navigation commands.
//!
//! A navigation command is 12 bytes on disc: a 32-bit instruction word followed
//! by a 32-bit destination and a 32-bit source operand.
//!
//! Instruction bits (MSB first, as libbluray's `mobj_parse_cmd` reads them):
//!
//! ```text
//! 31..29 op_cnt | 28..27 group | 26..24 sub-group |
//! 23 imm_op1 | 22 imm_op2 | 21..20 reserved | 19..16 branch option |
//! 15..12 reserved | 11..8 compare option | 7..5 reserved | 4..0 set option
//! ```

pub const OP_NOP: u32 = 0x0000_0000;
/// `JumpTitle`: branch/jump, one immediate operand.
pub const OP_JUMP_TITLE: u32 = 0x2181_0000;
/// `JumpObject`: branch/jump to a MovieObject, one immediate operand.
pub const OP_JUMP_OBJECT: u32 = 0x2180_0000;
/// `PlayPlaylist`: branch/play, one immediate operand.
pub const OP_PLAY_PLAYLIST: u32 = 0x2280_0000;
/// `PlayPlaylist` from a playmark: immediate playlist + mark in a GPR.
pub const OP_PLAY_PLAYLIST_PM: u32 = 0x2282_0000;
/// `SetStream`: set/system with two operands (destination and source).
pub const OP_SET_STREAM: u32 = 0x5180_0001;
/// `SetGPR`: set/set with `MOVE` and an immediate source.
pub const OP_SET_GPR: u32 = 0x5040_0001;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HdmvCommand {
    pub opcode: u32,
    pub dst: u32,
    pub src: u32,
}

impl HdmvCommand {
    pub const fn new(opcode: u32, dst: u32, src: u32) -> Self {
        Self { opcode, dst, src }
    }

    /// Encodes the command exactly as it appears in an IG button.
    pub fn to_bytes(self) -> [u8; 12] {
        let mut out = [0u8; 12];
        out[0..4].copy_from_slice(&self.opcode.to_be_bytes());
        out[4..8].copy_from_slice(&self.dst.to_be_bytes());
        out[8..12].copy_from_slice(&self.src.to_be_bytes());
        out
    }

    pub const fn opcode(self) -> u32 {
        self.opcode
    }
}

pub const fn jump_title(title: u32) -> HdmvCommand {
    HdmvCommand::new(OP_JUMP_TITLE, title, 0)
}

pub const fn jump_object(object: u32) -> HdmvCommand {
    HdmvCommand::new(OP_JUMP_OBJECT, object, 0)
}

pub const fn play_playlist(playlist: u32) -> HdmvCommand {
    HdmvCommand::new(OP_PLAY_PLAYLIST, playlist, 0)
}

/// Starts `playlist` at the playmark stored in `register`.
pub const fn play_playlist_mark(playlist: u32, register: u16) -> HdmvCommand {
    HdmvCommand::new(OP_PLAY_PLAYLIST_PM, playlist, register as u32)
}

/// Changes the primary audio stream. `track` is zero-based.
pub const fn set_audio_stream(track: u8) -> HdmvCommand {
    HdmvCommand::new(OP_SET_STREAM, 0x8000_0000 | ((track as u32) << 16), 0)
}

/// Changes the presentation-graphics subtitle stream. `stream` is zero-based.
pub const fn set_subtitle_stream(stream: u8) -> HdmvCommand {
    HdmvCommand::new(OP_SET_STREAM, 0x0000_8000 | stream as u32, 0)
}

/// `SetGPR(register, value)`: register `0..4095`, immediate value.
pub const fn set_gpr(register: u16, value: u32) -> HdmvCommand {
    HdmvCommand::new(OP_SET_GPR, register as u32, value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jump_title_uses_the_canonical_opcode() {
        let command = jump_title(7);
        assert_eq!(command.opcode(), 0x2181_0000);
        assert_eq!(
            command.to_bytes(),
            [0x21, 0x81, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0, 0, 0, 0]
        );
    }

    #[test]
    fn play_playlist_and_set_stream_encode_operands_big_endian() {
        assert_eq!(play_playlist(0x0102_0304).to_bytes()[4..8], [1, 2, 3, 4]);
        let audio = set_audio_stream(3);
        assert_eq!(audio.opcode(), OP_SET_STREAM);
        assert_eq!(audio.dst, 0x8003_0000);
        let subtitle = set_subtitle_stream(2);
        assert_eq!(subtitle.dst, 0x0000_8002);
    }

    #[test]
    fn set_gpr_is_a_move_with_immediate_source() {
        let command = set_gpr(15, 0xDEAD_BEEF);
        assert_eq!(command.opcode(), 0x5040_0001);
        assert_eq!(
            command.to_bytes(),
            [0x50, 0x40, 0x00, 0x01, 0, 0, 0, 15, 0xDE, 0xAD, 0xBE, 0xEF]
        );
    }
}
