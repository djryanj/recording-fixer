//! Apple's CoreAudio AAC encoder on Windows, driven through `CoreAudioToolbox.dll`.
//!
//! The DLL belongs to Apple Application Support (installed with iTunes). We load it at runtime and
//! never bundle or redistribute it. Output is raw AAC-LC packets wrapped in ADTS; ffmpeg muxes
//! them into an .m4a without re-encoding.

use crate::adts;
use crate::options::AacMode;
use crate::{Error, Result};
use aiff_core::Format;
use libloading::os::windows::Library;
use std::ffi::c_void;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

type OsStatus = i32;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Asbd {
    sample_rate: f64,
    format_id: u32,
    format_flags: u32,
    bytes_per_packet: u32,
    frames_per_packet: u32,
    bytes_per_frame: u32,
    channels_per_frame: u32,
    bits_per_channel: u32,
    reserved: u32,
}

#[repr(C)]
struct AudioBuffer {
    number_channels: u32,
    data_byte_size: u32,
    data: *mut c_void,
}

#[repr(C)]
struct AudioBufferList {
    number_buffers: u32,
    buffers: [AudioBuffer; 1],
}

#[repr(C)]
#[derive(Default)]
struct PacketDesc {
    start_offset: i64,
    variable_frames: u32,
    data_byte_size: u32,
}

type InputProc = unsafe extern "C" fn(
    *mut c_void,
    *mut u32,
    *mut AudioBufferList,
    *mut *mut PacketDesc,
    *mut c_void,
) -> OsStatus;
type NewFn = unsafe extern "C" fn(*const Asbd, *const Asbd, *mut *mut c_void) -> OsStatus;
type DisposeFn = unsafe extern "C" fn(*mut c_void) -> OsStatus;
type SetPropFn = unsafe extern "C" fn(*mut c_void, u32, u32, *const c_void) -> OsStatus;
type GetPropFn = unsafe extern "C" fn(*mut c_void, u32, *mut u32, *mut c_void) -> OsStatus;
type FillFn = unsafe extern "C" fn(
    *mut c_void,
    InputProc,
    *mut c_void,
    *mut u32,
    *mut AudioBufferList,
    *mut PacketDesc,
) -> OsStatus;

const fn fourcc(s: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*s)
}

const FORMAT_LPCM: u32 = fourcc(b"lpcm");
const FORMAT_AAC: u32 = fourcc(b"aac ");
const FLAG_BIG_ENDIAN: u32 = 2;
const FLAG_SIGNED_INT: u32 = 4;
const FLAG_PACKED: u32 = 8;
const AAC_LC_OBJECT_TYPE: u32 = 2;
const AAC_FRAMES_PER_PACKET: u32 = 1024;

const PROP_BITRATE: u32 = fourcc(b"brat");
const PROP_BITRATE_CONTROL_MODE: u32 = fourcc(b"acbf");
const PROP_VBR_QUALITY: u32 = fourcc(b"vbrq");
const PROP_CODEC_QUALITY: u32 = fourcc(b"cdqu");
const PROP_MAX_OUTPUT_PACKET: u32 = fourcc(b"xops");
const PROP_PRIME_INFO: u32 = fourcc(b"prim");

/// Apple's standard AAC encoder delay, used if the converter will not report its own.
pub const DEFAULT_PRIMING_FRAMES: u32 = 2112;

#[repr(C)]
#[derive(Default)]
struct PrimeInfo {
    leading_frames: u32,
    trailing_frames: u32,
}

const MODE_CONSTANT: u32 = 0;
const MODE_VARIABLE: u32 = 3;
const QUALITY_MAX: u32 = 0x7F;
/// LOAD_WITH_ALTERED_SEARCH_PATH: resolve the DLL's own dependencies from its folder first,
/// without changing the process-wide search path.
const LOAD_WITH_ALTERED_SEARCH_PATH: u32 = 0x8;

/// True-VBR quality (0..=127) for each of our five levels.
pub fn vbr_quality(level: u8) -> u32 {
    [23, 45, 63, 91, 127][usize::from(level.clamp(1, 5)) - 1]
}

struct Api {
    _lib: Library,
    new: NewFn,
    dispose: DisposeFn,
    set_prop: SetPropFn,
    get_prop: GetPropFn,
    fill: FillFn,
}

