// src/apu/output.rs

use anyhow::anyhow;
use cpal::traits::{ DeviceTrait, HostTrait, StreamTrait };
use cpal::{ SampleFormat, Stream };
use ringbuf::traits::{ Consumer, Split };
use ringbuf::HeapRb;

/// The emulator side — push samples into this from the main thread.
pub type SampleProducer = ringbuf::HeapProd<f32>;
/// The audio thread side — cpal drains from this.
type SampleConsumer = ringbuf::HeapCons<f32>;

pub struct AudioOutput {
    /// Keeping the stream alive — dropping it stops audio.
    _stream: Stream,
    pub producer: SampleProducer,
    pub sample_rate: f32,
}

impl AudioOutput {
    pub fn new() -> anyhow::Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| anyhow!("no audio output device found"))?;

        let config = device.default_output_config()?;
        let sample_rate = config.sample_rate() as f32;

        // ~200ms of stereo samples in the ring buffer
        let buf_size = ((sample_rate as usize) / 5) * 2;
        let rb = HeapRb::<f32>::new(buf_size);
        let (producer, consumer) = rb.split();

        let stream = match config.sample_format() {
            SampleFormat::F32 => build_stream::<f32>(&device, &config.into(), consumer)?,
            SampleFormat::I16 => build_stream::<i16>(&device, &config.into(), consumer)?,
            SampleFormat::U16 => build_stream::<u16>(&device, &config.into(), consumer)?,
            fmt => anyhow::bail!("unsupported sample format: {fmt:?}"),
        };

        stream.play()?;

        Ok(Self {
            _stream: stream,
            producer,
            sample_rate,
        })
    }
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut consumer: SampleConsumer
) -> anyhow::Result<Stream>
    where T: cpal::SizedSample + cpal::FromSample<f32>
{
    let channels = config.channels as usize;

    let stream = device.build_output_stream(
        config,
        move |output: &mut [T], _| {
            for frame in output.chunks_mut(channels) {
                // Pop a stereo pair; output silence if the buffer is starved
                let l = consumer.try_pop().unwrap_or(0.0);
                let r = consumer.try_pop().unwrap_or(0.0);
                frame[0] = T::from_sample(l);
                if channels > 1 {
                    frame[1] = T::from_sample(r);
                }
            }
        },
        |err| eprintln!("[APU] stream error: {err}"),
        None
    )?;

    Ok(stream)
}
