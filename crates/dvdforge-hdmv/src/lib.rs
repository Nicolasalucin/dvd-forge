//! HDMV Interactive Graphics (IG) compiler for Blu-ray / AVCHD menus.
//!
//! The crate emits the canonical Blu-ray IG segment set
//! (ICS, PDS, WDS, ODS, END) and wraps each segment in its own PES packet with
//! the Interactive Graphics PID (`0x1400`). It also contains the BDMV patching
//! helpers that attach a native IG menu clip to a tsMuxeR-authored disc.

pub mod bdmv;
pub mod compiler;
pub mod m2ts;
pub mod opcodes;
pub mod palette;
pub mod pes;
pub mod rle;
pub mod segments;

pub use bdmv::{
    append_menu_movie_object, dummy_pgs, dummy_pgs_for_slots, inject_ig_into_menu_m2ts,
    movie_object_count, packetize_pes, patch_menu_clpi, patch_menu_mpls,
    pes_transport_packet_count, pgs_transport_packet_count, set_index_menu_object, BdmvError,
    STILL_TIME_INFINITE,
};
pub use compiler::{
    compile_ig_m2ts, compile_ig_pes, compile_ig_pes_packets, compile_ig_segments, frame_rate_code,
    HdmvButton, HdmvButtonState, HdmvError, HdmvMenu, RgbaBitmap,
};
pub use m2ts::m2ts_from_pes_packets;
pub use opcodes::{
    jump_object, jump_title, play_playlist, play_playlist_mark, set_audio_stream, set_gpr,
    set_subtitle_stream, HdmvCommand, OP_JUMP_OBJECT, OP_JUMP_TITLE, OP_NOP, OP_PLAY_PLAYLIST,
    OP_PLAY_PLAYLIST_PM, OP_SET_GPR, OP_SET_STREAM,
};
pub use palette::{Palette, PaletteEntry};
pub use pes::{pes_packets_from_segments, PesError, IG_PID, IG_STREAM_ID};
pub use rle::{decode_rle, encode_rle, encode_rle_lines, RleError};
pub use segments::{
    end_of_display_set_segment, interactive_composition_segment, object_definition_segment,
    palette_definition_segment, window_definition_segment, ButtonDefinition, CompositionObject,
    InteractiveComposition, PageDefinition, Segment, SegmentError, Window, SEGMENT_END,
    SEGMENT_ICS, SEGMENT_ODS, SEGMENT_PDS, SEGMENT_WDS,
};

pub const CRATE_NAME: &str = "dvdforge-hdmv";
