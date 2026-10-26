//! The real audio output, through cpal: WASAPI (and ASIO when built with the `asio` feature) on
//! Windows, CoreAudio on macOS.

use crate::device::{
    AudioCallback, Backend, DeviceInfo, DeviceSelection, OpenedInfo, OpenedStream,
};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    BufferSize, DeviceId, FromSample, SampleFormat, SizedSample, StreamConfig, SupportedBufferSize,
};
use std::str::FromStr;

pub struct CpalBackend;

impl CpalBackend {
    pub fn new() -> Self {
        CpalBackend
    }
}

impl Default for CpalBackend {
    fn default() -> Self {
        Self::new()
    }
}

/// One supported configuration, reduced to what the choice depends on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RangeInfo {
    pub channels: u16,
    pub min_rate: u32,
    pub max_rate: u32,
    pub format: SampleFormat,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Choice {
    pub index: usize,
    pub rate: u32,
}

fn format_rank(f: SampleFormat) -> Option<u8> {
    match f {
        SampleFormat::F32 => Some(0),
        SampleFormat::I32 => Some(1),
        SampleFormat::I16 => Some(2),
        _ => None,
    }
}

/// Pick the configuration to open. Preference order: enough channels for the chosen output pair,
/// the file's own sample rate (no resampling), then the most natural sample format, then the
/// fewest surplus channels. Falls back gracefully when nothing matches exactly.
pub fn choose_config(
    ranges: &[RangeInfo],
    preferred_rate: u32,
    needed_channels: u16,
    default_rate: u32,
) -> Option<Choice> {
    let usable: Vec<(usize, &RangeInfo)> = ranges
        .iter()
        .enumerate()
        .filter(|(_, r)| format_rank(r.format).is_some() && r.channels >= 1)
        .collect();
    let rank = |r: &RangeInfo| {
        (
            u8::from(r.channels < needed_channels),
            u8::from(!(r.min_rate..=r.max_rate).contains(&preferred_rate)),
            format_rank(r.format).unwrap_or(9),
            r.channels.abs_diff(needed_channels.max(2)),
        )
    };
    let (index, best) = usable.into_iter().min_by_key(|(_, r)| rank(r))?;
    let rate = if (best.min_rate..=best.max_rate).contains(&preferred_rate) {
        preferred_rate
    } else if (best.min_rate..=best.max_rate).contains(&default_rate) {
        default_rate
    } else {
        preferred_rate.clamp(best.min_rate, best.max_rate)
    };
    Some(Choice { index, rate })
}

fn hosts() -> Vec<cpal::Host> {
    cpal::available_hosts()
        .into_iter()
        .filter_map(|id| cpal::host_from_id(id).ok())
        .collect()
}

fn describe(
    host_name: &str,
    device: &cpal::Device,
    is_default: bool,
    probe: bool,
) -> Option<DeviceInfo> {
    let id = device.id().ok()?.to_string();
    let name = device
        .description()
        .ok()
        .map(|d| d.name().to_string())
        .unwrap_or_else(|| id.clone());
    let (channels, default_rate) = if probe {
        let channels = device
            .supported_output_configs()
            .ok()
            .and_then(|c| c.map(|r| r.channels()).max())
            .unwrap_or(0);
        (
            channels,
            device
                .default_output_config()
                .map_or(0, |c| c.sample_rate()),
        )
    } else {
        (0, 0)
    };
    Some(DeviceInfo {
        id,
        host: host_name.to_string(),
        name,
        channels,
        default_rate,
        is_default,
    })
}

impl Backend for CpalBackend {
    fn devices(&self) -> Vec<DeviceInfo> {
        let mut out = Vec::new();
        for host in hosts() {
            let host_name = host.id().to_string();
            let default_id = host.default_output_device().and_then(|d| d.id().ok());
            // Asking an ASIO driver for its configs loads the driver, which can be slow or fail
            // while another program owns the hardware, so only probe the cheap hosts here.
            let probe = host_name != "asio";
            let Ok(devices) = host.output_devices() else {
                continue;
            };
            for device in devices {
                let is_default = default_id
                    .as_ref()
                    .is_some_and(|d| device.id().ok().as_ref() == Some(d));
                out.extend(describe(&host_name, &device, is_default, probe));
            }
        }
        out
    }

