// src/bin/viewer.rs
use gb_emulator::gpu::DebugOverlayConfig;
use gb_emulator::{ CPU, MemoryBus };
use gb_emulator::input::joypad::Button;
use gb_emulator::util::{ dmg_shade_to_u8, save_png };

use anyhow::Result;
use pixels::{ Pixels, SurfaceTexture };
use std::path::Path;
use std::sync::Arc;
use std::time::{ Duration, Instant };
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::{ ElementState, KeyEvent, StartCause, WindowEvent },
    event_loop::EventLoop,
    keyboard::{ KeyCode, PhysicalKey },
    window::{ Window, WindowId },
};

const W: usize = 160;
const H: usize = 144;
type Framebuffer = [[u8; W]; H];

// DMG timing (one LCD frame)
const DMG_DOTS_PER_FRAME: u32 = 70_224;
const DMG_CLOCK_HZ: u32 = 4_194_304;

// ~16.743 ms (exact ratio without float drift)
const NANOS_PER_SEC: u128 = 1_000_000_000;
const FRAME_NS: u128 = (NANOS_PER_SEC * (DMG_DOTS_PER_FRAME as u128)) / (DMG_CLOCK_HZ as u128);
const FRAME_DT: Duration = Duration::from_nanos(FRAME_NS as u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunState {
    Running,
    Paused,
    StepOneFrame,
    ExitRequested,
}

#[derive(Debug, Clone)]
struct ViewerConfig {
    frame_dt: Duration,
    max_catchup_frames: u32,
    pause_on_start: bool,
    trace_structured: bool,
    capture_first_n: u32,
    capture_skip: u32,
    capture_prefix: String,
    capture_require_lcd_on: bool,
}

impl ViewerConfig {
    fn from_env() -> Self {
        fn parse_u32(name: &str, default: u32) -> u32 {
            std::env
                ::var(name)
                .ok()
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(default)
        }

        fn parse_bool(name: &str, default: bool) -> bool {
            match std::env::var(name) {
                Ok(v) => {
                    let norm = v.trim().to_ascii_lowercase();
                    norm == "1" || norm == "true" || norm == "yes" || norm == "on"
                }
                Err(_) => default,
            }
        }

        let frame_dt = std::env
            ::var("GB_VIEWER_TARGET_FPS")
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|fps| *fps > 0.0)
            .map(|fps| Duration::from_secs_f64(1.0 / fps))
            .unwrap_or(FRAME_DT);

        Self {
            frame_dt,
            max_catchup_frames: parse_u32("GB_VIEWER_MAX_CATCHUP", 3).max(1),
            pause_on_start: parse_bool("GB_VIEWER_PAUSE_ON_START", false),
            trace_structured: parse_bool("GB_TRACE_STRUCTURED", false),
            capture_first_n: parse_u32("GB_CAPTURE_FIRST_N", 10),
            capture_skip: parse_u32("GB_CAPTURE_SKIP", 8),
            capture_prefix: std::env
                ::var("GB_CAPTURE_PREFIX")
                .unwrap_or_else(|_| "boot-frame".to_string()),
            capture_require_lcd_on: parse_bool("GB_CAPTURE_REQUIRE_LCD_ON", false),
        }
    }
}

/// Toggled via G/W/A/S hotkeys; drives DebugOverlayConfig on every change.
#[derive(Debug, Clone, Copy, Default)]
struct OverlayToggles {
    grid: bool,
    window: bool,
    axes: bool,
    sprites: bool,
}

/// Frame and screenshot counters for the viewer's capture system.
#[derive(Debug, Clone, Copy, Default)]
struct CaptureState {
    /// Counter for manual F9 screenshots.
    screenshot_id: u32,
    /// Total frames emulated (used for capture_skip logic).
    frame_count: u32,
    /// Number of automatic captures taken so far.
    captured_so_far: u32,
}

#[derive(Debug, Clone, Copy)]
struct ViewerStats {
    emulated_frames: u64,
    presented_frames: u64,
    dropped_deadlines: u64,
    catchup_frames: u64,
    last_emu_time: Duration,
    last_present_time: Duration,
}

