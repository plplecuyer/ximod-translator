//! Native Windows Media Audio (xWMA / WMAv2) decoding via **Media Foundation** —
//! Windows only.
//!
//! Approach (first version): rebuild the xWMA into a standard `RIFF/WAVE` whose
//! `fmt ` chunk declares WMAv2 (tag `0x0161`) plus the codec `extradata` flags the
//! xWMA container omits, then let a Media Foundation **Source Reader** decode it
//! to PCM. Windows' own WMA decoder handles the variable-block Skyrim voices that
//! ffmpeg's `wmav2` mis-decodes, giving full-quality audio.
//!
//! It runs on a dedicated **MTA** thread so it never disturbs the GUI thread's COM
//! apartment. Any failure returns `Err`, and [`super::decode_wma`] falls back to
//! the cross-platform ffmpeg path.
//!
//! STATUS: compiles cleanly on Windows (windows-crate 0.58). Reached only as the
//! rescue for streams the external converter can't decode (variable-block Skyrim
//! voices). The reconstructed WAVEFORMATEX restores the true bitrate and sets the
//! WMAv2 encode-options flags (incl. variable-block); if the Source-Reader-on-WAV
//! route still misbehaves, the next iteration will feed the WMA decoder MFT
//! (`CLSID_CWMADecMediaObject`) directly.

use std::os::windows::ffi::OsStrExt;

use windows::Win32::Media::MediaFoundation::{
    IMFMediaType, IMFSample, IMFSourceReader, MF_MT_AUDIO_BITS_PER_SAMPLE,
    MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE,
    MF_SOURCE_READER_ALL_STREAMS, MF_SOURCE_READER_FIRST_AUDIO_STREAM,
    MF_SOURCE_READERF_ENDOFSTREAM, MFAudioFormat_PCM, MFCreateMediaType,
    MFCreateSourceReaderFromURL, MFMediaType_Audio, MFShutdown, MFStartup,
};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
use windows::core::PCWSTR;

/// `MF_VERSION` = (MF_SDK_VERSION << 16) | MF_API_VERSION = (0x0002 << 16) | 0x0070.
const MF_VERSION: u32 = 0x0002_0070;
/// `MFSTARTUP_FULL` == 0.
const MFSTARTUP_FULL_FLAG: u32 = 0;

/// Decode a raw xWMA (`RIFF/XWMA`) byte stream to a 16-bit PCM WAV.
pub fn decode(xwma: &[u8]) -> Result<Vec<u8>, String> {
    // Isolate COM apartment (MTA) from the GUI thread.
    let data = xwma.to_vec();
    std::thread::Builder::new()
        .name("mf-wma-decode".into())
        .spawn(move || decode_mta(&data))
        .map_err(|e| format!("thread MF : {e}"))?
        .join()
        .map_err(|_| "le thread de décodage MF a paniqué".to_string())?
}

fn decode_mta(xwma: &[u8]) -> Result<Vec<u8>, String> {
    // 0x0007 = exp_vlc | bit_reservoir | variable_block. This path only receives
    // streams the external converter couldn't decode — in practice the
    // variable-block Skyrim voices — so the variable-block bit (0x04) must be set.
    let wave =
        build_wave_with_extradata(xwma, 0x0007).ok_or_else(|| "flux xWMA illisible".to_string())?;

    unsafe {
        // Multi-threaded apartment; ignore "already initialised" outcomes.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        MFStartup(MF_VERSION, MFSTARTUP_FULL_FLAG).map_err(|e| format!("MFStartup : {e}"))?;
    }

    let result = unsafe { read_all(&wave) };

    unsafe {
        let _ = MFShutdown();
        CoUninitialize();
    }
    result
}

unsafe fn read_all(wave: &[u8]) -> Result<Vec<u8>, String> {
    // Media Foundation reads from a URL/file; write the rebuilt WAV to a temp file.
    let mut path = std::env::temp_dir();
    path.push(format!("bsae_mf_{}.wav", std::process::id()));
    std::fs::write(&path, wave).map_err(|e| format!("écriture temporaire : {e}"))?;
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    // These stream selectors are newtype wrappers (`MF_SOURCE_READER_CONSTANTS`)
    // around `u32`; the reader methods take a plain `u32` stream index.
    let all_streams: u32 = MF_SOURCE_READER_ALL_STREAMS.0 as u32;
    let first_audio: u32 = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;

    // Rust 2024 requires each FFI/raw-pointer op to sit in an explicit `unsafe`
    // block even inside an `unsafe fn`; wrap the whole closure body once.
    let decode = || -> windows::core::Result<(u16, u32, Vec<u8>)> {
        unsafe {
            let reader: IMFSourceReader = MFCreateSourceReaderFromURL(PCWSTR(wide.as_ptr()), None)?;

            // Select only the first audio stream.
            reader.SetStreamSelection(all_streams, false)?;
            reader.SetStreamSelection(first_audio, true)?;

            // Ask for uncompressed 16-bit PCM output; MF fills in channels/rate.
            let pcm: IMFMediaType = MFCreateMediaType()?;
            pcm.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
            pcm.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
            pcm.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
            reader.SetCurrentMediaType(first_audio, None, &pcm)?;

            // Read back the negotiated output format (channels / sample rate).
            let out_type = reader.GetCurrentMediaType(first_audio)?;
            let channels = out_type.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS)? as u16;
            let sample_rate = out_type.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)?;

            // Decode loop.
            let mut pcm_bytes: Vec<u8> = Vec::new();
            loop {
                let mut flags: u32 = 0;
                let mut sample: Option<IMFSample> = None;
                reader.ReadSample(
                    first_audio,
                    0,
                    None,
                    Some(&mut flags),
                    None,
                    Some(&mut sample),
                )?;
                if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                    break;
                }
                let Some(sample) = sample else {
                    continue; // gap/stream tick with no data
                };
                let buffer = sample.ConvertToContiguousBuffer()?;
                let mut ptr: *mut u8 = std::ptr::null_mut();
                let mut len: u32 = 0;
                buffer.Lock(&mut ptr, None, Some(&mut len))?;
                if !ptr.is_null() && len > 0 {
                    pcm_bytes.extend_from_slice(std::slice::from_raw_parts(ptr, len as usize));
                }
                buffer.Unlock()?;
            }
            Ok((channels.max(1), sample_rate, pcm_bytes))
        }
    };

    let out = decode().map_err(|e| format!("Media Foundation : {e}"));
    let _ = std::fs::remove_file(&path);
    let (channels, sample_rate, pcm) = out?;
    if pcm.is_empty() {
        return Err("Media Foundation n'a produit aucun son".into());
    }
    Ok(wrap_pcm16_wav(channels, sample_rate, &pcm))
}