fn status(what: &str, s: OsStatus) -> Result<()> {
    if s == 0 {
        return Ok(());
    }
    // Statuses are often four-character codes; show both forms.
    let code = (s as u32).to_be_bytes();
    let text = if code.iter().all(|b| b.is_ascii_graphic()) {
        format!(" '{}'", String::from_utf8_lossy(&code))
    } else {
        String::new()
    };
    Err(Error::Apple(format!("{what} failed (status {s}{text})")))
}

/// Load each DLL once and keep it for the life of the process. Repeatedly unloading a
/// CoreFoundation-based DLL is a classic source of instability, and keeping it loaded is free.
fn api(dll: &Path) -> Result<&'static Api> {
    static LOADED: Mutex<Vec<(PathBuf, &'static Api)>> = Mutex::new(Vec::new());
    let mut loaded = LOADED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, a)) = loaded.iter().find(|(p, _)| p == dll) {
        return Ok(a);
    }
    let a: &'static Api = Box::leak(Box::new(Api::load(dll)?));
    loaded.push((dll.to_path_buf(), a));
    Ok(a)
}

impl Api {
    fn load(dll: &Path) -> Result<Api> {
        // SAFETY: loading Apple's DLL runs its initialisation code; that is inherent to using it.
        let lib = unsafe { Library::load_with_flags(dll, LOAD_WITH_ALTERED_SEARCH_PATH) }
            .map_err(|e| Error::Apple(format!("could not load {}: {e}", dll.display())))?;
        unsafe fn sym<T: Copy>(lib: &Library, name: &str) -> Result<T> {
            let s: libloading::os::windows::Symbol<T> = unsafe { lib.get(name.as_bytes()) }
                .map_err(|e| Error::Apple(format!("missing {name}: {e}")))?;
            Ok(*s)
        }
        // SAFETY: the signatures match Apple's published AudioToolbox C API.
        unsafe {
            Ok(Api {
                new: sym(&lib, "AudioConverterNew")?,
                dispose: sym(&lib, "AudioConverterDispose")?,
                set_prop: sym(&lib, "AudioConverterSetProperty")?,
                get_prop: sym(&lib, "AudioConverterGetProperty")?,
                fill: sym(&lib, "AudioConverterFillComplexBuffer")?,
                _lib: lib,
            })
        }
    }
}

fn pcm_description(f: &Format) -> Asbd {
    let bytes_per_frame = (f.block_align()) as u32;
    Asbd {
        sample_rate: f.sample_rate,
        format_id: FORMAT_LPCM,
        format_flags: FLAG_BIG_ENDIAN | FLAG_SIGNED_INT | FLAG_PACKED,
        bytes_per_packet: bytes_per_frame,
        frames_per_packet: 1,
        bytes_per_frame,
        channels_per_frame: u32::from(f.channels),
        bits_per_channel: u32::from(f.bits),
        reserved: 0,
    }
}

fn aac_description(f: &Format) -> Asbd {
    Asbd {
        sample_rate: f.sample_rate,
        format_id: FORMAT_AAC,
        format_flags: AAC_LC_OBJECT_TYPE,
        frames_per_packet: AAC_FRAMES_PER_PACKET,
        channels_per_frame: u32::from(f.channels),
        ..Asbd::default()
    }
}

struct Converter<'a> {
    api: &'a Api,
    handle: *mut c_void,
}

impl Drop for Converter<'_> {
    fn drop(&mut self) {
        // SAFETY: handle came from AudioConverterNew and is disposed exactly once.
        unsafe { (self.api.dispose)(self.handle) };
    }
}

impl<'a> Converter<'a> {
    fn new(api: &'a Api, format: &Format) -> Result<Self> {
        let (input, output) = (pcm_description(format), aac_description(format));
        let mut handle = std::ptr::null_mut();
        // SAFETY: both descriptions are valid for the call; handle receives the new converter.
        status("AudioConverterNew", unsafe {
            (api.new)(&input, &output, &mut handle)
        })?;
        Ok(Converter { api, handle })
    }

    fn set_u32(&self, what: &str, prop: u32, value: u32) -> Result<()> {
        // SAFETY: `value` outlives the call and the size matches.
        status(what, unsafe {
            (self.api.set_prop)(self.handle, prop, 4, (&raw const value).cast())
        })
    }

