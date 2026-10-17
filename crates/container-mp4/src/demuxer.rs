use crate::track::{CttsEntry, ElstEntry, FragmentRun, StscEntry, SttsEntry, Track};
use codec_core::{CodecError, CodecId, CodecResult, Packet, StreamInfo, Timestamp};
use std::ops::Range;

// Resource policy, enforced before every table reservation.
const MAX_ENTRIES: usize = 1_000_000;
const MAX_TABLE_BYTES: usize = 64 * 1024 * 1024;
const MAX_TRACKS: usize = 64;
const MAX_PACKET_BYTES: usize = 128 * 1024 * 1024;

fn u32_at(data: &[u8], at: usize) -> CodecResult<u32> {
    let bytes = data
        .get(at..at.checked_add(4).ok_or(CodecError::InvalidData)?)
        .ok_or(CodecError::InvalidData)?;
    Ok(u32::from_be_bytes(
        bytes.try_into().map_err(|_| CodecError::InvalidData)?,
    ))
}

fn u64_at(data: &[u8], at: usize) -> CodecResult<u64> {
    let bytes = data
        .get(at..at.checked_add(8).ok_or(CodecError::InvalidData)?)
        .ok_or(CodecError::InvalidData)?;
    Ok(u64::from_be_bytes(
        bytes.try_into().map_err(|_| CodecError::InvalidData)?,
    ))
}

type BoxView<'a> = ([u8; 4], &'a [u8], Range<usize>);

struct Boxes<'a> {
    data: &'a [u8],
    position: usize,
}
impl<'a> Boxes<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, position: 0 }
    }
    fn next(&mut self) -> CodecResult<Option<BoxView<'a>>> {
        if self.position == self.data.len() {
            return Ok(None);
        }
        let start = self.position;
        let size32 = u32_at(self.data, start)?;
        let kind: [u8; 4] = self
            .data
            .get(start + 4..start + 8)
            .ok_or(CodecError::InvalidData)?
            .try_into()
            .map_err(|_| CodecError::InvalidData)?;
        let (size, header) = if size32 == 1 {
            let bytes = self
                .data
                .get(start + 8..start + 16)
                .ok_or(CodecError::InvalidData)?;
            (
                usize::try_from(u64::from_be_bytes(
                    bytes.try_into().map_err(|_| CodecError::InvalidData)?,
                ))
                .map_err(|_| CodecError::InvalidData)?,
                16,
            )
        } else if size32 == 0 {
            (self.data.len() - start, 8)
        } else {
            (
                usize::try_from(size32).map_err(|_| CodecError::InvalidData)?,
                8,
            )
        };
        if size < header {
            return Err(CodecError::InvalidData);
        }
        let end = start.checked_add(size).ok_or(CodecError::InvalidData)?;
        let range = start + header..end;
        let payload = self
            .data
            .get(range.clone())
            .ok_or(CodecError::InvalidData)?;
        self.position = end;
        Ok(Some((kind, payload, range)))
    }
}

fn reserve<T>(table: &mut Vec<T>, count: usize, budget: &mut usize) -> CodecResult<()> {
    let bytes = count
        .checked_mul(std::mem::size_of::<T>())
        .ok_or(CodecError::InvalidData)?;
    if count > MAX_ENTRIES || bytes > *budget {
        return Err(CodecError::InvalidData);
    }
    table
        .try_reserve_exact(count)
        .map_err(|_| CodecError::OutOfMemory)?;
    *budget -= bytes;
    Ok(())
}
fn table_count(data: &[u8], width: usize, allow_v1: bool) -> CodecResult<usize> {
    let flags = u32_at(data, 0)?;
    if flags != 0 && !(allow_v1 && flags == 0x0100_0000) {
        return Err(CodecError::InvalidData);
    }
    let count = usize::try_from(u32_at(data, 4)?).map_err(|_| CodecError::InvalidData)?;
    if count > MAX_ENTRIES
        || count.checked_mul(width).and_then(|n| n.checked_add(8)) != Some(data.len())
    {
        return Err(CodecError::InvalidData);
    }
    Ok(count)
}