impl Default for ViewerStats {
    fn default() -> Self {
        Self {
            emulated_frames: 0,
            presented_frames: 0,
            dropped_deadlines: 0,
            catchup_frames: 0,
            last_emu_time: Duration::ZERO,
            last_present_time: Duration::ZERO,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct PendingPresent {
    frame_index: u64,
    ly_ready: u8,
    mode_ready: u8,
    if_reg: u8,
    emu_time: Duration,
    catchup_in_cycle: u32,
}

#[derive(Debug, Clone)]
enum CaptureDecision {
    Saved(String),
    SkippedBeforeWindow,
    SkippedQuotaReached,
    SkippedLcdOff,
    Failed(String),
}

struct App {
    // Window + surface
    window: Option<Arc<Window>>,
    pixels: Option<Pixels<'static>>,

    // Emulator
    cpu: Box<CPU>,
    bus: Box<MemoryBus>,
    fb: Framebuffer,

    // Runtime
    config: ViewerConfig,
    run_state: RunState,
    stats: ViewerStats,
    next_deadline: Instant,
    redraw_requested: bool,
    pending_present: Option<PendingPresent>,

    overlays: OverlayToggles,
    capture: CaptureState,
}

impl App {
    fn new(rom_path: String) -> Result<Self> {
        let config = ViewerConfig::from_env();

        let mut cpu = Box::new(CPU::new());
        let mut bus = Box::new(MemoryBus::new());
        post_boot_init(&mut cpu, &mut bus);

        // Load ROM
        let rom = std::fs::read(rom_path)?;
        bus.load_rom(&rom);
        apply_overlay(&mut bus, &OverlayToggles::default());

        let run_state = if config.pause_on_start { RunState::Paused } else { RunState::Running };
        let initial_deadline = Instant::now() + config.frame_dt;

        Ok(Self {
            window: None,
            pixels: None,
            cpu,
            bus,
            fb: [[0; W]; H],
            config,
            run_state,
            stats: ViewerStats::default(),
            next_deadline: initial_deadline,
            redraw_requested: false,
            pending_present: None,
            overlays: OverlayToggles::default(),
            capture: CaptureState::default(),
        })
    }

    #[inline]
    fn window_id(&self) -> Option<WindowId> {
        self.window.as_ref().map(|w| w.id())
    }

    fn save_png<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        save_png(&self.fb, path)
    }

    fn update_control_flow(&self, event_loop: &winit::event_loop::ActiveEventLoop) {
        use winit::event_loop::ControlFlow;
        match self.run_state {
            RunState::Running | RunState::StepOneFrame => {
                event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_deadline));
            }
            RunState::Paused => {
                event_loop.set_control_flow(ControlFlow::Wait);
            }
            RunState::ExitRequested => {
                event_loop.exit();
            }
        }
    }