    fn details(&self, id: &str) -> Option<DeviceInfo> {
        let id = DeviceId::from_str(id).ok()?;
        let host = cpal::host_from_id(id.host()).ok()?;
        let device = host.device_by_id(&id)?;
        describe(&id.host().to_string(), &device, false, true)
    }

    fn open(
        &self,
        selection: &DeviceSelection,
        preferred_rate: u32,
        callback: AudioCallback,
    ) -> Result<OpenedStream, String> {
        let device = match &selection.id {
            Some(s) => {
                let id = DeviceId::from_str(s).map_err(|e| format!("bad device id {s:?}: {e}"))?;
                let host = cpal::host_from_id(id.host())
                    .map_err(|e| format!("audio host {} is unavailable: {e}", id.host()))?;
                host.device_by_id(&id)
                    .ok_or_else(|| format!("audio device {s} was not found (unplugged?)"))?
            }
            None => cpal::default_host()
                .default_output_device()
                .ok_or("no default audio output device")?,
        };
        let id = device.id().map_err(|e| e.to_string())?;
        let name = device
            .description()
            .map(|d| d.name().to_string())
            .unwrap_or_else(|_| id.to_string());

        let ranges: Vec<_> = device
            .supported_output_configs()
            .map_err(|e| format!("could not query {name}: {e}"))?
            .collect();
        let infos: Vec<RangeInfo> = ranges
            .iter()
            .map(|r| RangeInfo {
                channels: r.channels(),
                min_rate: r.min_sample_rate(),
                max_rate: r.max_sample_rate(),
                format: r.sample_format(),
            })
            .collect();
        let default_rate = device
            .default_output_config()
            .map_or(0, |c| c.sample_rate());
        let needed = (selection.out_pair * 2 + 2) as u16;
        let choice = choose_config(&infos, preferred_rate, needed, default_rate)
            .ok_or_else(|| format!("{name} offers no output format this app can use"))?;
        let range = ranges[choice.index];
        let supported = range
            .try_with_sample_rate(choice.rate)
            .ok_or_else(|| format!("{name} does not support {} Hz", choice.rate))?;
        let buffer_size = match (selection.buffer_frames, supported.buffer_size()) {
            (Some(n), SupportedBufferSize::Range { min, max }) => {
                BufferSize::Fixed(n.clamp(*min, *max))
            }
            _ => BufferSize::Default,
        };
        let config = StreamConfig {
            channels: supported.channels(),
            sample_rate: supported.sample_rate(),
            buffer_size,
        };
        let stream = match supported.sample_format() {
            SampleFormat::F32 => build_f32(&device, config, callback),
            SampleFormat::I32 => build_int::<i32>(&device, config, callback),
            SampleFormat::I16 => build_int::<i16>(&device, config, callback),
            other => Err(format!("unsupported sample format {other:?}")),
        }
        .map_err(|e| format!("could not open {name}: {e}"))?;
        stream
            .play()
            .map_err(|e| format!("could not start {name}: {e}"))?;

        let info = OpenedInfo {
            id: id.to_string(),
            host: id.host().to_string(),
            name,
            rate: config.sample_rate,
            channels: config.channels,
            out_pair: if (selection.out_pair * 2 + 1) < usize::from(config.channels) {
                selection.out_pair
            } else {
                0
            },
            buffer_frames: match config.buffer_size {
                BufferSize::Fixed(n) => Some(n),
                BufferSize::Default => None,
            },
        };
        Ok(OpenedStream {
            info,
            keepalive: Box::new(stream),
        })
    }
}

fn log_stream_error(e: cpal::Error) {
    eprintln!("audio stream error: {e}");
}

fn build_f32(
    device: &cpal::Device,
    config: StreamConfig,
    mut callback: AudioCallback,
) -> Result<cpal::Stream, String> {
    device
        .build_output_stream(
            config,
            move |data: &mut [f32], _| {
                callback(data);
                for s in data.iter_mut() {
                    *s = s.clamp(-1.0, 1.0);
                }
            },
            log_stream_error,
            None,
        )
        .map_err(|e| e.to_string())
}

