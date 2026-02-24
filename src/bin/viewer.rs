// src/bin/viewer.rs
use gb_emulator::{ CPU, MemoryBus };
use gb_emulator::gpu::DebugOverlayConfig;

use anyhow::Result;
use pixels::{ Pixels, SurfaceTexture };
use png::{ BitDepth, ColorType, Encoder };
use std::fs::File;
use std::path::Path;
use std::sync::Arc;
use std::time::{ Duration, Instant };
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::{ ElementState, KeyEvent, StartCause, WindowEvent },
    event_loop::EventLoop, // keep ControlFlow path-qualified to avoid warnings
    keyboard::{ KeyCode, PhysicalKey },
    window::{ Window, WindowId },
};

const W: usize = 160;
const H: usize = 144;
type Framebuffer = [[u8; W]; H];

// DMG timing (one LCD frame)
const DMG_DOTS_PER_FRAME: u32 = 70_224;
const DMG_CLOCK_HZ: u32 = 4_194_304;

// ~16.743 ms (exact ratio w/out fp drift)
const NANOS_PER_SEC: u128 = 1_000_000_000;
const FRAME_NS: u128 = (NANOS_PER_SEC * (DMG_DOTS_PER_FRAME as u128)) / (DMG_CLOCK_HZ as u128);
const FRAME_DT: Duration = Duration::from_nanos(FRAME_NS as u64);

#[inline]
fn dmg_shade_to_u8(v: u8) -> u8 {
    match v & 0b11 {
        0 => 255,
        1 => 170,
        2 => 85,
        _ => 0,
    }
}

struct App {
    // Window + surface
    window: Option<Arc<Window>>,
    pixels: Option<Pixels<'static>>,

    // Emulator
    cpu: Box<CPU>,
    bus: Box<MemoryBus>,
    fb: Framebuffer,

    // Overlays
    o_grid: bool,
    o_window: bool,
    o_axes: bool,
    o_sprites: bool,

    // Screenshots
    screenshot_id: u32,

    // Frame cadence & state
    next_deadline: Instant,
    needs_present: bool,

    // Guards to prevent re-entrancy
    in_redraw: bool,
    in_compute: bool,

    // Capture control
    frame_count: u32,
    capture_first_n: u32,
    captured_so_far: u32,
    capture_prefix: String,
    capture_skip: u32,
}

impl App {
    fn new(rom_path: String) -> Result<Self> {
        let mut cpu = Box::new(CPU::new());
        let mut bus = Box::new(MemoryBus::new());
        post_boot_init(&mut cpu, &mut bus);

        // Overlays default; toggle via hotkeys
        let (o_grid, o_window, o_axes, o_sprites) = (true, true, false, true);

        // Load ROM
        let rom = std::fs::read(rom_path)?;
        bus.load_rom(&rom);

        // Optional auto-capture controls via env
        let capture_first_n = std::env
            ::var("GB_CAPTURE_FIRST_N")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(10);

        let capture_prefix = std::env
            ::var("GB_CAPTURE_PREFIX")
            .unwrap_or_else(|_| "boot-frame".into());

        Ok(Self {
            window: None,
            pixels: None,
            cpu,
            bus,
            fb: [[0; W]; H],
            o_grid,
            o_window,
            o_axes,
            o_sprites,
            screenshot_id: 0,
            next_deadline: Instant::now() + FRAME_DT, // cadence kicks in shortly
            needs_present: false,
            in_redraw: false,
            in_compute: false,
            frame_count: 0,
            capture_first_n,
            captured_so_far: 0,
            capture_skip: 8, // skip first N frames before capturing
            capture_prefix,
        })
    }

    #[inline]
    fn window_id(&self) -> Option<WindowId> {
        self.window.as_ref().map(|w| w.id())
    }