    fn handle_key(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        physical_key: PhysicalKey,
        state: ElementState
    ) {
        let mut overlay_changed = false;

        let pressed = state == ElementState::Pressed;

        // Game Boy buttons
        let gb_button = match physical_key {
            PhysicalKey::Code(KeyCode::ArrowRight) => Some(Button::Right),
            PhysicalKey::Code(KeyCode::ArrowLeft) => Some(Button::Left),
            PhysicalKey::Code(KeyCode::ArrowUp) => Some(Button::Up),
            PhysicalKey::Code(KeyCode::ArrowDown) => Some(Button::Down),
            PhysicalKey::Code(KeyCode::KeyZ) => Some(Button::A),
            PhysicalKey::Code(KeyCode::KeyX) => Some(Button::B),
            PhysicalKey::Code(KeyCode::Enter) => Some(Button::Start),
            PhysicalKey::Code(KeyCode::ShiftRight) => Some(Button::Select),
            _ => None,
        };
        if let Some(button) = gb_button {
            if pressed {
                self.bus.press_button(button);
            } else {
                self.bus.release_button(button);
            }
            return; // don't fall through to emulator hotkeys
        }

        if !pressed {
            return;
        }

        match physical_key {
            PhysicalKey::Code(KeyCode::Space) => {
                self.run_state = match self.run_state {
                    RunState::Running => RunState::Paused,
                    RunState::Paused => {
                        self.next_deadline = Instant::now() + self.config.frame_dt;
                        RunState::Running
                    }
                    RunState::StepOneFrame => RunState::Paused,
                    RunState::ExitRequested => RunState::ExitRequested,
                };
            }
            PhysicalKey::Code(KeyCode::KeyM) => {
                let muted = self.bus.toggle_mute();
                eprintln!("Audio {}", if muted { "muted" } else { "unmuted" });
            }
            PhysicalKey::Code(KeyCode::KeyN) => {
                if self.run_state == RunState::Paused {
                    self.run_state = RunState::StepOneFrame;
                    self.next_deadline = Instant::now();
                }
            }
            PhysicalKey::Code(KeyCode::KeyR) => {
                self.next_deadline = Instant::now() + self.config.frame_dt;
            }
            PhysicalKey::Code(KeyCode::KeyG) => {
                self.overlays.grid = !self.overlays.grid;
                overlay_changed = true;
            }
            PhysicalKey::Code(KeyCode::KeyW) => {
                self.overlays.window = !self.overlays.window;
                overlay_changed = true;
            }
            PhysicalKey::Code(KeyCode::KeyA) => {
                self.overlays.axes = !self.overlays.axes;
                overlay_changed = true;
            }
            PhysicalKey::Code(KeyCode::KeyS) => {
                self.overlays.sprites = !self.overlays.sprites;
                overlay_changed = true;
            }
            PhysicalKey::Code(KeyCode::F9) => {
                self.capture.screenshot_id = self.capture.screenshot_id.wrapping_add(1);
                let filename = format!("acid2-capture-{:03}.png", self.capture.screenshot_id);
                match self.save_png(&filename) {
                    Ok(()) => eprintln!("Saved {}", filename),
                    Err(e) => eprintln!("Save failed: {e}"),
                }
            }
            PhysicalKey::Code(KeyCode::Escape) => {
                self.run_state = RunState::ExitRequested;
                event_loop.exit();
            }
            _ => {}
        }

        if overlay_changed {
            apply_overlay(&mut self.bus, &self.overlays);
        }
    }

    fn capture_if_requested(
        &mut self,
        frame_index: u64,
        lcdc: u8,
        ly_ready: u8,
        mode_ready: u8
    ) -> CaptureDecision {
        if self.capture.frame_count <= self.config.capture_skip {
            return CaptureDecision::SkippedBeforeWindow;
        }
        if self.capture.captured_so_far >= self.config.capture_first_n {
            return CaptureDecision::SkippedQuotaReached;
        }
        if self.config.capture_require_lcd_on && (lcdc & 0x80) == 0 {
            return CaptureDecision::SkippedLcdOff;
        }

        let idx = self.capture.captured_so_far + 1;
        let filename = format!("{}-{:03}.png", self.config.capture_prefix, idx);
        match self.save_png(&filename) {
            Ok(()) => {
                self.capture.captured_so_far = self.capture.captured_so_far.saturating_add(1);
                if self.config.trace_structured {
                    eprintln!(
                        "[TRACE] capture frame={} file={} lcdc={:02X} ly={} mode={}",
                        frame_index,
                        filename,
                        lcdc,
                        ly_ready,
                        mode_ready
                    );
                }
                CaptureDecision::Saved(filename)
            }
            Err(e) => CaptureDecision::Failed(e.to_string()),
        }
    }