    fn get_u32(&self, what: &str, prop: u32) -> Result<u32> {
        let (mut value, mut size) = (0u32, 4u32);
        // SAFETY: `value` is a valid 4-byte out buffer and `size` states its capacity.
        status(what, unsafe {
            (self.api.get_prop)(self.handle, prop, &mut size, (&raw mut value).cast())
        })?;
        Ok(value)
    }

    /// Frames of encoder delay the decoder must skip (`kAudioConverterPrimeInfo`).
    fn leading_frames(&self) -> Result<u32> {
        let mut info = PrimeInfo::default();
        let mut size = std::mem::size_of::<PrimeInfo>() as u32;
        // SAFETY: `info` is a valid, correctly sized out buffer for this property.
        status("query priming", unsafe {
            (self.api.get_prop)(
                self.handle,
                PROP_PRIME_INFO,
                &mut size,
                (&raw mut info).cast(),
            )
        })?;
        Ok(info.leading_frames)
    }

    fn configure(&self, mode: &AacMode) -> Result<()> {
        self.set_u32("set quality", PROP_CODEC_QUALITY, QUALITY_MAX)?;
        match mode {
            AacMode::Cbr(kbps) => {
                self.set_u32(
                    "set constant bitrate mode",
                    PROP_BITRATE_CONTROL_MODE,
                    MODE_CONSTANT,
                )?;
                self.set_u32(&format!("set {kbps} kbps"), PROP_BITRATE, kbps * 1000)
            }
            AacMode::Vbr(level) => {
                self.set_u32(
                    "set variable bitrate mode",
                    PROP_BITRATE_CONTROL_MODE,
                    MODE_VARIABLE,
                )?;
                self.set_u32("set VBR quality", PROP_VBR_QUALITY, vbr_quality(*level))
            }
        }
    }
}

/// Feeds PCM to the converter from a reader, a chunk at a time.
struct Input<'r> {
    reader: &'r mut dyn Read,
    remaining: u64,
    buf: Vec<u8>,
    block_align: usize,
    channels: u32,
    consumed: u64,
    error: Option<std::io::Error>,
}

unsafe extern "C" fn input_proc(
    _converter: *mut c_void,
    packets: *mut u32,
    list: *mut AudioBufferList,
    _desc: *mut *mut PacketDesc,
    user: *mut c_void,
) -> OsStatus {
    // SAFETY: `user` is the `Input` we passed to FillComplexBuffer and it outlives the call.
    let st = unsafe { &mut *user.cast::<Input>() };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let want_frames = (unsafe { *packets } as usize).max(1);
        let want = (want_frames * st.block_align).min(st.remaining.min(usize::MAX as u64) as usize);
        if st.buf.len() < want {
            st.buf.resize(want, 0);
        }
        match st.reader.read_exact(&mut st.buf[..want]) {
            Ok(()) => Ok(want),
            Err(e) => Err(e),
        }
    }));
    let bytes = match result {
        Ok(Ok(n)) => n,
        Ok(Err(e)) => {
            st.error = Some(e);
            return -1;
        }
        Err(_) => return -1,
    };
    st.remaining -= bytes as u64;
    st.consumed += bytes as u64;
    // SAFETY: the converter gave us valid `packets` and `list` pointers for this callback.
    unsafe {
        (*list).number_buffers = 1;
        (*list).buffers[0] = AudioBuffer {
            number_channels: st.channels,
            data_byte_size: bytes as u32,
            data: if bytes == 0 {
                std::ptr::null_mut()
            } else {
                st.buf.as_mut_ptr().cast()
            },
        };
        *packets = (bytes / st.block_align) as u32;
    }
    0
}

/// Check that the DLL loads and can create an AAC encoder. Cheap; used to decide whether to offer it.
pub fn probe(dll: &Path) -> Result<()> {
    let api = api(dll)?;
    let format = Format {
        channels: 2,
        bits: 16,
        sample_rate: 44100.0,
    };
    let conv = Converter::new(api, &format)?;
    conv.configure(&AacMode::Cbr(256))
}

/// The encoder delay, in frames, that players must skip for this format and mode.
///
/// The encoder prepends this much silence ahead of the audio. OBS handles it the same way: it
/// reads the converter's prime info and shifts packet timestamps back by that amount, so the
/// muxer records an edit list rather than anything being padded or cut.
pub fn priming_frames(dll: &Path, format: &Format, mode: &AacMode) -> u32 {
    let query = || -> Result<u32> {
        let conv = Converter::new(api(dll)?, format)?;
        conv.configure(mode)?;
        conv.leading_frames()
    };
    query()
        .ok()
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_PRIMING_FRAMES)
}

