use core::f32;
use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, widgets, EguiState};
use std::sync::Arc;

pub struct KickSynth {
    pub params: Arc<KickParams>,
    sample_rate: f32,

    osc_state: OscillatorState,
    pitch_env_state: AhdsrState,
    amp_env_state: AhdsrState,

    last_midi_note: Option<u8>,
    midi_frequency: f32,
    midi_velocity: f32,
}

#[derive(Params)]
pub struct AhdsrParams {
    #[id = "attack"]
    pub attack_time: FloatParam,
    #[id = "hold"]
    pub hold_time: FloatParam,
    #[id = "decay"]
    pub decay_time: FloatParam,
    #[id = "sustain"]
    pub sustain_level: FloatParam,
    #[id = "release"]
    pub release_time: FloatParam,
}

#[derive(Copy, Clone, Debug)]
pub struct AhdsrValues {
    pub attack: f32,
    pub hold: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
}

impl AhdsrValues {
    fn ahdr_all(time: f32) -> Self {
        Self::ahdr(time, time, time, time)
    }
    fn ahdr(attack: f32, hold: f32, decay: f32, release: f32) -> Self {
        Self::ahdsr(attack, hold, decay, 1.0, release)
    }
    fn ahdsr(attack: f32, hold: f32, decay: f32, sustain: f32, release: f32) -> Self {
        Self {
            attack,
            hold,
            decay,
            sustain,
            release,
        }
    }
    fn time_for_stage(&self, stage: AhdsrStage) -> Option<f32> {
        match stage {
            AhdsrStage::NotTriggered => None,
            AhdsrStage::Attack => Some(self.attack),
            AhdsrStage::Hold => Some(self.hold),
            AhdsrStage::Decay => Some(self.decay),
            AhdsrStage::Sustain => None,
            AhdsrStage::Release => Some(self.release),
        }
    }
}