fn build_int<T>(
    device: &cpal::Device,
    config: StreamConfig,
    mut callback: AudioCallback,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
{
    // Sized up front so the callback does not allocate in the common case.
    let mut scratch: Vec<f32> = vec![0.0; 1 << 15];
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                if scratch.len() < data.len() {
                    scratch.resize(data.len(), 0.0);
                }
                let block = &mut scratch[..data.len()];
                callback(block);
                for (out, s) in data.iter_mut().zip(block.iter()) {
                    *out = T::from_sample(s.clamp(-1.0, 1.0));
                }
            },
            log_stream_error,
            None,
        )
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(channels: u16, min: u32, max: u32, format: SampleFormat) -> RangeInfo {
        RangeInfo {
            channels,
            min_rate: min,
            max_rate: max,
            format,
        }
    }

    #[test]
    fn prefers_the_files_own_rate_so_nothing_is_resampled() {
        let ranges = [
            r(2, 48000, 48000, SampleFormat::F32),
            r(2, 44100, 44100, SampleFormat::F32),
        ];
        assert_eq!(
            choose_config(&ranges, 44100, 2, 48000),
            Some(Choice {
                index: 1,
                rate: 44100
            })
        );
    }

    #[test]
    fn falls_back_to_the_device_default_rate_when_the_file_rate_is_unsupported() {
        let ranges = [r(2, 48000, 48000, SampleFormat::F32)];
        assert_eq!(
            choose_config(&ranges, 44100, 2, 48000),
            Some(Choice {
                index: 0,
                rate: 48000
            })
        );
    }

    #[test]
    fn clamps_into_range_if_even_the_default_rate_is_outside_it() {
        let ranges = [r(2, 96000, 192000, SampleFormat::F32)];
        assert_eq!(choose_config(&ranges, 44100, 2, 48000).unwrap().rate, 96000);
    }

    #[test]
    fn needs_enough_channels_for_the_chosen_output_pair() {
        let ranges = [
            r(2, 44100, 44100, SampleFormat::F32),
            r(8, 44100, 44100, SampleFormat::F32),
        ];
        assert_eq!(choose_config(&ranges, 44100, 6, 44100).unwrap().index, 1);
        assert_eq!(choose_config(&ranges, 44100, 2, 44100).unwrap().index, 0);
    }

    #[test]
    fn uses_fewer_channels_than_needed_only_as_a_last_resort() {
        let ranges = [r(2, 44100, 44100, SampleFormat::F32)];
        assert_eq!(choose_config(&ranges, 44100, 8, 44100).unwrap().index, 0);
    }

    #[test]
    fn prefers_float_then_32_bit_then_16_bit() {
        let ranges = [
            r(2, 44100, 44100, SampleFormat::I16),
            r(2, 44100, 44100, SampleFormat::I32),
            r(2, 44100, 44100, SampleFormat::F32),
        ];
        assert_eq!(choose_config(&ranges, 44100, 2, 44100).unwrap().index, 2);
        assert_eq!(
            choose_config(&ranges[..2], 44100, 2, 44100).unwrap().index,
            1
        );
    }

    #[test]
    fn ignores_formats_it_cannot_drive_and_returns_none_if_nothing_is_left() {
        assert_eq!(
            choose_config(&[r(2, 44100, 44100, SampleFormat::F64)], 44100, 2, 44100),
            None
        );
        assert_eq!(choose_config(&[], 44100, 2, 44100), None);
        let ranges = [
            r(2, 44100, 44100, SampleFormat::U8),
            r(2, 44100, 44100, SampleFormat::I16),
        ];
        assert_eq!(choose_config(&ranges, 44100, 2, 44100).unwrap().index, 1);
    }

    #[test]
    fn rate_match_outranks_sample_format() {
        // A 16-bit config at the file's rate beats a float config that would need resampling.
        let ranges = [
            r(2, 48000, 48000, SampleFormat::F32),
            r(2, 44100, 44100, SampleFormat::I16),
        ];
        assert_eq!(
            choose_config(&ranges, 44100, 2, 48000).unwrap(),
            Choice {
                index: 1,
                rate: 44100
            }
        );
    }

    #[test]
    fn the_real_backend_can_enumerate_without_crashing() {
        // Hardware-dependent contents, but listing must never panic, even with no devices.
        let devices = CpalBackend::new().devices();
        for d in &devices {
            assert!(!d.id.is_empty() && !d.host.is_empty(), "{d:?}");
        }
    }

    #[test]
    fn device_info_ids_round_trip_through_cpal() {
        for d in CpalBackend::new().devices() {
            let parsed = DeviceId::from_str(&d.id).expect("ids must parse back");
            assert_eq!(parsed.to_string(), d.id);
        }
    }
}
