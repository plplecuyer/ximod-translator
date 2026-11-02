//! Audio decoding for the preview pane: turn a game voice/sound file into a
//! playable **WAV** by shelling out to an external decoder.
//!
//! These formats have no practical pure-Rust decoder:
//! - `.wem` — Wwise Encoded Media. Voices/SFX in Fallout 4 & Starfield. Usually
//!   **Wwise Vorbis** (fmt tag `0xFFFF`), which needs codebook reconstruction —
//!   only **vgmstream** handles it (ffmpeg cannot).
//! - `.xwm` — xWMA (RIFF/WMA). Voice audio in Skyrim / Fallout 3-4. Decoded by
//!   **ffmpeg** (wmav2) or vgmstream.
//! - `.fuz` — Bethesda container = `FUZE` header + optional LIP lip-sync block +
//!   an embedded `.xwm` stream. We strip the header and decode the `.xwm`.
//!
//! Strategy: prefer `vgmstream-cli` (covers everything), else fall back to
//! `ffmpeg` (covers `.xwm`/`.fuz`). Both are looked up next to our own
//! executable and on `PATH`. Nothing is bundled; if neither is present the UI
//! explains what to install.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// Native Windows Media Audio decoder (Media Foundation Source Reader). Compiled
/// only on Windows; used as the rescue for variable-block Skyrim xWMA that the
/// external converter renders as silence. Any failure falls back to the
/// cross-platform ffmpeg extradata path.
#[cfg(windows)]
mod wma_win;

/// The external decoder we found, if any.
#[derive(Clone, Debug)]
pub enum Converter {
    /// vgmstream-cli — decodes every format (incl. Wwise Vorbis `.wem`).
    Vgmstream(PathBuf),
    /// ffmpeg — decodes `.xwm`/`.fuz` only (WMA), not Wwise Vorbis `.wem`.
    Ffmpeg(PathBuf),
    /// xWMAEncode — Microsoft's own xWMA tool. The only decoder that correctly
    /// handles **every** xWMA, including the Skyrim LE 32 kHz voices that ffmpeg/
    /// vgmstream cannot. Decodes `.xwm` → `.wav` (extension-driven). `.xwm`/`.fuz`
    /// only.
    XwmaEncode(PathBuf),
}

impl Converter {
    pub fn label(&self) -> &'static str {
        match self {
            Converter::Vgmstream(_) => "vgmstream",
            Converter::Ffmpeg(_) => "ffmpeg",
            Converter::XwmaEncode(_) => "xWMAEncode",
        }
    }
}

/// Formats rodio decodes directly (no external tool, no conversion).
pub fn is_native_ext(ext: &str) -> bool {
    matches!(ext, "ogg" | "wav" | "flac" | "mp3")
}

/// Audio extensions we can preview: rodio-native ones, plus the game-specific
/// containers we decode ourselves (`.wem`) or via an external tool (`.xwm`/`.fuz`).
pub fn is_audio_ext(ext: &str) -> bool {
    is_native_ext(ext) || matches!(ext, "wem" | "xwm" | "fuz")
}

/// Whether an entry can be played without any external decoder: rodio-native
/// formats and `.wem` (built-in Wwise Vorbis).
pub fn plays_without_external(ext: &str) -> bool {
    is_native_ext(ext) || ext == "wem"
}

/// Produce playable bytes for an audio entry:
/// - rodio-native (`.ogg`/`.wav`/`.flac`/`.mp3`): returned as-is (rodio decodes them);
/// - `.wem` (Wwise Vorbis): decoded **in pure Rust** to WAV, else the external tool;
/// - `.xwm`/`.fuz`: decoded to WAV via the external `converter` (vgmstream/ffmpeg).
pub fn decode(
    entry_path: &str,
    bytes: &[u8],
    converter: Option<&Converter>,
) -> Result<Vec<u8>, String> {
    let ext = ext_of(entry_path);
    if is_native_ext(&ext) {
        // rodio reads these containers directly.
        return Ok(bytes.to_vec());
    }
    if ext == "wem" {
        // Try the built-in Wwise Vorbis path first.
        match decode_wem_pure(bytes) {
            Ok(wav) => return Ok(wav),
            Err(rust_err) => {
                // Fall back to an external decoder for non-Vorbis .wem (Opus/XMA…).
                return match converter {
                    Some(conv) => decode_to_wav(entry_path, bytes, conv),
                    None => Err(format!(
                        "{rust_err}. Ce .wem n'est pas du Wwise Vorbis (Opus/XMA ?) : \
                         installez vgmstream-cli pour l'écouter."
                    )),
                };
            }
        }
    }
    if ext == "xwm" || ext == "fuz" {
        let xwma = if ext == "fuz" {
            strip_fuz(bytes)?
        } else {
            bytes.to_vec()
        };
        return decode_wma(&xwma, converter);
    }
    match converter {
        Some(conv) => decode_to_wav(entry_path, bytes, conv),
        None => Err(
            "Aucun décodeur pour ce format : installez ffmpeg (.xwm/.fuz) ou vgmstream-cli.".into(),
        ),
    }
}

