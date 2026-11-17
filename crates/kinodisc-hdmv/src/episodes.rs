//! Per-episode playback for HDMV discs.
//!
//! Kinodisc muxes every episode into one movie (`00000.mpls`, one PlayItem,
//! one entry mark per chapter). Playing "episode 2" from a menu used to start
//! at its mark and run to the end of the disc. Here each episode gets its own
//! playlist — the same clip with IN/OUT limited to the episode — and a
//! MovieObject that plays it from the mark in a GPR and then jumps to the
//! next object (the root menu, or the next episode). Play All keeps using
//! the full movie.

use crate::bdmv::BdmvError;
use crate::opcodes::{jump_object, play_playlist_mark, set_gpr, HdmvCommand};

/// GPR holding the mark (relative to the episode playlist) to start from.
pub const EPISODE_MARK_GPR: u16 = 1;

/// Timeline of a single-PlayItem movie playlist (45 kHz units).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MplsTimeline {
    pub in_time: u32,
    pub out_time: u32,
    /// Entry-mark timestamps in playlist order.
    pub marks: Vec<u32>,
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, BdmvError> {
    bytes
        .get(offset..offset + 2)
        .map(|value| u16::from_be_bytes([value[0], value[1]]))
        .ok_or(BdmvError::InvalidMpls("lectura fuera de rango"))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, BdmvError> {
    bytes
        .get(offset..offset + 4)
        .map(|value| u32::from_be_bytes([value[0], value[1], value[2], value[3]]))
        .ok_or(BdmvError::InvalidMpls("lectura fuera de rango"))
}

struct Layout {
    clip_start: usize,
    mark_pos: usize,
    mark_count: usize,
}

fn layout(mpls: &[u8]) -> Result<Layout, BdmvError> {
    if mpls.len() < 40 || (&mpls[..8] != b"MPLS0200" && &mpls[..8] != b"MPLS0100") {
        return Err(BdmvError::InvalidMpls("firma MPLS0100/0200 ausente"));
    }
    let list_pos = read_u32(mpls, 8)? as usize;
    let mark_pos = read_u32(mpls, 12)? as usize;
    if read_u16(mpls, list_pos + 6)? != 1 {
        return Err(BdmvError::InvalidMpls(
            "se esperaba exactamente un playitem",
        ));
    }
    let playitem_len = read_u16(mpls, list_pos + 10)? as usize;
    if playitem_len < 34 || list_pos + 12 + playitem_len > mpls.len() {
        return Err(BdmvError::InvalidMpls("playitem fuera de rango"));
    }
    let mark_len = read_u32(mpls, mark_pos)? as usize;
    let mark_count = read_u16(mpls, mark_pos + 4)? as usize;
    if mark_len < 2 + mark_count * 14 || mark_pos + 4 + mark_len > mpls.len() {
        return Err(BdmvError::InvalidMpls("tabla de marcas fuera de rango"));
    }
    Ok(Layout {
        clip_start: list_pos + 12,
        mark_pos,
        mark_count,
    })
}

/// Reads IN/OUT and the entry marks of a single-PlayItem playlist.
pub fn read_mpls_timeline(mpls: &[u8]) -> Result<MplsTimeline, BdmvError> {
    let layout = layout(mpls)?;
    let in_time = read_u32(mpls, layout.clip_start + 12)?;
    let out_time = read_u32(mpls, layout.clip_start + 16)?;
    let mut marks = Vec::with_capacity(layout.mark_count);
    for index in 0..layout.mark_count {
        let entry = layout.mark_pos + 6 + index * 14;
        if mpls[entry + 1] == 0x01 {
            marks.push(read_u32(mpls, entry + 4)?);
        }
    }
    Ok(MplsTimeline {
        in_time,
        out_time,
        marks,
    })
}

/// Copy of a single-PlayItem playlist limited to `[in_time, out_time)`:
/// IN/OUT are rewritten and only the marks inside the range are kept (a mark
/// at `in_time` is guaranteed to be the first). Data after the mark table
/// (extension data) is moved and its start address updated.
pub fn derive_range_mpls(mpls: &[u8], in_time: u32, out_time: u32) -> Result<Vec<u8>, BdmvError> {
    let layout = layout(mpls)?;
    let original = read_mpls_timeline(mpls)?;
    if out_time <= in_time || in_time < original.in_time || out_time > original.out_time {
        return Err(BdmvError::InvalidMpls(
            "rango del episodio fuera del playlist",
        ));
    }
    let mut output = mpls[..layout.mark_pos].to_vec();
    output[layout.clip_start + 12..layout.clip_start + 16].copy_from_slice(&in_time.to_be_bytes());
    output[layout.clip_start + 16..layout.clip_start + 20].copy_from_slice(&out_time.to_be_bytes());

    let mut entries: Vec<[u8; 14]> = Vec::new();
    for index in 0..layout.mark_count {
        let start = layout.mark_pos + 6 + index * 14;
        let mut entry = [0u8; 14];
        entry.copy_from_slice(&mpls[start..start + 14]);
        let time = u32::from_be_bytes([entry[4], entry[5], entry[6], entry[7]]);
        if (in_time..out_time).contains(&time) {
            entries.push(entry);
        }
    }
    let starts_at_in = entries.first().is_some_and(|entry| {
        u32::from_be_bytes([entry[4], entry[5], entry[6], entry[7]]) == in_time
    });
    if !starts_at_in {
        let mut entry = [0u8; 14];
        entry[1] = 0x01;
        entry[4..8].copy_from_slice(&in_time.to_be_bytes());
        entry[8..10].copy_from_slice(&[0xFF, 0xFF]);
        entries.insert(0, entry);
    }
    let count =
        u16::try_from(entries.len()).map_err(|_| BdmvError::InvalidMpls("demasiadas marcas"))?;
    let mark_len = 2 + entries.len() as u32 * 14;
    output.extend_from_slice(&mark_len.to_be_bytes());
    output.extend_from_slice(&count.to_be_bytes());
    for entry in &entries {
        output.extend_from_slice(entry);
    }
    let old_mark_end = layout.mark_pos + 4 + read_u32(mpls, layout.mark_pos)? as usize;
    let new_mark_end = output.len();
    output.extend_from_slice(&mpls[old_mark_end..]);
    let extension = read_u32(mpls, 16)? as usize;
    if extension >= old_mark_end {
        let moved = (extension + new_mark_end) - old_mark_end;
        output[16..20].copy_from_slice(&(moved as u32).to_be_bytes());
    }
    Ok(output)
}

/// Appends an object that plays `playlist` from the mark in
/// [`EPISODE_MARK_GPR`], clears it and then jumps to `after_object`. Returns the new
/// MovieObject.bdmv and the index of the added object.
pub fn append_episode_object(
    movie_object: &[u8],
    playlist: u16,
    after_object: u16,
) -> Result<(Vec<u8>, u16), BdmvError> {
    // The register is cleared before leaving so the next episode (or the
    // next button press) starts from its beginning.
    let commands: [HdmvCommand; 3] = [
        play_playlist_mark(u32::from(playlist), EPISODE_MARK_GPR),
        set_gpr(EPISODE_MARK_GPR, 0),
        jump_object(u32::from(after_object)),
    ];
    if movie_object.len() < 50
        || (&movie_object[..8] != b"MOBJ0200" && &movie_object[..8] != b"MOBJ0100")
    {
        return Err(BdmvError::InvalidMovieObject("firma MOBJ0100/0200 ausente"));
    }
    let data_len =
        read_u32(movie_object, 40).map_err(|_| BdmvError::InvalidMovieObject("data_len"))? as usize;
    let num_objects =
        read_u16(movie_object, 48).map_err(|_| BdmvError::InvalidMovieObject("num_objects"))?;
    if 44 + data_len != movie_object.len() {
        return Err(BdmvError::InvalidMovieObject("data_len incoherente"));
    }
    let mut output = movie_object.to_vec();
    output.extend_from_slice(&0x8000u16.to_be_bytes());
    output.extend_from_slice(&(commands.len() as u16).to_be_bytes());
    for command in &commands {
        output.extend_from_slice(&command.to_bytes());
    }
    let new_len = data_len + 4 + commands.len() * 12;
    output[40..44].copy_from_slice(&(new_len as u32).to_be_bytes());
    output[48..50].copy_from_slice(&(num_objects + 1).to_be_bytes());
    Ok((output, num_objects))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::{build_movie_object, build_playlist_mpls, NativePlaylist, NativeStream};

    fn movie(marks: Vec<u32>) -> Vec<u8> {
        build_playlist_mpls(&NativePlaylist {
            clip_id: 0,
            in_time: 0,
            out_time: 900_000,
            streams: vec![NativeStream::video(0x1011)],
            marks,
            still_time: None,
        })
        .expect("mpls")
    }

    #[test]
    fn episode_playlist_keeps_only_its_range_and_marks() {
        let mpls = movie(vec![0, 100_000, 300_000, 450_000, 700_000]);
        let timeline = read_mpls_timeline(&mpls).unwrap();
        assert_eq!(timeline.marks, vec![0, 100_000, 300_000, 450_000, 700_000]);
        let episode = derive_range_mpls(&mpls, 300_000, 700_000).unwrap();
        let derived = read_mpls_timeline(&episode).unwrap();
        assert_eq!(derived.in_time, 300_000);
        assert_eq!(derived.out_time, 700_000);
        assert_eq!(derived.marks, vec![300_000, 450_000]);
        // A range not starting on a mark gets one inserted.
        let derived =
            read_mpls_timeline(&derive_range_mpls(&mpls, 50_000, 100_000).unwrap()).unwrap();
        assert_eq!(derived.marks, vec![50_000]);
        assert!(derive_range_mpls(&mpls, 700_000, 700_000).is_err());
        assert!(derive_range_mpls(&mpls, 0, 900_001).is_err());
    }

    #[test]
    fn episode_object_plays_then_returns() {
        let base = build_movie_object(&[jump_object(0)]).unwrap();
        let (with_episode, index) = append_episode_object(&base, 7, 3).unwrap();
        assert_eq!(index, 1);
        assert_eq!(crate::bdmv::movie_object_count(&with_episode).unwrap(), 2);
        let tail = &with_episode[with_episode.len() - 36..];
        let command = |index: usize| {
            let bytes: [u8; 12] = tail[index * 12..index * 12 + 12].try_into().unwrap();
            crate::opcodes::decode_command(&bytes)
        };
        assert_eq!(command(0), play_playlist_mark(7, EPISODE_MARK_GPR));
        assert_eq!(command(1), set_gpr(EPISODE_MARK_GPR, 0));
        assert_eq!(command(2), jump_object(3));
    }
}