/// Encode `total_bytes` of big-endian PCM from `reader` to ADTS-framed AAC written to `out`.
/// `progress(done, total)` returns false to cancel.
pub fn encode(
    dll: &Path,
    format: &Format,
    mode: &AacMode,
    reader: &mut dyn Read,
    total_bytes: u64,
    out: &mut dyn Write,
    progress: &mut dyn FnMut(u64, u64) -> bool,
) -> Result<()> {
    let rate = format.sample_rate.round() as u32;
    if adts::sample_rate_index(rate).is_none() || !(1..=7).contains(&format.channels) {
        return Err(Error::Apple(format!(
            "{rate} Hz / {} ch cannot be encoded to AAC",
            format.channels
        )));
    }
    let api = api(dll)?;
    let conv = Converter::new(api, format)?;
    conv.configure(mode)?;
    let max_packet = conv
        .get_u32("query packet size", PROP_MAX_OUTPUT_PACKET)?
        .max(1024) as usize;

    let mut input = Input {
        reader,
        remaining: total_bytes,
        buf: Vec::new(),
        block_align: format.block_align() as usize,
        channels: u32::from(format.channels),
        consumed: 0,
        error: None,
    };
    let mut packet = vec![0u8; max_packet];
    loop {
        let mut packets = 1u32;
        let mut desc = PacketDesc::default();
        let mut list = AudioBufferList {
            number_buffers: 1,
            buffers: [AudioBuffer {
                number_channels: u32::from(format.channels),
                data_byte_size: packet.len() as u32,
                data: packet.as_mut_ptr().cast(),
            }],
        };
        // SAFETY: all pointers are valid for the call; `input` outlives it.
        let s = unsafe {
            (api.fill)(
                conv.handle,
                input_proc,
                (&raw mut input).cast(),
                &mut packets,
                &mut list,
                &mut desc,
            )
        };
        if let Some(e) = input.error.take() {
            return Err(Error::Io(e));
        }
        if packets == 0 {
            // End of stream is reported as zero packets; anything else with no output is an error.
            if s != 0 && input.remaining != 0 {
                status("encode", s)?;
            }
            break;
        }
        let len = list.buffers[0].data_byte_size as usize;
        let header = adts::header(rate, format.channels, len)
            .ok_or_else(|| Error::Apple("packet too large for ADTS".into()))?;
        out.write_all(&header)?;
        out.write_all(&packet[..len])?;
        if !progress(input.consumed, total_bytes) {
            return Err(Error::Cancelled);
        }
    }
    progress(total_bytes, total_bytes);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apple;

    fn dll() -> Option<std::path::PathBuf> {
        apple::find_windows_dll()
    }

    /// One second of a stereo 440 Hz sine, big-endian 16-bit.
    fn sine(seconds: usize) -> Vec<u8> {
        let mut v = Vec::new();
        for i in 0..44100 * seconds {
            let s = ((i as f64 * 440.0 * std::f64::consts::TAU / 44100.0).sin() * 12000.0) as i16;
            v.extend_from_slice(&s.to_be_bytes());
            v.extend_from_slice(&s.to_be_bytes());
        }
        v
    }

    fn encode_to_adts(mode: AacMode, seconds: usize) -> Option<Vec<u8>> {
        let dll = dll()?;
        let pcm = sine(seconds);
        let mut out = Vec::new();
        let f = Format {
            channels: 2,
            bits: 16,
            sample_rate: 44100.0,
        };
        encode(
            &dll,
            &f,
            &mode,
            &mut &pcm[..],
            pcm.len() as u64,
            &mut out,
            &mut |_, _| true,
        )
        .unwrap();
        Some(out)
    }

    /// Walk ADTS frames: returns (frame count, total bytes) and panics if the stream is malformed.
    fn count_frames(adts: &[u8]) -> (usize, usize) {
        let (mut pos, mut n) = (0, 0);
        while pos < adts.len() {
            assert_eq!(adts[pos], 0xFF, "sync at {pos}");
            assert_eq!(adts[pos + 1] & 0xF0, 0xF0);
            let len = (usize::from(adts[pos + 3] & 3) << 11)
                | (usize::from(adts[pos + 4]) << 3)
                | usize::from(adts[pos + 5] >> 5);
            assert!(
                len > 7 && pos + len <= adts.len(),
                "frame length {len} at {pos}"
            );
            pos += len;
            n += 1;
        }
        (n, adts.len())
    }

    #[test]
    fn the_installed_dll_loads_and_creates_an_encoder() {
        let Some(dll) = dll() else {
            return eprintln!("skipping: Apple Application Support not installed");
        };
        probe(&dll).unwrap();
    }

    #[test]
    fn cbr_output_is_valid_adts_of_the_right_length_and_bitrate() {
        let Some(adts) = encode_to_adts(AacMode::Cbr(256), 3) else {
            return eprintln!("skipping: no Apple DLL");
        };
        let (frames, bytes) = count_frames(&adts);
        // 3 s at 44.1 kHz is 129 frames of 1024 samples, plus encoder priming frames.
        assert!((129..=133).contains(&frames), "{frames} frames");
        let kbps = bytes as f64 * 8.0 / 3.0 / 1000.0;
        assert!((220.0..300.0).contains(&kbps), "{kbps} kbps");
    }

    #[test]
    fn vbr_levels_get_bigger_as_quality_rises() {
        let Some(low) = encode_to_adts(AacMode::Vbr(1), 3) else {
            return eprintln!("skipping: no Apple DLL");
        };
        let high = encode_to_adts(AacMode::Vbr(5), 3).unwrap();
        assert!(
            count_frames(&high).1 > count_frames(&low).1,
            "{} vs {}",
            high.len(),
            low.len()
        );
    }

    #[test]
    fn an_unsupported_sample_rate_is_rejected_before_touching_the_dll() {
        let f = Format {
            channels: 2,
            bits: 16,
            sample_rate: 44101.0,
        };
        let r = encode(
            std::path::Path::new("nope.dll"),
            &f,
            &AacMode::Cbr(128),
            &mut &[][..],
            0,
            &mut Vec::new(),
            &mut |_, _| true,
        );
        assert!(matches!(r, Err(Error::Apple(m)) if m.contains("44101")));
    }

    #[test]
    fn a_missing_dll_is_a_clear_error() {
        let r = probe(std::path::Path::new(
            "C:/definitely/not/here/CoreAudioToolbox.dll",
        ));
        assert!(matches!(r, Err(Error::Apple(m)) if m.contains("could not load")));
    }

    #[test]
    fn cancel_stops_encoding() {
        let Some(dll) = dll() else { return };
        let pcm = sine(5);
        let f = Format {
            channels: 2,
            bits: 16,
            sample_rate: 44100.0,
        };
        let r = encode(
            &dll,
            &f,
            &AacMode::Cbr(128),
            &mut &pcm[..],
            pcm.len() as u64,
            &mut Vec::new(),
            &mut |_, _| false,
        );
        assert!(matches!(r, Err(Error::Cancelled)));
    }

    #[test]
    fn the_converter_reports_apples_standard_delay() {
        let Some(dll) = dll() else {
            return eprintln!("skipping: no Apple DLL");
        };
        let f = Format {
            channels: 2,
            bits: 16,
            sample_rate: 44100.0,
        };
        let n = priming_frames(&dll, &f, &AacMode::Cbr(256));
        // 2112 on every Apple AAC-LC encoder we know of; accept a sane range in case a newer one changes it.
        assert!((1024..=4096).contains(&n), "{n}");
        assert_eq!(
            n, DEFAULT_PRIMING_FRAMES,
            "this machine's encoder reports a different delay; update the default"
        );
    }

    #[test]
    fn an_unloadable_dll_falls_back_to_the_default_delay() {
        let f = Format {
            channels: 2,
            bits: 16,
            sample_rate: 44100.0,
        };
        let n = priming_frames(Path::new("C:/not/here.dll"), &f, &AacMode::Cbr(256));
        assert_eq!(n, DEFAULT_PRIMING_FRAMES);
    }

    #[test]
    fn vbr_quality_mapping_is_monotonic_and_in_range() {
        let q: Vec<u32> = (1..=5).map(vbr_quality).collect();
        assert!(q.windows(2).all(|w| w[0] < w[1]));
        assert!(q.iter().all(|&v| v <= 127));
    }
}