/// Decode a raw **xWMA** stream to WAV, choosing the best available decoder.
///
/// Order (each attempt accepted only if it yields real, non-silent audio):
/// 0. **xWMAEncode** (Microsoft's own tool) if present next to the app or on PATH —
///    the only decoder that correctly handles **every** xWMA, including the Skyrim
///    LE 32 kHz voices that ffmpeg/vgmstream render as garble/silence. Preferred;
/// 1. the external `converter` (vgmstream/ffmpeg) on the **raw** stream — clean,
///    full quality for the common files (Fallout 3/4, Skyrim SE). These decode
///    correctly as-is, so they must NOT be intercepted by a rescue path;
/// 2. **(Windows only)** native Media Foundation WMA decoder — a further attempt
///    for streams the external converter couldn't handle;
/// 3. **extradata rescue** via ffmpeg — last resort: rebuild the stream with
///    reconstructed WMAv2 codec data (normalized bitrate + encode flags).
///    Partial/imperfect for the hardest variable-block files.
///
/// Note: the Skyrim LE (2011) 32 kHz voices are decodable *correctly* only by
/// Microsoft's own xWMA decoder — the one Skyrim uses in-game. ffmpeg's wmav2
/// (hence vgmstream) and the standard Windows WMA decoder all mis-decode them,
/// so step 0 (xWMAEncode) is what makes those files play.
pub fn decode_wma(xwma: &[u8], converter: Option<&Converter>) -> Result<Vec<u8>, String> {
    // 0. xWMAEncode — correct for every xWMA (incl. Skyrim LE). Looked up next to
    //    the executable (or a sub-folder) and on PATH, like the other tools.
    if let Some(bin) = find_program("xWMAEncode")
        && let Ok(wav) = run_and_read(&Converter::XwmaEncode(bin), "xwm", xwma.to_vec())
    {
        return Ok(wav);
    }

    // 1. Raw stream through the external converter (clean for FO3/FO4/Skyrim SE).
    //    For the hard Skyrim LE voices this yields silence/an error and is rejected.
    if let Some(conv) = converter
        && let Ok(wav) = run_and_read(conv, "xwm", xwma.to_vec())
    {
        return Ok(wav);
    }

    // 2. Native Windows path — see `wma_win::decode` (Media Foundation). Compiled
    //    only on Windows; reached only for streams the converter couldn't handle.
    #[cfg(windows)]
    if let Ok(wav) = wma_win::decode(xwma)
        && wav_has_audio(&wav)
        && !wav_is_silent(&wav)
    {
        return Ok(wav);
    }

    // 3. Extradata rescue — needs ffmpeg specifically (the fix targets ffmpeg's
    //    wmav2 decoder). Rebuild the xWMA with normalized WMAv2 codec data.
    if let Some(ffmpeg) = find_ffmpeg()
        && let Some(fixed) = build_xwma_with_extradata(xwma, 0x0007)
        && let Ok(wav) = run_and_read(&Converter::Ffmpeg(ffmpeg), "xwm", fixed)
    {
        return Ok(wav);
    }

    Err(
        "Impossible de décoder ce fichier xWMA. Les voix Skyrim LE (32 kHz) ne sont décodables \
         correctement que par l'outil Microsoft xWMAEncode : placez xWMAEncode.exe à côté de \
         l'exécutable (ou dans le PATH). ffmpeg/vgmstream suffisent pour Fallout 3/4 et Skyrim SE."
            .into(),
    )
}

