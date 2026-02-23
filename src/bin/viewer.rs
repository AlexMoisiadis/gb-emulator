// src/bin/viewer.rs
use gb_emulator::{ CPU, MemoryBus };
use gb_emulator::gpu::DebugOverlayConfig;

use std::sync::Arc;
use std::time::{ Duration, Instant };
use std::{ fs::File, path::Path };

use pixels::{ Pixels, SurfaceTexture };
use png::{ BitDepth, ColorType, Encoder };
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::{ ElementState, KeyEvent, StartCause, WindowEvent },
    event_loop::EventLoop, // intentionally not importing ControlFlow to avoid warnings
    keyboard::{ KeyCode, PhysicalKey },
    window::{ Window, WindowId },
};

const W: usize = 160;
const H: usize = 144;
type Framebuffer = [[u8; W]; H];

// DMG timing
const DMG_DOTS_PER_FRAME: u32 = 70_224;
const DMG_CLOCK_HZ: u32 = 4_194_304;

// ~16.743 ms without floating error
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

    screenshot_id: u32,

    // Frame cadence & state
    next_deadline: Instant,
    needs_present: bool,

    // Guards to prevent re-entrancy and nested compute
    in_redraw: bool,
    in_compute: bool,
    frame_count: u32,
}

impl App {
    fn new(rom_path: String) -> anyhow::Result<Self> {
        let mut cpu = Box::new(CPU::new());
        let mut bus = Box::new(MemoryBus::new());
        post_boot_init(&mut cpu, &mut bus);

        // Overlays default; toggle via hotkeys
        let (o_grid, o_window, o_axes, o_sprites) = (true, true, false, true);
        // apply_overlay(&mut bus, o_grid, o_window, o_axes, o_sprites);

        // Load ROM
        let rom = std::fs::read(rom_path)?;
        bus.load_rom(&rom);

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
        })
    }

    fn save_png<P: AsRef<Path>>(&self, path: P) -> anyhow::Result<()> {
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

    #[inline]
    fn window_id(&self) -> Option<WindowId> {
        self.window.as_ref().map(|w| w.id())
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

            // Optional one-time kick; you can comment it out if you prefer cadence-only
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
                        // event_loop.exit();
                    }
                }
            }

            WindowEvent::RedrawRequested => {
                // Guard against nested delivery of RedrawRequested
                if self.in_redraw {
                    // Skip nested redraw to avoid stack growth on platforms that re-enter
                    return;
                }
                self.in_redraw = true;

                // Only present if a frame is ready (compute happens in about_to_wait)
                if self.needs_present {
                    self.needs_present = false;

                    // Copy finished frame from PPU
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
                            self.in_redraw = false;
                            return;
                        }
                    }
                }

                self.in_redraw = false;
            }

            WindowEvent::KeyboardInput { event: KeyEvent { physical_key, state, .. }, .. } => {
                if state == ElementState::Pressed {
                    match physical_key {
                        PhysicalKey::Code(KeyCode::KeyG) => {
                            self.o_grid = !self.o_grid;
                        }
                        PhysicalKey::Code(KeyCode::KeyW) => {
                            self.o_window = !self.o_window;
                        }
                        PhysicalKey::Code(KeyCode::KeyA) => {
                            self.o_axes = !self.o_axes;
                        }
                        PhysicalKey::Code(KeyCode::KeyS) => {
                            self.o_sprites = !self.o_sprites;
                        }
                        PhysicalKey::Code(KeyCode::F9) => {
                            self.screenshot_id = self.screenshot_id.wrapping_add(1);
                            let filename = format!("acid2-capture-{:03}.png", self.screenshot_id);
                            match self.save_png(&filename) {
                                Ok(()) => eprintln!("Saved {}", filename),
                                Err(e) => eprintln!("Save failed: {e}"),
                            }
                        }
                        PhysicalKey::Code(KeyCode::Escape) => event_loop.exit(),
                        _ => {}
                    }
                    apply_overlay(
                        &mut self.bus,
                        self.o_grid,
                        self.o_window,
                        self.o_axes,
                        self.o_sprites
                    );
                }
            }

            _ => {}
        }
    }

    fn new_events(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop, _cause: StartCause) {
        // Intentionally empty: cadence & compute handled in about_to_wait()
    }

    fn about_to_wait(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        // Set deadline-based wakeup so winit doesn't sleep indefinitely
        event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(self.next_deadline));

        let now = Instant::now();
        if now >= self.next_deadline {
            if self.in_compute {
                self.next_deadline = now + FRAME_DT;
                return;
            }
            self.in_compute = true;

            let mut safety_tcycles = 0u32;
            while !self.bus.gpu.frame_is_ready() && safety_tcycles < 70_224 {
                let cy = self.cpu.step(&mut self.bus);
                safety_tcycles = safety_tcycles.saturating_add(cy);
            }

            // In about_to_wait, after the first frame:
            if safety_tcycles == 65664 || self.screenshot_id == 0 {
                eprintln!(
                    "LCDC={:02X} BGP={:02X} OBP0={:02X} OBP1={:02X} SCX={} SCY={} WX={} WY={}",
                    self.bus.gpu.get_lcdc(),
                    self.bus.gpu.get_bgp(),
                    self.bus.gpu.get_obp0(),
                    self.bus.gpu.get_obp1(),
                    self.bus.gpu.get_scx(),
                    self.bus.gpu.get_scy(),
                    self.bus.gpu.get_wx(),
                    self.bus.gpu.get_wy()
                );
                // Also dump first 32 bytes of OAM
                for i in 0..8 {
                    eprintln!(
                        "OAM[{}]: y={} x={} tile={} attr={:02X}",
                        i,
                        self.bus.gpu.read_oam(i * 4),
                        self.bus.gpu.read_oam(i * 4 + 1),
                        self.bus.gpu.read_oam(i * 4 + 2),
                        self.bus.gpu.read_oam(i * 4 + 3)
                    );
                }
                for i in 0..32u16 {
                    let byte = self.bus.read_byte(0x9c00 + i);
                    eprint!("{:02X} ", byte);
                }
                // Dump tile 163 (used by sprites per OAM) raw bytes
                for i in 0..16u16 {
                    let byte = self.bus.read_byte(0x8000 + 163 * 16 + i);
                    eprint!("{:02X} ", byte);
                }
                eprintln!(" <-- tile 163 pattern");
                eprintln!(" <-- 0x9C00 row 0");
            }

            // Temporary: print state after each frame
            eprintln!(
                "frame done: safety_tcycles={}, frame_ready={}, ly={}",
                safety_tcycles,
                self.bus.gpu.frame_is_ready(),
                self.bus.gpu.ly()
            );

            if self.bus.gpu.take_frame_ready() {
                self.frame_count += 1;
                self.needs_present = true;

                if self.bus.gpu.get_lcdc() == 0xf3 && self.frame_count > 5 {
                    let filename = format!("auto-lcdc-f3-{}.png", self.frame_count);
                    if let Err(e) = self.save_png(&filename) {
                        eprintln!("auto-save failed: {e}");
                    } else {
                        eprintln!("Auto-saved {filename}");
                        self.frame_count = 0;
                    }
                }
            }

            self.next_deadline = now + FRAME_DT;
            self.in_compute = false;

            // Request redraw AFTER releasing in_compute
            if self.needs_present {
                if let Some(win) = &self.window {
                    win.request_redraw();
                }
            }
        }
    }
}

