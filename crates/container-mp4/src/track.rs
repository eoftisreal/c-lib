use codec_core::StreamInfo;

#[derive(Debug, Default, Clone)]
pub struct Track {
    pub stream_info: Option<StreamInfo>,
    pub track_id: u32,

    // Sample tables
    pub stsz: Vec<u32>,         // sizes
    pub stco: Vec<u64>,         // chunk offsets
    pub stsc: Vec<StscEntry>,   // sample to chunk
    pub stts: Vec<SttsEntry>,   // time to sample
    pub ctts: Vec<CttsEntry>,   // composition offset
    pub stss: Option<Vec<u32>>, // sync samples
    pub elst: Vec<ElstEntry>,   // edit-list timeline composition entries

    /// Cumulative DTS prefix table for O(log N) binary-search seeking.
    pub cumulative_dts: Vec<i64>,

    pub timescale: u32,
    pub movie_timescale: u32,
    pub sample_description_count: u32,
    pub selected_description_index: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StscEntry {
    pub first_chunk: u32,
    pub samples_per_chunk: u32,
    pub sample_description_index: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SttsEntry {
    pub sample_count: u32,
    pub sample_delta: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CttsEntry {
    pub sample_count: u32,
    pub sample_offset: i64, // preserves unsigned v0 and signed v1
}

/// ISOBMFF Edit List (`elst`) entry mapping media timeline segments into movie presentation time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElstEntry {
    pub segment_duration: u64,
    pub media_time: i64,
    pub media_rate_integer: i16,
    pub media_rate_fraction: i16,
}

/// Parsed Fragmented MP4 (`fMP4` / CMAF) `moof` + `traf` + `trun` track fragment run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FragmentRun {
    pub sequence_number: u32,
    pub track_id: u32,
    pub base_media_decode_time: u64,
    pub sample_durations: Vec<u32>,
    pub sample_sizes: Vec<u32>,
    pub sample_composition_offsets: Vec<i64>,
    pub data_offset: Option<i32>,
}