/// Restore the true WMAv2 average bytes-per-second for an xWMA stream. MS's
/// xWMA encoder writes *fake* bitrates for a handful of channel / sample-rate
/// combinations (identical codec data, wrong `nAvgBytesPerSec`); a real WMA
/// decoder produces **garbled audio** unless the genuine bitrate is restored.
/// Mirrors vgmstream's `ffmpeg_make_riff_xwma` normalization table.
pub(crate) fn normalize_xwma_avg_bps(channels: u16, sample_rate: u32, avg_bps: u32) -> u32 {
    let mut br = avg_bps.saturating_mul(8);
    match channels {
        1 => match sample_rate {
            22050 if br == 48000 || br == 192000 => br = 20000,
            32000 if br == 48000 || br == 192000 => br = 20000,
            44100 if br == 96000 || br == 192000 => br = 48000,
            _ => {}
        },
        2 => match sample_rate {
            22050 if br == 48000 || br == 192000 => br = 32000,
            32000 if br == 192000 => br = 48000,
            _ => {}
        },
        _ => {}
    }
    br / 8
}

/// Locate an `ffmpeg` binary for the extradata rescue (next to the exe or on PATH).
fn find_ffmpeg() -> Option<PathBuf> {
    find_program("ffmpeg")
}

/// Rebuild an xWMA (`RIFF/XWMA`) stream with a WMAv2 `extradata` block carrying
/// the codec `flags2` that the container omits — what ffmpeg's wmav2 decoder needs
/// for variable-block Skyrim voices. Keeps the `dpds` seek table and audio data.
/// Returns `None` if the input isn't a parseable xWMA.
fn build_xwma_with_extradata(xwma: &[u8], flags2: u16) -> Option<Vec<u8>> {
    if xwma.len() < 12 || &xwma[0..4] != b"RIFF" || &xwma[8..12] != b"XWMA" {
        return None;
    }
    let mut fmt: Option<&[u8]> = None;
    let mut dpds: Option<&[u8]> = None;
    let mut data: Option<&[u8]> = None;
    let mut i = 12;
    while i + 8 <= xwma.len() {
        let id = &xwma[i..i + 4];
        let sz = u32::from_le_bytes([xwma[i + 4], xwma[i + 5], xwma[i + 6], xwma[i + 7]]) as usize;
        let start = i + 8;
        let end = (start + sz).min(xwma.len());
        match id {
            b"fmt " => fmt = Some(&xwma[start..end]),
            b"dpds" => dpds = Some(&xwma[start..end]),
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
    // WAVEFORMATEX base (16 bytes), with the average-bytes-per-second field
    // (offset 8..12) normalized to the true bitrate — MS's xWMA encoder writes a
    // fake value for some rates and a real WMA decoder garbles the audio otherwise.
    let channels = u16::from_le_bytes([fmt[2], fmt[3]]);
    let sample_rate = u32::from_le_bytes([fmt[4], fmt[5], fmt[6], fmt[7]]);
    let avg_bps = u32::from_le_bytes([fmt[8], fmt[9], fmt[10], fmt[11]]);
    let avg_bps = normalize_xwma_avg_bps(channels, sample_rate, avg_bps);
    let mut base = fmt[..16].to_vec();
    base[8..12].copy_from_slice(&avg_bps.to_le_bytes());
    // WAVEFORMATEX base (16 bytes) + cbSize(2)=10 + extradata(10): flags1=0, flags2, 0, 0.
    let mut new_fmt = Vec::with_capacity(28);
    new_fmt.extend_from_slice(&base);
    new_fmt.extend_from_slice(&10u16.to_le_bytes());
    new_fmt.extend_from_slice(&0u32.to_le_bytes()); // flags1
    new_fmt.extend_from_slice(&flags2.to_le_bytes());
    new_fmt.extend_from_slice(&0u16.to_le_bytes());
    new_fmt.extend_from_slice(&0u16.to_le_bytes());

    let mut body = Vec::new();
    let push_chunk = |body: &mut Vec<u8>, id: &[u8; 4], payload: &[u8]| {
        body.extend_from_slice(id);
        body.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        body.extend_from_slice(payload);
        if payload.len() & 1 == 1 {
            body.push(0);
        }
    };
    push_chunk(&mut body, b"fmt ", &new_fmt);
    if let Some(dp) = dpds {
        push_chunk(&mut body, b"dpds", dp);
    }
    push_chunk(&mut body, b"data", data);

    let mut out = Vec::with_capacity(12 + body.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((4 + body.len()) as u32).to_le_bytes());
    out.extend_from_slice(b"XWMA");
    out.extend_from_slice(&body);
    Some(out)
}

/// Run a converter on in-memory bytes and return the WAV **only if it carries
/// real, non-silent audio**. Used by every decode path so we never accept an
/// empty or silent result as success.
fn run_and_read(conv: &Converter, in_ext: &str, in_bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    let dir = std::env::temp_dir().join("bsa-explorer-audio");
    std::fs::create_dir_all(&dir).map_err(|e| format!("dossier temporaire : {e}"))?;
    let uid = next_uid();
    let in_path = dir.join(format!("in_{uid}.{in_ext}"));
    let out_path = dir.join(format!("out_{uid}.wav"));
    std::fs::write(&in_path, &in_bytes).map_err(|e| format!("écriture temporaire : {e}"))?;
    let conv_result = run_converter(conv, &in_path, &out_path);
    let _ = std::fs::remove_file(&in_path);
    let wav = std::fs::read(&out_path).unwrap_or_default();
    let _ = std::fs::remove_file(&out_path);
    if wav_has_audio(&wav) && !wav_is_silent(&wav) {
        return Ok(wav);
    }
    Err(match conv_result {
        Err(e) => e,
        Ok(()) => "sortie vide ou muette".to_string(),
    })
}

/// Decode a Wwise Vorbis `.wem` to WAV in pure Rust: rebuild a standard Ogg with
/// [`ww2ogg`] (bundled codebooks), then decode it with [`lewton`].
///
/// Wwise `.wem` come in a few flavours (external standard / external aoTuV /
/// inline codebooks). We try each and keep the first whose Ogg lewton actually
/// decodes — a structurally-valid Ogg from the wrong codebook set still fails at
/// Vorbis header decode, so full decode is the real validation.
pub fn decode_wem_pure(bytes: &[u8]) -> Result<Vec<u8>, String> {
    // (label, aoTuV codebooks?, inline codebooks?)
    let configs = [
        ("standard", false, false),
        ("aoTuV", true, false),
        ("inline", false, true),
    ];
    let mut errs = Vec::new();
    for (label, aotuv, inline) in configs {
        match wem_to_ogg(bytes, aotuv, inline).and_then(|ogg| ogg_to_wav(&ogg)) {
            Ok(wav) => return Ok(wav),
            Err(e) => errs.push(format!("{label}: {e}")),
        }
    }
    Err(format!("Wwise Vorbis non décodé ({})", errs.join(" ; ")))
}

fn wem_to_ogg(bytes: &[u8], aotuv: bool, inline: bool) -> Result<Vec<u8>, String> {
    use ww2ogg::{CodebookLibrary, WwiseRiffVorbis};
    let cb = if aotuv {
        CodebookLibrary::aotuv_codebooks()
    } else {
        CodebookLibrary::default_codebooks()
    }
    .map_err(|e| e.to_string())?;
    let mut conv = WwiseRiffVorbis::builder(std::io::Cursor::new(bytes), cb)
        .inline_codebooks(inline)
        .build()
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    conv.generate_ogg(&mut out).map_err(|e| e.to_string())?;
    Ok(out)
}

fn ogg_to_wav(ogg: &[u8]) -> Result<Vec<u8>, String> {
    use lewton::inside_ogg::OggStreamReader;
    let mut reader = OggStreamReader::new(std::io::Cursor::new(ogg))
        .map_err(|e| format!("Ogg illisible : {e}"))?;
    let channels = reader.ident_hdr.audio_channels as u16;
    let sample_rate = reader.ident_hdr.audio_sample_rate;
    let mut samples: Vec<i16> = Vec::new();
    while let Some(pck) = reader
        .read_dec_packet_itl()
        .map_err(|e| format!("décodage Vorbis : {e}"))?
    {
        samples.extend_from_slice(&pck);
    }
    if samples.is_empty() {
        return Err("flux Vorbis vide".into());
    }
    Ok(build_wav(channels.max(1), sample_rate, &samples))
}

/// Build a 16-bit PCM WAV from interleaved samples.
fn build_wav(channels: u16, sample_rate: u32, samples: &[i16]) -> Vec<u8> {
    let bits = 16u16;
    let block_align = channels * (bits / 8);
    let byte_rate = sample_rate * block_align as u32;
    let data_len = (samples.len() * 2) as u32;
    let mut v = Vec::with_capacity(44 + data_len as usize);
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
    for s in samples {
        v.extend_from_slice(&s.to_le_bytes());
    }
    v
}

/// Locate a decoder: prefer vgmstream (full coverage), else ffmpeg. Looks next to
/// our own executable first (a bundled copy), then on `PATH`.
pub fn detect() -> Option<Converter> {
    for name in ["vgmstream-cli", "vgmstream_cli"] {
        if let Some(p) = find_program(name) {
            return Some(Converter::Vgmstream(p));
        }
    }
    if let Some(p) = find_program("ffmpeg") {
        return Some(Converter::Ffmpeg(p));
    }
    None
}

/// Whether `xWMAEncode` is reachable (next to the executable or on `PATH`). It
/// decodes every xWMA — including the Skyrim LE 32 kHz voices no other tool
/// handles — so its presence alone makes `.xwm`/`.fuz` previewable.
pub fn has_xwmaencode() -> bool {
    find_program("xWMAEncode").is_some()
}

/// Resolve a program: an executable sitting next to our binary (or in an
/// immediate sub-folder, e.g. an extracted `vgmstream-win64/` next to the exe),
/// otherwise the bare name if it runs from `PATH`.
fn find_program(name: &str) -> Option<PathBuf> {
    let exe_names = [name.to_string(), format!("{name}.exe")];
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        // Directly next to the executable.
        for n in &exe_names {
            let cand = dir.join(n);
            if cand.is_file() {
                return Some(cand);
            }
        }
        // One level down (a dropped-in tool folder).
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let sub = entry.path();
                if sub.is_dir() {
                    for n in &exe_names {
                        let cand = sub.join(n);
                        if cand.is_file() {
                            return Some(cand);
                        }
                    }
                }
            }
        }
    }
    // On PATH: a benign spawn that succeeds means the binary exists.
    for n in &exe_names {
        if Command::new(n)
            .arg("-h")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok()
        {
            return Some(PathBuf::from(n));
        }
    }
    None
}