    fn emulate_one_frame(&mut self, catchup_in_cycle: u32) {
        let emu_start = Instant::now();
        self.stats.emulated_frames = self.stats.emulated_frames.saturating_add(1);

        let mut tcycles: u32 = 0;
        while tcycles < DMG_DOTS_PER_FRAME {
            let cy = self.cpu.step(&mut self.bus);
            tcycles = tcycles.saturating_add(cy);
        }
        self.stats.last_emu_time = emu_start.elapsed();

        if self.bus.gpu.take_frame_ready() {
            self.capture.frame_count = self.capture.frame_count.saturating_add(1);
            self.bus.gpu.copy_frame(&mut self.fb);

            let ly_ready = self.bus.gpu.ly();
            let mode_ready = self.bus.gpu.mode_code();
            let if_reg = self.bus.read_byte(0xff0f);
            let lcdc = self.bus.read_byte(0xff40);
            let frame_index = self.bus.gpu
                .take_last_frame_telemetry()
                .map(|t| t.frame_index)
                .unwrap_or(self.capture.frame_count as u64);

            let decision = self.capture_if_requested(frame_index, lcdc, ly_ready, mode_ready);
            if !self.config.trace_structured {
                match decision {
                    CaptureDecision::Saved(name) => eprintln!("Captured {}", name),
                    CaptureDecision::Failed(msg) => eprintln!("Capture failed: {}", msg),
                    | CaptureDecision::SkippedBeforeWindow
                    | CaptureDecision::SkippedQuotaReached
                    | CaptureDecision::SkippedLcdOff => {}
                }
            }

            self.pending_present = Some(PendingPresent {
                frame_index,
                ly_ready,
                mode_ready,
                if_reg,
                emu_time: self.stats.last_emu_time,
                catchup_in_cycle,
            });

            if !self.redraw_requested {
                if let Some(win) = &self.window {
                    win.request_redraw();
                    self.redraw_requested = true;
                }
            }
        }
    }

    fn emulate_until_deadline(&mut self, now: Instant) {
        match self.run_state {
            RunState::Running => {
                let mut catchup: u32 = 0;
                while now >= self.next_deadline && catchup < self.config.max_catchup_frames {
                    self.emulate_one_frame(catchup);
                    if catchup > 0 {
                        self.stats.catchup_frames = self.stats.catchup_frames.saturating_add(1);
                    }
                    self.next_deadline += self.config.frame_dt;
                    catchup = catchup.saturating_add(1);
                }

                if now >= self.next_deadline {
                    let mut dropped: u64 = 0;
                    while now >= self.next_deadline {
                        self.next_deadline += self.config.frame_dt;
                        dropped = dropped.saturating_add(1);
                    }
                    self.stats.dropped_deadlines =
                        self.stats.dropped_deadlines.saturating_add(dropped);
                    if self.config.trace_structured {
                        eprintln!(
                            "[TRACE] dropped_deadlines={} total={}",
                            dropped,
                            self.stats.dropped_deadlines
                        );
                    }
                }
            }
            RunState::StepOneFrame => {
                self.emulate_one_frame(0);
                self.next_deadline = now + self.config.frame_dt;
                self.run_state = RunState::Paused;
            }
            RunState::Paused | RunState::ExitRequested => {}
        }
    }