/// Rebuild an xWMA (`RIFF/XWMA`) into a standard `RIFF/WAVE` with a WMAv2 `fmt `
/// chunk carrying a 10-byte extradata (`flags1=0, flags2, 0, 0`). Returns `None`
/// if the input isn't a parseable xWMA.
fn build_wave_with_extradata(xwma: &[u8], flags2: u16) -> Option<Vec<u8>> {
    if xwma.len() < 12 || &xwma[0..4] != b"RIFF" || &xwma[8..12] != b"XWMA" {
        return None;
    }
    let mut fmt: Option<&[u8]> = None;
    let mut data: Option<&[u8]> = None;
    let mut i = 12;
    while i + 8 <= xwma.len() {
        let id = &xwma[i..i + 4];
        let sz = u32::from_le_bytes([xwma[i + 4], xwma[i + 5], xwma[i + 6], xwma[i + 7]]) as usize;
        let start = i + 8;
        let end = (start + sz).min(xwma.len());
        match id {
            b"fmt " => fmt = Some(&xwma[start..end]),
            b"data" => data = Some(&xwma[start..end]),
            _ => {}
        }
        i = start + sz + (sz & 1);
    }
    let fmt = fmt?;
    let data = data?;
    if fmt.len() < 16 {
        return None;
    }
    // Normalize the average-bytes-per-second field (offset 8..12): MS's xWMA
    // encoder writes a fake bitrate for some rates and Media Foundation's WMA
    // decoder garbles the audio unless the true bitrate is restored.
    let channels = u16::from_le_bytes([fmt[2], fmt[3]]);
    let sample_rate = u32::from_le_bytes([fmt[4], fmt[5], fmt[6], fmt[7]]);
    let avg_bps = u32::from_le_bytes([fmt[8], fmt[9], fmt[10], fmt[11]]);
    let avg_bps = super::normalize_xwma_avg_bps(channels, sample_rate, avg_bps);
    let mut base = fmt[..16].to_vec();
    base[8..12].copy_from_slice(&avg_bps.to_le_bytes());

    // WAVEFORMATEX base (16 B) + cbSize=10 + 10-byte WMAv2 codec data:
    // dwSamplesPerBlock(4)=0, wEncodeOptions(2)=flags2, reserved(4)=0.
    let mut new_fmt = Vec::with_capacity(28);
    new_fmt.extend_from_slice(&base);
    new_fmt.extend_from_slice(&10u16.to_le_bytes()); // cbSize
    new_fmt.extend_from_slice(&0u32.to_le_bytes()); // dwSamplesPerBlock
    new_fmt.extend_from_slice(&flags2.to_le_bytes()); // wEncodeOptions
    new_fmt.extend_from_slice(&0u16.to_le_bytes());
    new_fmt.extend_from_slice(&0u16.to_le_bytes());

    let mut body = Vec::new();
    body.extend_from_slice(b"fmt ");
    body.extend_from_slice(&(new_fmt.len() as u32).to_le_bytes());
    body.extend_from_slice(&new_fmt);
    body.extend_from_slice(b"data");
    body.extend_from_slice(&(data.len() as u32).to_le_bytes());
    body.extend_from_slice(data);

    let mut out = Vec::with_capacity(12 + body.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((4 + body.len()) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(&body);
    Some(out)
}

/// Wrap raw interleaved 16-bit PCM into a WAV container.
fn wrap_pcm16_wav(channels: u16, sample_rate: u32, pcm: &[u8]) -> Vec<u8> {
    let bits = 16u16;
    let block_align = channels * (bits / 8);
    let byte_rate = sample_rate * block_align as u32;
    let data_len = pcm.len() as u32;
    let mut v = Vec::with_capacity(44 + pcm.len());
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data_len).to_le_bytes());
    v.extend_from_slice(b"WAVE");
    v.extend_from_slice(b"fmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes()); // PCM
    v.extend_from_slice(&channels.to_le_bytes());
    v.extend_from_slice(&sample_rate.to_le_bytes());
    v.extend_from_slice(&byte_rate.to_le_bytes());
    v.extend_from_slice(&block_align.to_le_bytes());
    v.extend_from_slice(&bits.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data_len.to_le_bytes());
    v.extend_from_slice(pcm);
    v
}