/// Decode a non-Vorbis `.wem` (Opus/XMA…) to WAV via the external `conv`. Wwise
/// Vorbis `.wem` are handled in pure Rust upstream; `.xwm`/`.fuz` go through
/// [`decode_wma`]. `entry_path` supplies the extension.
pub fn decode_to_wav(entry_path: &str, bytes: &[u8], conv: &Converter) -> Result<Vec<u8>, String> {
    let ext = ext_of(entry_path);
    if matches!(conv, Converter::Ffmpeg(_)) && ext == "wem" {
        return Err(
            "Les .wem (Wwise Vorbis) ne sont pas décodables par ffmpeg. \
             Installez vgmstream-cli (à côté de l'exécutable ou dans le PATH) pour les écouter."
                .into(),
        );
    }
    run_and_read(conv, &ext, bytes.to_vec())
}

/// Whether a WAV buffer actually carries audio (a non-empty `data` chunk).
fn wav_has_audio(wav: &[u8]) -> bool {
    wav_data_chunk(wav).map(|(_, sz)| sz > 0).unwrap_or(false)
}

/// Locate the `data` chunk: returns (offset of payload, declared size).
fn wav_data_chunk(wav: &[u8]) -> Option<(usize, usize)> {
    if wav.len() < 12 || &wav[0..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return None;
    }
    let mut i = 12;
    while i + 8 <= wav.len() {
        let id = &wav[i..i + 4];
        let sz = u32::from_le_bytes([wav[i + 4], wav[i + 5], wav[i + 6], wav[i + 7]]) as usize;
        if id == b"data" {
            return Some((i + 8, sz));
        }
        i += 8 + sz + (sz & 1);
    }
    None
}