fn main() -> anyhow::Result<()> {
    let rom_path = std::env::args().nth(1).expect("Usage: viewer <path.gb>");

    let mut app = App::new(rom_path)?; // propagate errors

    let event_loop = EventLoop::new()?; // returns Result in 0.30.x
    event_loop.run_app(&mut app)?; // also returns Result
    Ok(())
}

// ----- helpers -----

fn apply_overlay(bus: &mut MemoryBus, grid: bool, window: bool, axes: bool, sprites: bool) {
    let mut cfg = DebugOverlayConfig::default();
    cfg.show_bg_tile_grid = grid;
    cfg.show_window_bounds = window;
    cfg.show_bg_axes = axes;
    cfg.show_sprite_boxes = sprites;
    cfg.shade_grid = 3;
    cfg.shade_window = 2;
    cfg.shade_axes = 1;
    cfg.shade_sprite_box = 3;
    bus.set_gpu_debug_config(cfg);
}

fn post_boot_init(cpu: &mut CPU, bus: &mut MemoryBus) {
    cpu.regs.set_af(0x01b0);
    cpu.regs.set_bc(0x0013);
    cpu.regs.set_de(0x00d8);
    cpu.regs.set_hl(0x014d);
    cpu.sp = 0xfffe;
    cpu.pc = 0x0100;

    bus.write_byte(0xff40, 0x91); // LCDC
    bus.write_byte(0xff42, 0x00); // SCY
    bus.write_byte(0xff43, 0x00); // SCX
    bus.write_byte(0xff47, 0xfc); // BGP
    bus.write_byte(0xff4a, 0x00); // WY
    bus.write_byte(0xff4b, 0x00); // WX
}