impl AhdsrParams {
    pub fn new(
        prefix: &str,
        factor: f32,
        min_values: AhdsrValues,
        max_values: AhdsrValues,
        default_values: AhdsrValues,
    ) -> Self {
        Self {
            attack_time: FloatParam::new(
                format!("{prefix}Attack Time"),
                default_values.attack,
                FloatRange::Skewed {
                    min: min_values.attack,
                    max: max_values.attack,
                    factor,
                },
            )
            .with_unit(" s")
            .with_smoother(SmoothingStyle::Linear(5.0))
            .with_value_to_string(Arc::new(|value| format!("{value:.2}"))),
            hold_time: FloatParam::new(
                format!("{prefix}Hold Time"),
                default_values.hold,
                FloatRange::Skewed {
                    min: min_values.hold,
                    max: max_values.hold,
                    factor,
                },
            )
            .with_unit(" s")
            .with_smoother(SmoothingStyle::Linear(5.0))
            .with_value_to_string(Arc::new(|value| format!("{value:.2}"))),
            decay_time: FloatParam::new(
                format!("{prefix}Decay Time"),
                default_values.decay,
                FloatRange::Skewed {
                    min: min_values.decay,
                    max: max_values.decay,
                    factor,
                },
            )
            .with_unit(" s")
            .with_smoother(SmoothingStyle::Linear(5.0))
            .with_value_to_string(Arc::new(|value| format!("{value:.2}"))),
            release_time: FloatParam::new(
                format!("{prefix}Release Time"),
                default_values.release,
                FloatRange::Skewed {
                    min: min_values.release,
                    max: max_values.release,
                    factor,
                },
            )
            .with_unit(" s")
            .with_smoother(SmoothingStyle::Linear(5.0))
            .with_value_to_string(Arc::new(|value| format!("{value:.2}"))),
            sustain_level: FloatParam::new(
                "Sustain Value",
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
        }
    }
}

impl Default for AhdsrParams {
    fn default() -> Self {
        Self::new(
            "",
            FloatRange::skew_factor(-2.0),
            AhdsrValues::ahdr_all(0.0),
            AhdsrValues::ahdr_all(10.0),
            AhdsrValues::ahdr_all(1.0),
        )
    }
}

#[derive(Params)]
pub struct KickParams {
    #[nested(id_prefix = "amp_env")]
    amp_env: AhdsrParams,
    #[nested(id_prefix = "pitch_env")]
    pitch_env: AhdsrParams,
    #[id = "start_freq"]
    pub start_freq: FloatParam,
    #[id = "end_freq"]
    pub end_freq: FloatParam,
    #[id = "phase_offset"]
    pub phase_offset: FloatParam,
}

impl Default for KickSynth {
    fn default() -> Self {
        Self {
            params: Default::default(),
            sample_rate: 0.0,
            osc_state: Default::default(),
            midi_frequency: 200.0,
            midi_velocity: 0.0,
            pitch_env_state: Default::default(),
            amp_env_state: Default::default(),
            last_midi_note: None,
        }
    }
}

impl Default for KickParams {
    fn default() -> Self {
        Self {
            amp_env: AhdsrParams::new(
                "Amp ",
                FloatRange::skew_factor(-2.0),
                AhdsrValues::ahdr_all(0.0),
                AhdsrValues::ahdr_all(10.0),
                AhdsrValues::ahdsr(0.0, 0.0, 0.5, 0.0, 0.5),
            ),
            pitch_env: AhdsrParams::new(
                "Pitch ",
                FloatRange::skew_factor(-2.0),
                AhdsrValues::ahdr_all(0.0),
                AhdsrValues::ahdr_all(1.0),
                AhdsrValues::ahdsr(0.0, 0.0, 0.025, 0.0, 0.025),
            ),
            start_freq: FloatParam::new(
                "Start Freq",
                1000.0,
                FloatRange::Skewed {
                    min: 20.0,
                    max: 20000.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_value_to_string(formatters::v2s_f32_hz_then_khz_with_note_name(0, true))
            .with_string_to_value(formatters::s2v_f32_hz_then_khz()),
            end_freq: FloatParam::new(
                "End Freq",
                41.0,
                FloatRange::Skewed {
                    min: 20.0,
                    max: 20000.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_value_to_string(formatters::v2s_f32_hz_then_khz_with_note_name(0, true))
            .with_string_to_value(formatters::s2v_f32_hz_then_khz()),
            phase_offset: FloatParam::new(
                "Phase Offset",
                0.25,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_step_size(0.01),
        }
    }
}

impl Plugin for KickSynth {
    const NAME: &'static str = "but heres the kicker";
    const VENDOR: &'static str = "Rigel Narcissus";
    const URL: &'static str = "https://youtu.be/dQw4w9WgXcQ";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(1),
        ..AudioIOLayout::const_default()
    }];

    type SysExMessage = ();
    type BackgroundTask = ();

    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    // fn editor(&mut self, async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
    // }

    fn initialize(
        &mut self,
        audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;
        self.osc_state.sample_rate = buffer_config.sample_rate;
        self.pitch_env_state.sample_rate = buffer_config.sample_rate;
        self.amp_env_state.sample_rate = buffer_config.sample_rate;
        true
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let mut next_event = context.next_event();
        for (sample_id, mut channel_samples) in buffer.iter_samples().enumerate() {
            while let Some(event) = next_event {
                if event.timing() > sample_id as u32 {
                    break;
                }
                match event {
                    NoteEvent::NoteOn { note, velocity, .. } => {
                        self.midi_frequency = util::midi_note_to_freq(note);
                        self.midi_velocity = velocity;
                        self.last_midi_note = Some(note);
                        self.amp_env_state.trigger(true);
                        self.pitch_env_state.trigger(true);
                        self.osc_state.phase = self.params.phase_offset.modulated_plain_value();
                    }
                    NoteEvent::NoteOff { note, .. } if Some(note) == self.last_midi_note => {
                        self.last_midi_note = None;
                        self.amp_env_state.trigger(false);
                        self.pitch_env_state.trigger(false);
                    }
                    _ => {}
                }
                next_event = context.next_event();
            }

            self.pitch_env_state.apply_params(&self.params.pitch_env);
            self.amp_env_state.apply_params(&self.params.amp_env);

            let pitch_env = self.pitch_env_state.advance();
            let amp_env = self.amp_env_state.advance();

            let start_freq = self.params.start_freq.smoothed.next();
            let end_freq = self.params.end_freq.smoothed.next();
            let freq = lerp(pitch_env, end_freq, start_freq);

            let osc_scample = amp_env * osc_sine(self.osc_state.advance(freq));

            for sample in channel_samples.iter_mut() {
                *sample = osc_scample;
            }
        }
        ProcessStatus::KeepAlive
    }
}

impl ClapPlugin for KickSynth {
    const CLAP_ID: &'static str = "net.xavil.kick-synth";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("A basic kick synth");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Drum,
        ClapFeature::Mono,
    ];
}
nih_export_clap!(KickSynth);

#[derive(Copy, Clone, Debug, Default)]
struct OscillatorState {
    sample_rate: f32,
    phase: f32,
}

impl OscillatorState {
    fn advance(&mut self, frequency: f32) -> f32 {
        let old_phase = self.phase;
        self.phase += frequency * self.sample_rate.recip();
        if self.phase >= 1.0 {
            self.phase -= f32::floor(self.phase);
        }
        old_phase
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Default)]
enum AhdsrStage {
    #[default]
    NotTriggered,
    Attack,
    Hold,
    Decay,
    Sustain,
    Release,
}

impl AhdsrStage {
    fn next(&self) -> AhdsrStage {
        match self {
            AhdsrStage::NotTriggered => AhdsrStage::Attack,
            AhdsrStage::Attack => AhdsrStage::Hold,
            AhdsrStage::Hold => AhdsrStage::Decay,
            AhdsrStage::Decay => AhdsrStage::Sustain,
            AhdsrStage::Sustain => AhdsrStage::Release,
            AhdsrStage::Release => AhdsrStage::NotTriggered,
        }
    }

    fn endpoint_values(&self, current: f32, sustain: f32) -> (f32, f32) {
        match self {
            AhdsrStage::NotTriggered => (0.0, 0.0),
            AhdsrStage::Attack => (current, 1.0),
            AhdsrStage::Hold => (1.0, 1.0),
            AhdsrStage::Decay => (1.0, sustain),
            AhdsrStage::Sustain => (sustain, sustain),
            AhdsrStage::Release => (current, 0.0),
        }
    }
}

#[derive(Copy, Clone, Debug, Default)]
struct AhdsrState {
    sample_rate: f32,

    current_stage: AhdsrStage,
    samples_since_stage_start: u64,
    last_value_at_transition: f32,
    current: f32,

    attack: f32,
    hold: f32,
    decay: f32,
    sustain: f32,
    release: f32,
}

impl AhdsrState {
    fn apply_params(&mut self, params: &AhdsrParams) {
        self.attack = params.attack_time.smoothed.next();
        self.hold = params.hold_time.smoothed.next();
        self.decay = params.decay_time.smoothed.next();
        self.sustain = params.sustain_level.smoothed.next();
        self.release = params.release_time.smoothed.next();
    }

    fn trigger(&mut self, triggered: bool) {
        self.set_stage(match triggered {
            true => AhdsrStage::Attack,
            false => AhdsrStage::Release,
        });
    }

    fn set_stage(&mut self, stage: AhdsrStage) {
        self.current_stage = stage;
        self.samples_since_stage_start = 0;
        let (start, _) = stage.endpoint_values(self.current, self.sustain);
        self.current = start;
        self.last_value_at_transition = start;
    }

    fn advance(&mut self) -> f32 {
        let seconds_per_sample = self.sample_rate.recip();

        let stage_time = loop {
            let time = match self.current_stage {
                // neither of these stages have a time associated with them, so just bail early.
                AhdsrStage::NotTriggered => return 0.0,
                AhdsrStage::Sustain => return self.sustain,

                AhdsrStage::Attack => self.attack,
                AhdsrStage::Hold => self.hold,
                AhdsrStage::Decay => self.decay,
                AhdsrStage::Release => self.release,
            };
            if time > 0.0 {
                // shatter the fabric of spacetime, etc.
                break time;
            }
            // skip to the next stage that isn't zero-length
            self.set_stage(self.current_stage.next());
        };

        let mut time_since_stage_start = self.samples_since_stage_start as f32 * seconds_per_sample;

        if time_since_stage_start >= stage_time {
            self.set_stage(self.current_stage.next());
            time_since_stage_start = 0.0;
        }
        self.samples_since_stage_start += 1;

        let (start_value, end_value) = self
            .current_stage
            .endpoint_values(self.last_value_at_transition, self.sustain);
        let t = time_since_stage_start / stage_time;
        self.current = lerp(t, start_value.powf(0.5), end_value.powf(0.5)).powf(2.0);
        self.current
    }
}

fn invlerp(x: f32, a: f32, b: f32) -> f32 {
    (x - a) / (b - a)
}

fn lerp(t: f32, a: f32, b: f32) -> f32 {
    a + (b - a) * t
}

fn osc_sine(phase: f32) -> f32 {
    f32::sin(f32::consts::TAU * phase)
}