/// Whether a 16-bit PCM WAV is essentially silent (peak below ~ -48 dBFS). Used
/// to catch decoders that emit full-length silence instead of failing. Returns
/// false for non-PCM16 or when it can't tell, to avoid false positives.
fn wav_is_silent(wav: &[u8]) -> bool {
    // Require PCM (fmt tag 1) 16-bit to interpret samples.
    let mut bits = 0u16;
    let mut tag = 0u16;
    let mut i = 12;
    while i + 8 <= wav.len() {
        let id = &wav[i..i + 4];
        let sz = u32::from_le_bytes([wav[i + 4], wav[i + 5], wav[i + 6], wav[i + 7]]) as usize;
        if id == b"fmt " && i + 8 + 16 <= wav.len() {
            tag = u16::from_le_bytes([wav[i + 8], wav[i + 9]]);
            bits = u16::from_le_bytes([wav[i + 22], wav[i + 23]]);
        }
        i += 8 + sz + (sz & 1);
    }
    if tag != 1 || bits != 16 {
        return false;
    }
    let Some((off, sz)) = wav_data_chunk(wav) else {
        return false;
    };
    let end = (off + sz).min(wav.len());
    if end <= off {
        return false;
    }
    let mut peak = 0i32;
    let mut j = off;
    while j + 1 < end {
        let s = i16::from_le_bytes([wav[j], wav[j + 1]]).unsigned_abs() as i32;
        if s > peak {
            peak = s;
            if peak >= 256 {
                return false; // clearly audible → not silent
            }
        }
        j += 2;
    }
    peak < 256
}