    fn try_present(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        let meta = match self.pending_present.take() {
            Some(m) => m,
            None => {
                self.redraw_requested = false;
                return;
            }
        };

        let present_start = Instant::now();

        if let Some(pixels) = &mut self.pixels {
            let frame: &mut [u8] = pixels.frame_mut();
            for y in 0..H {
                for x in 0..W {
                    let v = dmg_shade_to_u8(self.fb[y][x]);
                    let i = (y * W + x) * 4;
                    frame[i + 0] = v;
                    frame[i + 1] = v;
                    frame[i + 2] = v;
                    frame[i + 3] = 0xff;
                }
            }
            if let Err(e) = pixels.render() {
                eprintln!("pixels.render() failed: {e}");
                self.run_state = RunState::ExitRequested;
                event_loop.exit();
                return;
            }
        }

        self.stats.last_present_time = present_start.elapsed();
        self.stats.presented_frames = self.stats.presented_frames.saturating_add(1);
        self.redraw_requested = false;

        if self.config.trace_structured {
            eprintln!(
                "[TRACE] frame={} ly={} mode={} if={:02X} emu_ms={:.3} present_ms={:.3} catchup={}",
                meta.frame_index,
                meta.ly_ready,
                meta.mode_ready,
                meta.if_reg,
                meta.emu_time.as_secs_f64() * 1000.0,
                self.stats.last_present_time.as_secs_f64() * 1000.0,
                meta.catchup_in_cycle
            );
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if self.window.is_none() {
            let scale: u32 = 4;
            let attrs = Window::default_attributes()
                .with_title("GB Viewer")
                .with_inner_size(
                    LogicalSize::new(((W as u32) * scale) as f64, ((H as u32) * scale) as f64)
                )
                .with_min_inner_size(LogicalSize::new(W as f64, H as f64));

            let window = event_loop.create_window(attrs).expect("failed to create window");
            let window = Arc::new(window);

            let size = window.inner_size();
            let surface = SurfaceTexture::new(size.width, size.height, window.clone());
            let pixels = Pixels::new(W as u32, H as u32, surface).expect(
                "failed to create Pixels surface"
            );

            self.window = Some(window.clone());
            self.pixels = Some(pixels);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent
    ) {
        if self.window_id() != Some(window_id) {
            return;
        }

        match event {
            WindowEvent::CloseRequested => {
                self.run_state = RunState::ExitRequested;
                event_loop.exit();
            }

            WindowEvent::Resized(size) => {
                if let Some(pixels) = &mut self.pixels {
                    if let Err(e) = pixels.resize_surface(size.width, size.height) {
                        eprintln!("pixels.resize_surface failed: {e}");
                    }
                    if let Err(e) = pixels.resize_buffer(W as u32, H as u32) {
                        eprintln!("pixels.resize_buffer failed: {e}");
                    }
                }
            }

            WindowEvent::RedrawRequested => {
                self.try_present(event_loop);
            }

            WindowEvent::KeyboardInput { event: KeyEvent { physical_key, state, .. }, .. } => {
                self.handle_key(event_loop, physical_key, state);
            }

            _ => {}
        }
    }

    fn new_events(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop, _cause: StartCause) {
        // Intentionally empty: cadence handled in about_to_wait()
    }

    fn about_to_wait(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.update_control_flow(event_loop);

        let now = Instant::now();
        match self.run_state {
            RunState::Running => {
                if now >= self.next_deadline {
                    self.emulate_until_deadline(now);
                }
            }
            RunState::StepOneFrame => {
                self.emulate_until_deadline(now);
            }
            RunState::Paused | RunState::ExitRequested => {}
        }
    }
}

fn main() -> Result<()> {
    let rom_path = std::env::args().nth(1).expect("Usage: viewer <path.gb>");
    let mut app = App::new(rom_path)?;
    let event_loop = EventLoop::new()?;
    event_loop.run_app(&mut app)?;
    Ok(())
}

// ----- helpers -----
fn apply_overlay(bus: &mut MemoryBus, overlays: &OverlayToggles) {
    let mut cfg = DebugOverlayConfig::default();
    cfg.show_bg_tile_grid = overlays.grid;
    cfg.show_window_bounds = overlays.window;
    cfg.show_bg_axes = overlays.axes;
    cfg.show_sprite_boxes = overlays.sprites;
    cfg.shade_grid = 1;
    cfg.shade_window = 1;
    cfg.shade_axes = 1;
    cfg.shade_sprite_box = 2;
    bus.set_gpu_debug_config(cfg);
}

fn post_boot_init(cpu: &mut CPU, bus: &mut MemoryBus) {
    cpu.regs.set_af(0x01b0);
    cpu.regs.set_bc(0x0013);
    cpu.regs.set_de(0x00d8);
    cpu.regs.set_hl(0x014d);
    cpu.sp = 0xfffe;
    cpu.pc = 0x0100;

    // Initial LCD state
    bus.write_byte(0xff40, 0x91); // LCDC
    bus.write_byte(0xff42, 0x00); // SCY
    bus.write_byte(0xff43, 0x00); // SCX
    bus.write_byte(0xff47, 0xfc); // BGP
    bus.write_byte(0xff4a, 0x00); // WY
    bus.write_byte(0xff4b, 0x00); // WX
}