#[derive(Debug)]
pub struct Mp4Demuxer<'a> {
    data: &'a [u8],
    pub streams: Vec<StreamInfo>,
    tracks: Vec<Track>,
    mdat: Vec<Range<usize>>,
}
impl<'a> Mp4Demuxer<'a> {
    /// Validated tracks are read-only so packet extraction can trust their tables.
    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            streams: Vec::new(),
            tracks: Vec::new(),
            mdat: Vec::new(),
        }
    }
    pub fn parse_moov(&mut self) -> CodecResult<()> {
        // Transactional parse: a failure cannot leave partially usable tracks.
        self.streams.clear();
        self.tracks.clear();
        self.mdat.clear();
        let mut parsed = Self::new(self.data);
        parsed.parse()?;
        *self = parsed;
        Ok(())
    }
    fn parse(&mut self) -> CodecResult<()> {
        let mut boxes = Boxes::new(self.data);
        let mut moov = None;
        while let Some((kind, payload, range)) = boxes.next()? {
            match &kind {
                b"moov" => {
                    if moov.replace(payload).is_some() {
                        return Err(CodecError::InvalidData);
                    }
                }
                b"mdat" => {
                    if self.mdat.len() >= MAX_ENTRIES {
                        return Err(CodecError::InvalidData);
                    }
                    self.mdat
                        .try_reserve(1)
                        .map_err(|_| CodecError::OutOfMemory)?;
                    self.mdat.push(range);
                }
                b"moof" => return Err(CodecError::Unsupported),
                _ => {}
            }
        }
        let mut budget = MAX_TABLE_BYTES;
        let mut boxes = Boxes::new(moov.ok_or(CodecError::InvalidData)?);
        let mut track_count = 0;
        let mut movie_timescale = 1000u32;
        while let Some((kind, payload, _)) = boxes.next()? {
            if &kind == b"mvex" {
                return Err(CodecError::Unsupported);
            }
            if &kind == b"mvhd" && payload.len() >= 16 {
                let flags = u32_at(payload, 0)?;
                let offset = if flags == 0x0100_0000 && payload.len() >= 24 {
                    20
                } else {
                    12
                };
                if let Ok(ts) = u32_at(payload, offset) {
                    if ts > 0 {
                        movie_timescale = ts;
                    }
                }
                continue;
            }
            if &kind != b"trak" {
                continue;
            }
            track_count += 1;
            if track_count > MAX_TRACKS {
                return Err(CodecError::InvalidData);
            }
            let mut track = Track {
                movie_timescale,
                ..Track::default()
            };
            let mut seen = Vec::new();
            self.track_boxes(payload, &mut track, &mut seen, &mut budget, 0)?;
            for required in [*b"mdhd", *b"stsd", *b"stts", *b"stsc", *b"stsz", *b"stco"] {
                if !seen.contains(&required) {
                    return Err(CodecError::InvalidData);
                }
            }
            if seen.contains(b"ctts") && track.ctts.is_empty() && !track.stsz.is_empty() {
                return Err(CodecError::InvalidData);
            }
            self.validate_track(&mut track, &mut budget)?;
            if let Some(mut info) = track.stream_info.clone() {
                info.stream_id = self.streams.len() as u32;
                track.stream_info = Some(info.clone());
                self.streams.push(info);
                self.tracks.push(track);
            }
        }
        if self.streams.is_empty() {
            return Err(CodecError::Unsupported);
        }
        Ok(())
    }
    fn track_boxes(
        &self,
        data: &[u8],
        track: &mut Track,
        seen: &mut Vec<[u8; 4]>,
        budget: &mut usize,
        depth: usize,
    ) -> CodecResult<()> {
        let mut boxes = Boxes::new(data);
        while let Some((kind, payload, _)) = boxes.next()? {
            if depth == 0 && &kind == b"edts" {
                if seen.contains(&kind) {
                    return Err(CodecError::InvalidData);
                }
                seen.push(kind);
                self.parse_edts(payload, track, budget)?;
                continue;
            }
            let container = match depth {
                0 => *b"mdia",
                1 => *b"minf",
                2 => *b"stbl",
                _ => [0; 4],
            };
            if kind == container {
                if seen.contains(&kind) {
                    return Err(CodecError::InvalidData);
                }
                seen.push(kind);
                self.track_boxes(payload, track, seen, budget, depth + 1)?;
            } else if &kind == b"mdhd" && depth == 1 {
                if seen.contains(&kind) {
                    return Err(CodecError::InvalidData);
                }
                seen.push(kind);
                let flags = u32_at(payload, 0)?;
                let offset = match flags {
                    0 if payload.len() == 24 => 12,
                    0x0100_0000 if payload.len() == 36 => 20,
                    _ => return Err(CodecError::InvalidData),
                };
                track.timescale = u32_at(payload, offset)?;
                if track.timescale == 0 {
                    return Err(CodecError::InvalidData);
                }
            } else if depth == 3 {
                let canonical = if &kind == b"co64" { *b"stco" } else { kind };
                if [
                    *b"stsd", *b"stts", *b"ctts", *b"stsz", *b"stco", *b"stsc", *b"stss",
                ]
                .contains(&canonical)
                {
                    if seen.contains(&canonical) {
                        return Err(CodecError::InvalidData);
                    }
                    seen.push(canonical);
                    self.table(kind, payload, track, budget)?;
                } else if &kind == b"stz2" {
                    return Err(CodecError::Unsupported);
                }
            }
        }
        Ok(())
    }

    fn parse_edts(&self, data: &[u8], track: &mut Track, budget: &mut usize) -> CodecResult<()> {
        let mut boxes = Boxes::new(data);
        while let Some((kind, payload, _)) = boxes.next()? {
            if &kind == b"elst" {
                let flags = u32_at(payload, 0)?;
                let v1 = match flags {
                    0 => false,
                    0x0100_0000 => true,
                    _ => return Err(CodecError::InvalidData),
                };
                let entry_size = if v1 { 20 } else { 12 };
                let count = table_count(payload, entry_size, true)?;
                reserve(&mut track.elst, count, budget)?;
                for i in 0..count {
                    let at = 8 + i * entry_size;
                    let (segment_duration, media_time, rate_offset) = if v1 {
                        (
                            u64_at(payload, at)?,
                            u64_at(payload, at + 8)? as i64,
                            at + 16,
                        )
                    } else {
                        (
                            u64::from(u32_at(payload, at)?),
                            i64::from(u32_at(payload, at + 4)? as i32),
                            at + 8,
                        )
                    };
                    let rate_raw = u32_at(payload, rate_offset)?;
                    track.elst.push(ElstEntry {
                        segment_duration,
                        media_time,
                        media_rate_integer: (rate_raw >> 16) as i16,
                        media_rate_fraction: (rate_raw & 0xffff) as i16,
                    });
                }
            }
        }
        Ok(())
    }

    fn table(
        &self,
        kind: [u8; 4],
        data: &[u8],
        track: &mut Track,
        budget: &mut usize,
    ) -> CodecResult<()> {
        match &kind {
            b"stsd" => self.descriptions(data, track, budget)?,
            b"stsz" => {
                if u32_at(data, 0)? != 0 {
                    return Err(CodecError::InvalidData);
                }
                let size = u32_at(data, 4)?;
                let count =
                    usize::try_from(u32_at(data, 8)?).map_err(|_| CodecError::InvalidData)?;
                let expected = if size == 0 {
                    count.checked_mul(4).and_then(|n| n.checked_add(12))
                } else {
                    Some(12)
                };
                if expected != Some(data.len()) {
                    return Err(CodecError::InvalidData);
                }
                reserve(&mut track.stsz, count, budget)?;
                for i in 0..count {
                    track.stsz.push(if size == 0 {
                        u32_at(data, 12 + i * 4)?
                    } else {
                        size
                    });
                }
            }
            b"stts" | b"ctts" => {
                let count = table_count(data, 8, &kind == b"ctts")?;
                if &kind == b"stts" {
                    reserve(&mut track.stts, count, budget)?;
                } else {
                    reserve(&mut track.ctts, count, budget)?;
                }
                for i in 0..count {
                    let sample_count = u32_at(data, 8 + i * 8)?;
                    let value = u32_at(data, 12 + i * 8)?;
                    if sample_count == 0 {
                        return Err(CodecError::InvalidData);
                    }
                    if &kind == b"stts" {
                        track.stts.push(SttsEntry {
                            sample_count,
                            sample_delta: value,
                        });
                    } else {
                        let sample_offset = if data[0] == 1 {
                            i64::from(value as i32)
                        } else {
                            i64::from(value)
                        };
                        track.ctts.push(CttsEntry {
                            sample_count,
                            sample_offset,
                        });
                    }
                }
                if &kind == b"ctts" && count == 0 && !track.stsz.is_empty() {
                    return Err(CodecError::InvalidData);
                }
            }
            b"stsc" => {
                let count = table_count(data, 12, false)?;
                reserve(&mut track.stsc, count, budget)?;
                for i in 0..count {
                    track.stsc.push(StscEntry {
                        first_chunk: u32_at(data, 8 + i * 12)?,
                        samples_per_chunk: u32_at(data, 12 + i * 12)?,
                        sample_description_index: u32_at(data, 16 + i * 12)?,
                    });
                }
            }
            b"stco" | b"co64" => {
                let width = if &kind == b"co64" { 8 } else { 4 };
                let count = table_count(data, width, false)?;
                reserve(&mut track.stco, count, budget)?;
                for i in 0..count {
                    let at = 8 + i * width;
                    let offset = if width == 8 {
                        u64::from_be_bytes(
                            data[at..at + 8]
                                .try_into()
                                .map_err(|_| CodecError::InvalidData)?,
                        )
                    } else {
                        u64::from(u32_at(data, at)?)
                    };
                    track.stco.push(offset);
                }
            }
            b"stss" => {
                let count = table_count(data, 4, false)?;
                let mut entries = Vec::new();
                reserve(&mut entries, count, budget)?;
                for i in 0..count {
                    entries.push(u32_at(data, 8 + i * 4)?);
                }
                track.stss = Some(entries);
            }
            _ => return Err(CodecError::Unsupported),
        }
        Ok(())
    }
    fn descriptions(&self, data: &[u8], track: &mut Track, budget: &mut usize) -> CodecResult<()> {
        if u32_at(data, 0)? != 0 {
            return Err(CodecError::InvalidData);
        }
        let count = u32_at(data, 4)?;
        if count == 0 || count as usize > MAX_ENTRIES || count as usize > (data.len() - 8) / 16 {
            return Err(CodecError::InvalidData);
        }
        track.sample_description_count = count;
        let mut boxes = Boxes::new(&data[8..]);
        for index in 1..=count {
            let (kind, payload, _) = boxes.next()?.ok_or(CodecError::InvalidData)?;
            if payload.len() < 8 || payload[..6] != [0; 6] || payload[6..8] != [0, 1] {
                return Err(CodecError::Unsupported); // only self-contained data reference 1
            }
            let (codec_id, width, height, extradata) = match &kind {
                b"av01" => {
                    if payload.len() < 78 {
                        return Err(CodecError::InvalidData);
                    }
                    let width = u16::from_be_bytes([payload[24], payload[25]]) as u32;
                    let height = u16::from_be_bytes([payload[26], payload[27]]) as u32;
                    if width == 0 || height == 0 {
                        return Err(CodecError::InvalidData);
                    }
                    let mut children = Boxes::new(&payload[78..]);
                    let mut config = None;
                    while let Some((kind, bytes, _)) = children.next()? {
                        if &kind == b"av1C" && config.replace(bytes).is_some() {
                            return Err(CodecError::InvalidData);
                        }
                    }
                    let config = config.ok_or(CodecError::InvalidData)?;
                    if config.len() < 4
                        || config[0] != 0x81
                        || config[1] >> 5 > 2
                        || config[3] & 0xe0 != 0
                        || (config[3] & 0x10 == 0 && config[3] & 15 != 0)
                    {
                        return Err(CodecError::InvalidData);
                    }
                    let mut extra = Vec::new();
                    reserve(&mut extra, config.len(), budget)?;
                    extra.extend_from_slice(config);
                    (CodecId::Av1, width, height, extra)
                }
                b"mp4a" => {
                    if payload.len() < 28 {
                        return Err(CodecError::InvalidData);
                    }
                    (CodecId::Aac, 0, 0, Vec::new())
                }
                b"Opus" => {
                    if payload.len() < 28 {
                        return Err(CodecError::InvalidData);
                    }
                    (CodecId::Opus, 0, 0, Vec::new())
                }
                b"fLaC" => {
                    if payload.len() < 28 {
                        return Err(CodecError::InvalidData);
                    }
                    (CodecId::Flac, 0, 0, Vec::new())
                }
                b"wvtt" => (CodecId::Webvtt, 0, 0, Vec::new()),
                b"tx3g" => (CodecId::Tx3g, 0, 0, Vec::new()),
                _ => continue,
            };
            // Switching descriptions requires per-packet configuration, not yet supported.
            if track.stream_info.is_some() {
                return Err(CodecError::Unsupported);
            }
            track.selected_description_index = Some(index);
            track.stream_info = Some(StreamInfo {
                stream_id: 0,
                codec_id,
                width,
                height,
                bit_rate: None,
                extradata,
            });
        }
        if boxes.next()?.is_some() {
            return Err(CodecError::InvalidData);
        }
        Ok(())
    }
    fn validate_track(&self, track: &mut Track, budget: &mut usize) -> CodecResult<()> {
        let count = track.stsz.len() as u64;
        if track.timescale == 0 {
            return Err(CodecError::InvalidData);
        }
        let mut covered = 0u64;
        let mut duration = 0i64;
        reserve(&mut track.cumulative_dts, track.stsz.len(), budget)?;
        for t in &track.stts {
            covered = covered
                .checked_add(u64::from(t.sample_count))
                .ok_or(CodecError::InvalidData)?;
            // Reject oversized timing runs before iterating attacker-controlled counts.
            if covered > count {
                return Err(CodecError::InvalidData);
            }
            for _ in 0..t.sample_count {
                if track.cumulative_dts.len() < track.stsz.len() {
                    track.cumulative_dts.push(duration);
                }
                duration = duration
                    .checked_add(i64::from(t.sample_delta))
                    .ok_or(CodecError::InvalidData)?;
            }
        }
        if covered != count {
            return Err(CodecError::InvalidData);
        }
        if !track.ctts.is_empty() {
            let total: u64 = track.ctts.iter().map(|c| u64::from(c.sample_count)).sum();
            if total != count {
                return Err(CodecError::InvalidData);
            }
        }
        if let Some(sync) = &track.stss {
            let mut previous = 0;
            for &sample in sync {
                if sample <= previous || u64::from(sample) > count {
                    return Err(CodecError::InvalidData);
                }
                previous = sample;
            }
        }
        if count == 0 {
            if !track.stco.is_empty() || !track.stsc.is_empty() {
                return Err(CodecError::InvalidData);
            }
            return Ok(());
        }
        if track.stsc.first().map(|s| s.first_chunk) != Some(1) {
            return Err(CodecError::InvalidData);
        }
        let mut sample = 0usize;
        for (i, entry) in track.stsc.iter().enumerate() {
            let next = track
                .stsc
                .get(i + 1)
                .map(|e| e.first_chunk as usize)
                .unwrap_or(track.stco.len() + 1);
            let first = entry.first_chunk as usize;
            if first == 0
                || first >= next
                || next > track.stco.len() + 1
                || entry.samples_per_chunk == 0
                || entry.sample_description_index == 0
                || entry.sample_description_index > track.sample_description_count
            {
                return Err(CodecError::InvalidData);
            }
            if track
                .selected_description_index
                .is_some_and(|index| index != entry.sample_description_index)
            {
                return Err(CodecError::Unsupported);
            }
            for chunk in first - 1..next - 1 {
                let offset =
                    usize::try_from(track.stco[chunk]).map_err(|_| CodecError::InvalidData)?;
                let end_sample = sample
                    .checked_add(entry.samples_per_chunk as usize)
                    .ok_or(CodecError::InvalidData)?;
                let sizes = track
                    .stsz
                    .get(sample..end_sample)
                    .ok_or(CodecError::InvalidData)?;
                let mut end = offset;
                for &size in sizes {
                    if size as usize > MAX_PACKET_BYTES {
                        return Err(CodecError::InvalidData);
                    }
                    end = end
                        .checked_add(size as usize)
                        .ok_or(CodecError::InvalidData)?;
                }
                if !self
                    .mdat
                    .iter()
                    .any(|range| offset >= range.start && end <= range.end)
                {
                    return Err(CodecError::InvalidData);
                }
                sample = end_sample;
            }
        }
        if sample != track.stsz.len() {
            return Err(CodecError::InvalidData);
        }
        Ok(())
    }

    /// Sub-millisecond keyframe bisection seek: finds the latest sync sample (1-based `stss`
    /// converted to 0-based sample index) whose decode timestamp is `<= target_dts`.
    pub fn seek_keyframe_bisect(&self, track_idx: usize, target_dts: i64) -> CodecResult<usize> {
        let track = self.tracks.get(track_idx).ok_or(CodecError::InvalidData)?;
        if track.stsz.is_empty() {
            return Err(CodecError::EndOfStream);
        }
        let target_sample = match track.cumulative_dts.binary_search(&target_dts) {
            Ok(idx) => idx,
            Err(ins) => ins.saturating_sub(1),
        };
        if let Some(ref sync) = track.stss {
            if sync.is_empty() {
                return Err(CodecError::InvalidData);
            }
            let target_1based = (target_sample + 1) as u32;
            let sync_pos = match sync.binary_search(&target_1based) {
                Ok(pos) => pos,
                Err(ins) => ins.saturating_sub(1),
            };
            Ok((sync[sync_pos] - 1) as usize)
        } else {
            Ok(target_sample)
        }
    }

    /// Maps a media-track timestamp (`media_pts` in `track.timescale`) into edit-list (`elst`)
    /// adjusted presentation time in `track.timescale`.
    pub fn compose_edit_list_pts(&self, track_idx: usize, media_pts: i64) -> CodecResult<i64> {
        let track = self.tracks.get(track_idx).ok_or(CodecError::InvalidData)?;
        if track.elst.is_empty() {
            return Ok(media_pts);
        }
        let movie_ts = i128::from(track.movie_timescale.max(1));
        let media_ts = i128::from(track.timescale.max(1));
        let mut presentation_accum = 0i128;
        for entry in &track.elst {
            let seg_in_media = (i128::from(entry.segment_duration) * media_ts) / movie_ts;
            if entry.media_time == -1 {
                // Empty dwell edit shifts presentation start forward
                presentation_accum += seg_in_media;
            } else {
                let start = i128::from(entry.media_time);
                let end = start + seg_in_media;
                let m = i128::from(media_pts);
                if m >= start && (seg_in_media == 0 || m < end) {
                    let out = presentation_accum + (m - start);
                    return i64::try_from(out).map_err(|_| CodecError::InvalidData);
                }
                presentation_accum += seg_in_media;
            }
        }
        Ok(media_pts)
    }

    /// Parses a Fragmented MP4 (`fMP4` / CMAF) `moof` box (`mfhd` + `traf` + `tfhd` + `tfdt` + `trun`)
    /// for adaptive streaming playback.
    pub fn parse_fmp4_moof(moof_payload: &[u8]) -> CodecResult<Vec<FragmentRun>> {
        let mut boxes = Boxes::new(moof_payload);
        let mut sequence_number = 0u32;
        let mut runs = Vec::new();
        while let Some((kind, payload, _)) = boxes.next()? {
            match &kind {
                b"mfhd" => {
                    if payload.len() != 8 || u32_at(payload, 0)? != 0 {
                        return Err(CodecError::InvalidData);
                    }
                    sequence_number = u32_at(payload, 4)?;
                }
                b"traf" => {
                    let mut tboxes = Boxes::new(payload);
                    let mut track_id = 0u32;
                    let mut default_duration = 0u32;
                    let mut default_size = 0u32;
                    let mut base_media_decode_time = 0u64;
                    while let Some((tkind, tpayload, _)) = tboxes.next()? {
                        match &tkind {
                            b"tfhd" => {
                                if tpayload.len() < 8 {
                                    return Err(CodecError::InvalidData);
                                }
                                let flags = u32_at(tpayload, 0)? & 0x00ff_ffff;
                                track_id = u32_at(tpayload, 4)?;
                                let mut cursor = 8usize;
                                if flags & 0x01 != 0 {
                                    cursor += 8;
                                }
                                if flags & 0x02 != 0 {
                                    cursor += 4;
                                }
                                if flags & 0x08 != 0 {
                                    default_duration = u32_at(tpayload, cursor)?;
                                    cursor += 4;
                                }
                                if flags & 0x10 != 0 {
                                    default_size = u32_at(tpayload, cursor)?;
                                }
                            }
                            b"tfdt" => {
                                if tpayload.len() < 8 {
                                    return Err(CodecError::InvalidData);
                                }
                                let version = tpayload[0];
                                base_media_decode_time = if version == 1 {
                                    u64_at(tpayload, 4)?
                                } else {
                                    u64::from(u32_at(tpayload, 4)?)
                                };
                            }
                            b"trun" => {
                                if tpayload.len() < 8 {
                                    return Err(CodecError::InvalidData);
                                }
                                let flags = u32_at(tpayload, 0)? & 0x00ff_ffff;
                                let sample_count = usize::try_from(u32_at(tpayload, 4)?)
                                    .map_err(|_| CodecError::InvalidData)?;
                                if sample_count > MAX_ENTRIES {
                                    return Err(CodecError::InvalidData);
                                }
                                let mut cursor = 8usize;
                                let data_offset = if flags & 0x01 != 0 {
                                    let off = u32_at(tpayload, cursor)? as i32;
                                    cursor += 4;
                                    Some(off)
                                } else {
                                    None
                                };
                                if flags & 0x04 != 0 {
                                    cursor += 4; // first_sample_flags
                                }
                                let mut sample_durations = Vec::with_capacity(sample_count);
                                let mut sample_sizes = Vec::with_capacity(sample_count);
                                let mut sample_composition_offsets =
                                    Vec::with_capacity(sample_count);
                                for _ in 0..sample_count {
                                    let dur = if flags & 0x100 != 0 {
                                        let v = u32_at(tpayload, cursor)?;
                                        cursor += 4;
                                        v
                                    } else {
                                        default_duration
                                    };
                                    let sz = if flags & 0x200 != 0 {
                                        let v = u32_at(tpayload, cursor)?;
                                        cursor += 4;
                                        v
                                    } else {
                                        default_size
                                    };
                                    if flags & 0x400 != 0 {
                                        cursor += 4; // sample_flags
                                    }
                                    let cto = if flags & 0x800 != 0 {
                                        let raw = u32_at(tpayload, cursor)?;
                                        cursor += 4;
                                        if tpayload[0] == 1 {
                                            i64::from(raw as i32)
                                        } else {
                                            i64::from(raw)
                                        }
                                    } else {
                                        0
                                    };
                                    sample_durations.push(dur);
                                    sample_sizes.push(sz);
                                    sample_composition_offsets.push(cto);
                                }
                                runs.push(FragmentRun {
                                    sequence_number,
                                    track_id,
                                    base_media_decode_time,
                                    sample_durations,
                                    sample_sizes,
                                    sample_composition_offsets,
                                    data_offset,
                                });
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(runs)
    }

    pub fn get_sample(&self, track_idx: usize, sample_idx: usize) -> CodecResult<Packet> {
        let track = self.tracks.get(track_idx).ok_or(CodecError::InvalidData)?;
        if sample_idx >= track.stsz.len() {
            return Err(CodecError::EndOfStream);
        }
        let mut base = 0usize;
        let mut location = None;
        for (i, entry) in track.stsc.iter().enumerate() {
            let next = track
                .stsc
                .get(i + 1)
                .map(|e| e.first_chunk as usize)
                .unwrap_or(track.stco.len() + 1);
            for chunk in entry.first_chunk as usize - 1..next - 1 {
                let end = base + entry.samples_per_chunk as usize;
                if sample_idx < end {
                    let mut offset =
                        usize::try_from(track.stco[chunk]).map_err(|_| CodecError::InvalidData)?;
                    for &size in &track.stsz[base..sample_idx] {
                        offset = offset
                            .checked_add(size as usize)
                            .ok_or(CodecError::InvalidData)?;
                    }
                    location = Some(offset);
                    break;
                }
                base = end;
            }
            if location.is_some() {
                break;
            }
        }
        let offset = location.ok_or(CodecError::InvalidData)?;
        let size = track.stsz[sample_idx] as usize;
        let end = offset.checked_add(size).ok_or(CodecError::InvalidData)?;
        let bytes = self.data.get(offset..end).ok_or(CodecError::InvalidData)?;
        let mut data = Vec::new();
        data.try_reserve_exact(size)
            .map_err(|_| CodecError::OutOfMemory)?;
        data.extend_from_slice(bytes);
        let mut dts = 0i64;
        let mut base = 0usize;
        let mut duration = None;
        for t in &track.stts {
            let end = base + t.sample_count as usize;
            let n = if sample_idx < end {
                sample_idx - base
            } else {
                t.sample_count as usize
            };
            dts = dts
                .checked_add(
                    (n as i64)
                        .checked_mul(i64::from(t.sample_delta))
                        .ok_or(CodecError::InvalidData)?,
                )
                .ok_or(CodecError::InvalidData)?;
            if sample_idx < end {
                duration = Some(u64::from(t.sample_delta));
                break;
            }
            base = end;
        }
        let mut composition = 0;
        let mut base = 0usize;
        for c in &track.ctts {
            base += c.sample_count as usize;
            if sample_idx < base {
                composition = c.sample_offset;
                break;
            }
        }
        let pts = dts
            .checked_add(composition)
            .ok_or(CodecError::InvalidData)?;
        Ok(Packet {
            data,
            pts: Some(Timestamp::new(pts, 1, track.timescale)?),
            dts: Some(Timestamp::new(dts, 1, track.timescale)?),
            duration,
            is_keyframe: track
                .stss
                .as_ref()
                .is_none_or(|entries| entries.binary_search(&((sample_idx + 1) as u32)).is_ok()),
            stream_id: track
                .stream_info
                .as_ref()
                .ok_or(CodecError::InvalidData)?
                .stream_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bx(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut data = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        data.extend(kind);
        data.extend(payload);
        data
    }
    fn words(values: &[u32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_be_bytes()).collect()
    }
    fn tables() -> Vec<([u8; 4], Vec<u8>)> {
        let mut entry = vec![0; 78];
        entry[7] = 1;
        entry[25] = 1;
        entry[27] = 1;
        entry.extend(bx(b"av1C", &[0x81, 0, 0x0c, 0]));
        let mut stsd = words(&[0, 1]);
        stsd.extend(bx(b"av01", &entry));
        vec![
            (*b"stsd", stsd),
            (*b"stts", words(&[0, 1, 2, 100])),
            (*b"stsc", words(&[0, 1, 1, 2, 1])),
            (*b"stsz", words(&[0, 1, 2])),
            (*b"stco", words(&[0, 1, 8])),
        ]
    }
    fn file(tables: Vec<([u8; 4], Vec<u8>)>, scale: u32) -> Vec<u8> {
        let stbl: Vec<u8> = tables
            .into_iter()
            .flat_map(|(kind, data)| bx(&kind, &data))
            .collect();
        let mut mdhd = words(&[0, 0, 0, scale, 200, 0]);
        mdhd = bx(b"mdhd", &mdhd);
        mdhd.extend(bx(b"minf", &bx(b"stbl", &stbl)));
        let mut data = bx(b"mdat", &[0, 0]);
        data.extend(bx(b"moov", &bx(b"trak", &bx(b"mdia", &mdhd))));
        data
    }
    fn accepted(data: &[u8]) -> bool {
        Mp4Demuxer::new(data).parse_moov().is_ok()
    }
    #[test]
    fn valid_samples_and_eof() {
        let data = file(tables(), 1000);
        let mut demux = Mp4Demuxer::new(&data);
        demux.parse_moov().unwrap();
        assert_eq!(demux.get_sample(0, 1).unwrap().dts.unwrap().pts, 100);
        assert!(matches!(
            demux.get_sample(0, 2),
            Err(CodecError::EndOfStream)
        ));
        demux.parse_moov().unwrap();
        assert_eq!(demux.tracks.len(), 1);
    }
    #[test]
    fn keyframe_bisect_elst_and_fmp4_parsing() {
        let mut t = tables();
        t.push((*b"stss", words(&[0, 1, 1])));
        let data = file(t, 1000);
        let mut demux = Mp4Demuxer::new(&data);
        demux.parse_moov().unwrap();
        assert_eq!(demux.seek_keyframe_bisect(0, 150).unwrap(), 0);

        // Build synthetic moof (mfhd + traf(tfhd + tfdt + trun))
        let mfhd = bx(b"mfhd", &words(&[0, 7]));
        let tfhd = bx(b"tfhd", &words(&[0x0000_0018, 1, 1024, 512]));
        let tfdt = bx(b"tfdt", &words(&[0, 4096]));
        let trun = bx(b"trun", &words(&[0x0000_0001, 2, 64]));
        let mut traf_payload = Vec::new();
        traf_payload.extend(tfhd);
        traf_payload.extend(tfdt);
        traf_payload.extend(trun);
        let mut moof_payload = mfhd;
        moof_payload.extend(bx(b"traf", &traf_payload));
        let runs = Mp4Demuxer::parse_fmp4_moof(&moof_payload).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].sequence_number, 7);
        assert_eq!(runs[0].base_media_decode_time, 4096);
        assert_eq!(runs[0].sample_durations, vec![1024, 1024]);
        assert_eq!(runs[0].sample_sizes, vec![512, 512]);
    }
    #[test]
    fn unsigned_and_signed_composition_offsets() {
        for (version, expected) in [(0, 4_294_967_295i64), (0x0100_0000, -1)] {
            let mut tables = tables();
            tables.push((*b"ctts", words(&[version, 1, 2, u32::MAX])));
            let data = file(tables, 1000);
            let mut demux = Mp4Demuxer::new(&data);
            demux.parse_moov().unwrap();
            assert_eq!(demux.get_sample(0, 0).unwrap().pts.unwrap().pts, expected);
        }
    }
    #[test]
    fn missing_truncated_duplicate_and_oversized_tables() {
        for i in 0..5 {
            let mut t = tables();
            t.remove(i);
            assert!(!accepted(&file(t, 1000)));
            let mut t = tables();
            t[i].1.truncate(4);
            assert!(!accepted(&file(t, 1000)));
            let mut t = tables();
            t.push(t[i].clone());
            assert!(!accepted(&file(t, 1000)));
        }
        for size in [0, 1] {
            let mut t = tables();
            t[3].1 = words(&[0, size, u32::MAX]);
            assert!(!accepted(&file(t, 1000)));
        }
        let mut t = tables();
        t[0].1[4..8].copy_from_slice(&99u32.to_be_bytes());
        assert!(!accepted(&file(t, 1000)));
    }
    #[test]
    fn stsc_order_descriptions_and_coverage() {
        for values in [
            vec![0, 1, 0, 2, 1],
            vec![0, 1, 2, 2, 1],
            vec![0, 1, 1, 0, 1],
            vec![0, 1, 1, 1, 1],
            vec![0, 1, 1, 3, 1],
            vec![0, 1, 1, 2, 0],
            vec![0, 1, 1, 2, 2],
            vec![0, 2, 1, 1, 1, 1, 1, 1],
            vec![0, 2, 1, 1, 1, 3, 1, 1],
        ] {
            let mut t = tables();
            t[2].1 = words(&values);
            assert!(!accepted(&file(t, 1000)), "{values:?}");
        }
    }
    #[test]
    fn timing_locations_and_sync_validation() {
        assert!(!accepted(&file(tables(), 0)));
        for count in [0, 1, 3, u32::MAX] {
            let mut t = tables();
            t[1].1 = words(&[0, 1, count, 100]);
            assert!(!accepted(&file(t, 1000)));
            let mut t = tables();
            t.push((*b"ctts", words(&[0, 1, count, 1])));
            assert!(!accepted(&file(t, 1000)));
        }
        for offset in [0, 7, 9, u32::MAX] {
            let mut t = tables();
            t[4].1 = words(&[0, 1, offset]);
            assert!(!accepted(&file(t, 1000)));
        }
        let mut t = tables();
        t[4] = (
            *b"co64",
            [words(&[0, 1]), u64::MAX.to_be_bytes().to_vec()].concat(),
        );
        assert!(!accepted(&file(t, 1000)));
        for entries in [vec![0, 1, 0], vec![0, 1, 3], vec![0, 2, 1, 1]] {
            let mut t = tables();
            t.push((*b"stss", words(&entries)));
            assert!(!accepted(&file(t, 1000)));
        }
        let mut data = file(tables(), 1000);
        data.push(0);
        assert!(!accepted(&data));
    }
    #[test]
    fn empty_ctts_rejected_regardless_of_order() {
        for at in [0, 5] {
            let mut t = tables();
            t.insert(at, (*b"ctts", words(&[0, 0])));
            assert!(!accepted(&file(t, 1000)));
        }
    }
    #[test]
    fn hostile_inputs_never_panic() {
        let seed = file(tables(), 1000);
        for end in 0..seed.len() {
            let _ = Mp4Demuxer::new(&seed[..end]).parse_moov();
        }
        for at in 0..seed.len() {
            let mut bytes = seed.clone();
            bytes[at] ^= 0xff;
            let mut demux = Mp4Demuxer::new(&bytes);
            if demux.parse_moov().is_ok() {
                let _ = demux.get_sample(0, 0);
            }
        }
    }

    #[test]
    fn malformed_mdhd_and_description_boundaries() {
        let original = file(tables(), 1000);
        let mdhd = original.windows(4).position(|w| w == b"mdhd").unwrap();
        let mut invalid = original.clone();
        invalid[mdhd + 4] = 2;
        assert!(!accepted(&invalid));
        let mut invalid = original.clone();
        invalid[mdhd - 4..mdhd].copy_from_slice(&20u32.to_be_bytes());
        assert!(!accepted(&invalid));
        let mut invalid = original;
        invalid[mdhd..mdhd + 4].copy_from_slice(b"free");
        assert!(!accepted(&invalid));
        for size in [7, 15, 85, u32::MAX] {
            let mut t = tables();
            t[0].1[8..12].copy_from_slice(&size.to_be_bytes());
            assert!(!accepted(&file(t, 1000)));
        }
        let mut t = tables();
        t.push((*b"ctts", words(&[0x0200_0000, 1, 2, 0])));
        assert!(!accepted(&file(t, 1000)));
    }
}