fn run_converter(conv: &Converter, input: &Path, output: &Path) -> Result<(), String> {
    let out = match conv {
        Converter::Vgmstream(bin) => Command::new(bin).arg("-o").arg(output).arg(input).output(),
        Converter::Ffmpeg(bin) => Command::new(bin)
            .arg("-nostdin")
            .arg("-y")
            .arg("-i")
            .arg(input)
            .arg(output)
            .output(),
        // xWMAEncode picks the direction from the file extensions: a `.xwm` input
        // and a `.wav` output means "decode". No other flags are needed.
        Converter::XwmaEncode(bin) => Command::new(bin).arg(input).arg(output).output(),
    };
    let out = out.map_err(|e| format!("lancement de {} : {e}", conv.label()))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let tail: String = err
            .lines()
            .rev()
            .take(3)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join(" | ");
        return Err(format!("{} a échoué : {tail}", conv.label()));
    }
    Ok(())
}

/// Unwrap a `.fuz` container to its embedded `.xwm` byte stream.
fn strip_fuz(bytes: &[u8]) -> Result<Vec<u8>, String> {
    // Layout: "FUZE" (4) | version u32 (4) | lip_size u32 (4) | lip[lip_size] | xwm...
    if bytes.len() < 12 || &bytes[0..4] != b"FUZE" {
        return Err("en-tête FUZE invalide".into());
    }
    let lip_size = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
    let start = 12usize
        .checked_add(lip_size)
        .filter(|&s| s <= bytes.len())
        .ok_or("taille LIP incohérente dans le .fuz")?;
    Ok(bytes[start..].to_vec())
}

fn ext_of(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rsplit_once('.') {
        Some((_, e)) => e.to_ascii_lowercase(),
        None => String::new(),
    }
}

