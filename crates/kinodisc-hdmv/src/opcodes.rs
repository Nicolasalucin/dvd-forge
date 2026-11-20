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
/// Two operands (op_cnt = 2), otherwise the mark operand is never fetched.
pub const OP_PLAY_PLAYLIST_PM: u32 = 0x4282_0000;
/// `SetStream`: set/system with two operands (destination and source).
pub const OP_SET_STREAM: u32 = 0x5180_0001;
/// `SetGPR`: set/set with `MOVE` and an immediate source.
pub const OP_SET_GPR: u32 = 0x5040_0001;
/// `Move(register, register)`: set/set with `MOVE`, both operands registers.
pub const OP_MOVE_REG: u32 = 0x5000_0001;
/// Compare a register with an immediate (`LT` = option 7, `GE` = option 4,
/// unsigned). When the comparison is false the next command is skipped.
pub const OP_CMP_LT_IMM: u32 = 0x4840_0700;
pub const OP_CMP_GE_IMM: u32 = 0x4840_0400;
/// Register operand that names a player status register instead of a GPR.
pub const PSR_OPERAND: u32 = 0x8000_0000;
/// Scratch GPR for the audio buttons (0 and 1 carry the play marks).
pub const SUBTITLE_SCRATCH_GPR: u16 = 2;

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

    /// Decodes the 12-byte on-disc representation.
    pub fn from_bytes(bytes: &[u8; 12]) -> Self {
        Self {
            opcode: u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            dst: u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            src: u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
        }
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

/// Changes the primary audio stream. `track` is zero-based; HDMV primary audio
/// stream numbers (PSR1) are one-based.
pub const fn set_audio_stream(track: u8) -> HdmvCommand {
    HdmvCommand::new(OP_SET_STREAM, 0x8000_0000 | ((track as u32 + 1) << 16), 0)
}

/// Selects the presentation-graphics subtitle stream and turns subtitle display
/// on. `stream` is zero-based; PSR2 stream numbers are one-based.
pub const fn set_subtitle_stream(stream: u8) -> HdmvCommand {
    HdmvCommand::new(OP_SET_STREAM, 0x0000_C000 | (stream as u32 + 1), 0)
}

/// Turns subtitle display off: no stream flag is set, so the selected
/// presentation-graphics stream stays as it is and only the display flag
/// (bit 14 of the destination operand) is cleared.
pub const fn disable_subtitle_stream() -> HdmvCommand {
    HdmvCommand::new(OP_SET_STREAM, 0, 0)
}

/// Changes the primary audio stream and leaves subtitle display as it is.
///
/// `SetStream` rewrites the subtitle display flag (PSR2 bit 31) on every
/// call, so a plain audio change would switch the subtitles off. This reads
/// PSR2 into `scratch` and issues the audio change with the display flag the
/// player already has (a compare skips the command that does not apply).
pub const fn set_audio_stream_keeping_subtitles(track: u8, scratch: u16) -> [HdmvCommand; 5] {
    let audio = 0x8000_0000 | ((track as u32 + 1) << 16);
    let register = scratch as u32;
    [
        HdmvCommand::new(OP_MOVE_REG, register, PSR_OPERAND | 2),
        HdmvCommand::new(OP_CMP_LT_IMM, register, 0x8000_0000),
        HdmvCommand::new(OP_SET_STREAM, audio, 0),
        HdmvCommand::new(OP_CMP_GE_IMM, register, 0x8000_0000),
        HdmvCommand::new(OP_SET_STREAM, audio | 0x0000_4000, 0),
    ]
}

/// `SetGPR(register, value)`: register `0..4095`, immediate value.
pub const fn set_gpr(register: u16, value: u32) -> HdmvCommand {
    HdmvCommand::new(OP_SET_GPR, register as u32, value)
}

/// Decodes a 12-byte HDMV navigation command.
pub fn decode_command(bytes: &[u8; 12]) -> HdmvCommand {
    HdmvCommand::from_bytes(bytes)
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
        assert_eq!(audio.dst, 0x8004_0000);
        let subtitle = set_subtitle_stream(2);
        assert_eq!(subtitle.dst, 0x0000_C003);
        // Audio that keeps subtitles: PSR2 -> scratch, then one of two
        // SetStream commands (display flag off / on), never the stream flag.
        let keep = set_audio_stream_keeping_subtitles(3, SUBTITLE_SCRATCH_GPR);
        assert_eq!(
            keep[0].to_bytes(),
            [0x50, 0, 0, 1, 0, 0, 0, 2, 0x80, 0, 0, 2]
        );
        assert_eq!(keep[1].to_bytes()[..4], [0x48, 0x40, 0x07, 0x00]);
        assert_eq!(keep[2], audio);
        assert_eq!(keep[3].to_bytes()[..4], [0x48, 0x40, 0x04, 0x00]);
        assert_eq!(keep[4].dst, 0x8004_4000);
        assert_eq!(keep[1].src, 0x8000_0000);
        // Off: neither the stream flag (bit 15) nor the display flag (bit 14).
        let off = disable_subtitle_stream();
        assert_eq!(off.opcode(), OP_SET_STREAM);
        assert_eq!(off.dst & 0x0000_C000, 0);
        assert_eq!(off.dst & 0x8000_0000, 0);
    }

    #[test]
    fn play_playlist_mark_has_two_operands() {
        let command = play_playlist_mark(1, 10);
        assert_eq!(command.opcode() >> 29, 2);
        assert_eq!((command.opcode() >> 16) & 0x0F, 2);
        assert_eq!(command.src, 10);
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

    #[test]
    fn every_canonical_command_round_trips_through_twelve_bytes() {
        for command in [
            jump_title(3),
            jump_object(4),
            play_playlist(5),
            play_playlist_mark(6, 7),
            set_audio_stream(1),
            set_subtitle_stream(2),
            disable_subtitle_stream(),
            set_gpr(15, 0xDEAD_BEEF),
        ] {
            assert_eq!(decode_command(&command.to_bytes()), command);
        }
    }
}
