#[derive(Debug, Clone)]
pub struct StreamInfo {
    pub stream_id: u32,
    pub codec_id: crate::decoder::CodecId,
    pub width: u32,
    pub height: u32,
    pub bit_rate: Option<u64>,
    pub extradata: Vec<u8>,
}