/// Monotonic id so concurrent temp files never collide.
fn next_uid() -> u64 {
    static N: AtomicU64 = AtomicU64::new(0);
    let pid = std::process::id() as u64;
    (pid << 32) | N.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_audio_extensions() {
        for e in ["wem", "xwm", "fuz", "ogg", "wav", "flac", "mp3"] {
            assert!(is_audio_ext(e), "{e} should be audio");
        }
        assert!(!is_audio_ext("dds"));
        assert!(!is_audio_ext("nif"));
        // Native (rodio-decodable) formats play without any external tool.
        for e in ["ogg", "wav", "flac", "mp3", "wem"] {
            assert!(plays_without_external(e), "{e} should play built-in");
        }
        assert!(!plays_without_external("xwm"));
        assert!(!plays_without_external("fuz"));
    }

    #[test]
    fn native_formats_pass_through_unchanged() {
        // rodio-native containers are returned as-is (no conversion).
        let raw = b"OggS........fake".to_vec();
        assert_eq!(decode("voice/x.ogg", &raw, None).unwrap(), raw);
        let wav = b"RIFF....WAVEfmt ".to_vec();
        assert_eq!(decode("music/x.wav", &wav, None).unwrap(), wav);
    }

    #[test]
    fn strips_fuz_header() {
        // FUZE + version 1 + lip_size 4 + 4 lip bytes + "XWMDATA"
        let mut f = b"FUZE".to_vec();
        f.extend_from_slice(&1u32.to_le_bytes());
        f.extend_from_slice(&4u32.to_le_bytes());
        f.extend_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]); // lip
        f.extend_from_slice(b"XWMDATA");
        assert_eq!(strip_fuz(&f).unwrap(), b"XWMDATA");
    }

    #[test]
    fn rejects_bad_fuz() {
        assert!(strip_fuz(b"NOPE").is_err());
        // lip_size larger than the buffer.
        let mut f = b"FUZE".to_vec();
        f.extend_from_slice(&1u32.to_le_bytes());
        f.extend_from_slice(&999u32.to_le_bytes());
        assert!(strip_fuz(&f).is_err());
    }

    #[test]
    fn wav_has_audio_detects_empty() {
        // A header-only WAV (data size 0) — what ffmpeg emits when the decoder
        // rejects every packet — must be seen as "no audio".
        let mut empty = b"RIFF".to_vec();
        empty.extend_from_slice(&36u32.to_le_bytes());
        empty.extend_from_slice(b"WAVEfmt ");
        empty.extend_from_slice(&16u32.to_le_bytes());
        empty.extend_from_slice(&[1, 0, 1, 0]); // pcm, 1ch
        empty.extend_from_slice(&44100u32.to_le_bytes());
        empty.extend_from_slice(&88200u32.to_le_bytes());
        empty.extend_from_slice(&[2, 0, 16, 0]);
        empty.extend_from_slice(b"data");
        empty.extend_from_slice(&0u32.to_le_bytes());
        assert!(!wav_has_audio(&empty));

        let mut ok = empty.clone();
        // flip data size to 4 and append 4 bytes of PCM
        let dpos = ok.len() - 4;
        ok[dpos..].copy_from_slice(&4u32.to_le_bytes());
        ok.extend_from_slice(&[1, 2, 3, 4]);
        assert!(wav_has_audio(&ok));
    }

    fn ffmpeg_available() -> bool {
        Command::new("ffmpeg")
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn frames_mono16(wav: &[u8]) -> usize {
        wav_data_chunk(wav).map(|(_, sz)| sz / 2).unwrap_or(0)
    }

    /// Real-data: the Skyrim .fuz (variable-block xWMA that raw ffmpeg can't
    /// decode) must now be **rescued** by the extradata rebuild into audible
    /// audio (partial is fine). Skips if ffmpeg or the sample is absent.
    #[test]
    fn extradata_rescue_decodes_skyrim_fuz() {
        if !ffmpeg_available() {
            return;
        }
        let Ok(bytes) = std::fs::read("/tmp/sk.fuz") else {
            return;
        };
        let xwma = strip_fuz(&bytes).expect("strip fuz");
        let conv = Converter::Ffmpeg(PathBuf::from("ffmpeg"));
        let wav = decode_wma(&xwma, Some(&conv)).expect("rescue should yield audible audio");
        assert!(wav_has_audio(&wav) && !wav_is_silent(&wav));
        // Expected ~88064 mono frames; the rescue recovers a substantial part.
        assert!(
            frames_mono16(&wav) > 44000,
            "recovered {} frames",
            frames_mono16(&wav)
        );
    }

    /// Real-data: a Fallout 4 .fuz decodes fully via the raw stream (the
    /// extradata rescue must NOT be needed and must NOT be applied to it).
    #[test]
    fn raw_path_decodes_fo4_fuz() {
        if !ffmpeg_available() {
            return;
        }
        let Ok(bytes) = std::fs::read("/tmp/f4.fuz") else {
            return;
        };
        let xwma = strip_fuz(&bytes).expect("strip fuz");
        let conv = Converter::Ffmpeg(PathBuf::from("ffmpeg"));
        let wav = decode_wma(&xwma, Some(&conv)).expect("fo4 should decode");
        assert!(wav_has_audio(&wav) && !wav_is_silent(&wav));
        assert!(
            frames_mono16(&wav) > 150000,
            "fo4 frames {}",
            frames_mono16(&wav)
        );
    }

    #[test]
    fn xwma_bitrate_normalization() {
        // Fake-bitrate combos from MS's xWMA encoder get restored to the true rate.
        // 1ch 32000Hz written as 192000bps (24000 B/s) → real 20000bps (2500 B/s).
        assert_eq!(normalize_xwma_avg_bps(1, 32000, 24000), 2500);
        // 1ch 22050Hz written as 48000bps (6000 B/s) → real 20000bps (2500 B/s).
        assert_eq!(normalize_xwma_avg_bps(1, 22050, 6000), 2500);
        // 2ch 32000Hz written as 192000bps (24000 B/s) → real 48000bps (6000 B/s).
        assert_eq!(normalize_xwma_avg_bps(2, 32000, 24000), 6000);
        // A genuine bitrate (1ch 32000Hz @ 20000bps = 2500 B/s) is left untouched.
        assert_eq!(normalize_xwma_avg_bps(1, 32000, 2500), 2500);
        // An unlisted combo passes through unchanged.
        assert_eq!(normalize_xwma_avg_bps(1, 48000, 8000), 8000);
    }

    #[test]
    fn extradata_rebuild_is_valid_xwma() {
        // Minimal XWMA: fmt(18) + data. Rebuild must add a 10-byte extradata.
        let mut fmt = Vec::new();
        fmt.extend_from_slice(&0x0161u16.to_le_bytes()); // WMAv2
        fmt.extend_from_slice(&1u16.to_le_bytes()); // ch
        fmt.extend_from_slice(&32000u32.to_le_bytes());
        fmt.extend_from_slice(&2500u32.to_le_bytes());
        fmt.extend_from_slice(&1280u16.to_le_bytes());
        fmt.extend_from_slice(&16u16.to_le_bytes());
        fmt.extend_from_slice(&0u16.to_le_bytes()); // cbSize 0
        let data = vec![0u8; 1280];
        let mut body = Vec::new();
        body.extend_from_slice(b"fmt ");
        body.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
        body.extend_from_slice(&fmt);
        body.extend_from_slice(b"data");
        body.extend_from_slice(&(data.len() as u32).to_le_bytes());
        body.extend_from_slice(&data);
        let mut xwma = b"RIFF".to_vec();
        xwma.extend_from_slice(&((4 + body.len()) as u32).to_le_bytes());
        xwma.extend_from_slice(b"XWMA");
        xwma.extend_from_slice(&body);

        let out = build_xwma_with_extradata(&xwma, 0x0003).expect("rebuild");
        assert_eq!(&out[0..4], b"RIFF");
        assert_eq!(&out[8..12], b"XWMA");
        // cbSize should now be 10 (fmt base 16 → cbSize at offset 16 of fmt body).
        // Find fmt chunk and check.
        let mut i = 12;
        let mut found = false;
        while i + 8 <= out.len() {
            let id = &out[i..i + 4];
            let sz = u32::from_le_bytes([out[i + 4], out[i + 5], out[i + 6], out[i + 7]]) as usize;
            if id == b"fmt " {
                let cb = u16::from_le_bytes([out[i + 8 + 16], out[i + 8 + 17]]);
                assert_eq!(cb, 10);
                found = true;
            }
            i += 8 + sz + (sz & 1);
        }
        assert!(found);
    }

    #[test]
    fn silence_detection() {
        // Build a PCM16 mono WAV; all-zero data = silent, a loud sample = not.
        let make = |sample: i16, n: usize| {
            let data: Vec<u8> = (0..n).flat_map(|_| sample.to_le_bytes()).collect();
            let mut w = b"RIFF".to_vec();
            w.extend_from_slice(&((36 + data.len()) as u32).to_le_bytes());
            w.extend_from_slice(b"WAVEfmt ");
            w.extend_from_slice(&16u32.to_le_bytes());
            w.extend_from_slice(&1u16.to_le_bytes()); // PCM
            w.extend_from_slice(&1u16.to_le_bytes()); // mono
            w.extend_from_slice(&44100u32.to_le_bytes());
            w.extend_from_slice(&88200u32.to_le_bytes());
            w.extend_from_slice(&2u16.to_le_bytes());
            w.extend_from_slice(&16u16.to_le_bytes());
            w.extend_from_slice(b"data");
            w.extend_from_slice(&(data.len() as u32).to_le_bytes());
            w.extend_from_slice(&data);
            w
        };
        assert!(wav_is_silent(&make(0, 100)));
        assert!(wav_is_silent(&make(50, 100))); // below threshold
        assert!(!wav_is_silent(&make(10000, 100))); // clearly audible
    }

    #[test]
    fn ffmpeg_refuses_wem() {
        let conv = Converter::Ffmpeg(PathBuf::from("ffmpeg"));
        // A .wem never reaches ffmpeg — decode_to_wav must reject it early.
        let err = decode_to_wav("voice/x.wem", b"RIFF....", &conv).unwrap_err();
        assert!(err.contains("vgmstream"));
    }

    /// End-to-end decode via ffmpeg, if present: generate a short clip, run it
    /// through `decode_to_wav`, and check we get a valid WAV back. Skips cleanly
    /// when ffmpeg (or its vorbis encoder) isn't available.
    #[test]
    fn ffmpeg_decodes_to_wav_when_available() {
        let null = || std::process::Stdio::null();
        let has_ffmpeg = Command::new("ffmpeg")
            .arg("-version")
            .stdout(null())
            .stderr(null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !has_ffmpeg {
            return;
        }
        let src = std::env::temp_dir().join(format!("bsae_audiotest_{}.ogg", std::process::id()));
        let made = Command::new("ffmpeg")
            .args([
                "-y",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=0.3",
                "-c:a",
                "libvorbis",
            ])
            .arg(&src)
            .stdout(null())
            .stderr(null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !made {
            return; // no vorbis encoder → skip
        }
        let bytes = std::fs::read(&src).unwrap();
        let _ = std::fs::remove_file(&src);
        // Labelled .xwm; ffmpeg detects the real container by content.
        let conv = Converter::Ffmpeg(PathBuf::from("ffmpeg"));
        let wav = decode_to_wav("clip.xwm", &bytes, &conv).expect("ffmpeg decode");
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
    }
}