    fn save_png<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let mut gray = vec![0u8; W * H];
        for y in 0..H {
            for x in 0..W {
                gray[y * W + x] = dmg_shade_to_u8(self.fb[y][x]);
            }
        }
        let file = File::create(path.as_ref())?;
        let mut enc = Encoder::new(file, W as u32, H as u32);
        enc.set_color(ColorType::Grayscale);
        enc.set_depth(BitDepth::Eight);
        let mut writer = enc.write_header()?;
        writer.write_image_data(&gray)?;
        Ok(())
    }

    /// Uploads the current framebuffer to the Pixels surface and renders it.
    fn present_current_frame(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        // Copy finished frame from PPU (we only copy if we decided to present)
        self.bus.gpu.copy_frame(&mut self.fb);

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
                event_loop.exit();
            }
        }
    }

    /// Run CPU/PPU for one full frame of PPU dots. Returns true if a frame should be presented.
    fn step_one_frame(&mut self) -> bool {
        let mut tcycles: u32 = 0;
        let mut saw_frame_ready = false;

        while tcycles < DMG_DOTS_PER_FRAME {
            if !saw_frame_ready && self.bus.gpu.frame_is_ready() {
                saw_frame_ready = true;
            }
            let cy = self.cpu.step(&mut self.bus);
            tcycles = tcycles.saturating_add(cy);
        }

        let mut should_present = false;

        if saw_frame_ready && self.bus.gpu.take_frame_ready() {
            self.frame_count = self.frame_count.saturating_add(1);
            should_present = true;

            // Optional: Skip first N frames, then capture next M frames
            if self.frame_count > self.capture_skip && self.captured_so_far < self.capture_first_n {
                // Copy frame (we will present anyway, but we capture now from the fresh PPU buffer)
                self.bus.gpu.copy_frame(&mut self.fb);
                let idx = self.captured_so_far + 1;
                let filename = format!("{}-{:03}.png", self.capture_prefix, idx);
                match self.save_png(&filename) {
                    Ok(()) => {
                        #[cfg(any(feature = "trace_ppu", feature = "debug_timing"))]
                        eprintln!("Captured {}", filename);
                    }
                    Err(e) => eprintln!("Capture failed: {e}"),
                }
                self.captured_so_far = self.captured_so_far.saturating_add(1);
            }
        }

        #[cfg(any(feature = "trace_ppu", feature = "debug_timing"))]
        {
            eprintln!(
                "frame done: should_present={}, level_now={}, ly={}",
                should_present,
                self.bus.gpu.frame_is_ready(),
                self.bus.gpu.ly()
            );
            eprintln!("LYC={} STAT={:02X}", self.bus.read_byte(0xff45), self.bus.read_byte(0xff41));
        }

        should_present
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if self.window.is_none() {
            let scale: u32 = 4;
            let attrs = Window::default_attributes()
                .with_title("GB Viewer (decoupled cadence)")
                .with_inner_size(
                    LogicalSize::new(((W as u32) * scale) as f64, ((H as u32) * scale) as f64)
                )
                .with_min_inner_size(LogicalSize::new(W as f64, H as f64));

            let window = event_loop.create_window(attrs).expect("failed to create window");
            let window = Arc::new(window);

            // Pixels<'static> via Arc<Window> clone to SurfaceTexture
            let size = window.inner_size();
            let surface = SurfaceTexture::new(size.width, size.height, window.clone());
            let pixels = Pixels::new(W as u32, H as u32, surface).expect(
                "failed to create Pixels surface"
            );

            self.window = Some(window.clone());
            self.pixels = Some(pixels);

            // Optional one-time kick; cadence will take over anyway
            window.request_redraw();
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
                event_loop.exit();
            }

            WindowEvent::Resized(size) => {
                if let Some(pixels) = &mut self.pixels {
                    if let Err(e) = pixels.resize_surface(size.width, size.height) {
                        eprintln!("pixels.resize_surface failed: {e}");
                    }
                }
            }

            WindowEvent::RedrawRequested => {
                // Guard against nested delivery of RedrawRequested
                if self.in_redraw {
                    return; // Skip nested redraw to avoid stack growth
                }
                self.in_redraw = true;

                if self.needs_present {
                    self.needs_present = false;
                    self.present_current_frame(event_loop);
                }

                self.in_redraw = false;
            }

            WindowEvent::KeyboardInput { event: KeyEvent { physical_key, state, .. }, .. } => {
                if state == ElementState::Pressed {
                    let mut overlay_changed = false;
                    match physical_key {
                        PhysicalKey::Code(KeyCode::KeyG) => {
                            self.o_grid = !self.o_grid;
                            overlay_changed = true;
                        }
                        PhysicalKey::Code(KeyCode::KeyW) => {
                            self.o_window = !self.o_window;
                            overlay_changed = true;
                        }
                        PhysicalKey::Code(KeyCode::KeyA) => {
                            self.o_axes = !self.o_axes;
                            overlay_changed = true;
                        }
                        PhysicalKey::Code(KeyCode::KeyS) => {
                            self.o_sprites = !self.o_sprites;
                            overlay_changed = true;
                        }
                        PhysicalKey::Code(KeyCode::F9) => {
                            self.screenshot_id = self.screenshot_id.wrapping_add(1);
                            // Copy latest frame before saving
                            self.bus.gpu.copy_frame(&mut self.fb);
                            let filename = format!("acid2-capture-{:03}.png", self.screenshot_id);
                            match self.save_png(&filename) {
                                Ok(()) => eprintln!("Saved {}", filename),
                                Err(e) => eprintln!("Save failed: {e}"),
                            }
                        }
                        PhysicalKey::Code(KeyCode::Escape) => event_loop.exit(),
                        _ => {}
                    }

                    if overlay_changed {
                        apply_overlay(
                            &mut self.bus,
                            self.o_grid,
                            self.o_window,
                            self.o_axes,
                            self.o_sprites
                        );
                    }
                }
            }

            _ => {}
        }
    }

    fn new_events(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop, _cause: StartCause) {
        // Intentionally empty: cadence & compute handled in about_to_wait()
    }

    fn about_to_wait(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        // Drive cadence (decoupled compute from redraw)
        event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(self.next_deadline));
        let now = Instant::now();

        if now >= self.next_deadline {
            // Avoid re-entrant compute if platform schedules tightly
            if self.in_compute {
                self.next_deadline = now + FRAME_DT;
                return;
            }
            self.in_compute = true;

            // Step one whole frame worth of work
            let present = self.step_one_frame();

            if present {
                self.needs_present = true;
                if let Some(win) = &self.window {
                    win.request_redraw();
                }
            }

            self.next_deadline = now + FRAME_DT;
            self.in_compute = false;
        }
    }
}

fn main() -> Result<()> {
    let rom_path = std::env::args().nth(1).expect("Usage: viewer <path.gb>");
    let mut app = App::new(rom_path)?; // propagate errors
    let event_loop = EventLoop::new()?; // winit 0.30.x returns Result
    event_loop.run_app(&mut app)?; // returns Result
    Ok(())
}

// ----- helpers -----
fn apply_overlay(bus: &mut MemoryBus, grid: bool, window: bool, axes: bool, sprites: bool) {
    let mut cfg = DebugOverlayConfig::default();
    cfg.show_bg_tile_grid = grid;
    cfg.show_window_bounds = window;
    cfg.show_bg_axes = axes;
    cfg.show_sprite_boxes = sprites;
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
