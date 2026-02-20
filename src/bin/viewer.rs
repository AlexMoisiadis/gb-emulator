// src/bin/viewer.rs
use gb_emulator::{ CPU, MemoryBus };
use gb_emulator::gpu::DebugOverlayConfig;

use pixels::{ Pixels, SurfaceTexture };
use std::sync::Arc;
use std::{ fs::File, path::Path };
use png::{ Encoder, ColorType, BitDepth };
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::{ ElementState, KeyEvent, WindowEvent },
    event_loop::EventLoop,
    keyboard::{ KeyCode, PhysicalKey },
    window::{ Window, WindowId },
};

const W: usize = 160;
const H: usize = 144;
type Framebuffer = [[u8; W]; H];

const LY_ADDR: u16 = 0xff44; // LY register
const STAT_ADDR: u16 = 0xff41; // STAT register; bits[1:0] = mode (0..3)
const FRAME_DOTS: u32 = 70_224; // 154 * 456  (≈59.7 Hz)  [1](https://www.reddit.com/r/EmuDev/comments/10orf0d/question_about_the_gameboy_window_xcoordinate/)
struct App {
    window: Option<Arc<Window>>,
    pixels: Option<Pixels<'static>>,
    // Emulator
    cpu: CPU,
    bus: MemoryBus,
    fb: Framebuffer,
    // Overlays
    o_grid: bool,
    o_window: bool,
    o_axes: bool,
    o_sprites: bool,
    prev_ly: u8, // Last LY we saw (0.=153)
    in_vblank: bool,
    screenshot_id: u32,
}

impl App {
    fn new(rom_path: String) -> anyhow::Result<Self> {
        let mut cpu = CPU::new();
        let mut bus = MemoryBus::new();
        post_boot_init(&mut cpu, &mut bus);

        // Default overlays; hotkeys toggle them at runtime
        let (o_grid, o_window, o_axes, o_sprites) = (true, true, false, true);
        apply_overlay(&mut bus, o_grid, o_window, o_axes, o_sprites);

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
            prev_ly: 0,
            in_vblank: false,
            screenshot_id: 0,
        })
    }

    fn save_png<P: AsRef<Path>>(&self, path: P) -> anyhow::Result<()> {
        const W: usize = 160;
        const H: usize = 144;

        // Convert shade indices (0..3) to 8-bit grayscale.
        let lut: [u8; 4] = [0xff, 0xaa, 0x55, 0x00]; // white..black
        let mut gray = vec![0u8; W * H];
        for y in 0..H {
            for x in 0..W {
                let idx = self.fb[y][x] as usize;
                gray[y * W + x] = lut.get(idx).copied().unwrap_or(0x00);
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

    fn window_id(&self) -> Option<WindowId> {
        self.window.as_ref().map(|w| w.id())
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if self.window.is_none() {
            // --- Create window via 0.30 API ---
            let scale: u32 = 4;
            let attrs = Window::default_attributes()
                .with_title("GB Viewer (winit 0.30 + pixels)")
                .with_inner_size(
                    LogicalSize::new(((W as u32) * scale) as f64, ((H as u32) * scale) as f64)
                )
                .with_min_inner_size(LogicalSize::new(W as f64, H as f64));

            let window = event_loop.create_window(attrs).expect("failed to create window");
            let window = Arc::new(window);

            // --- Create Pixels<'static> by owning the window handle (Arc clone) ---
            let size = window.inner_size();
            let surface = SurfaceTexture::new(size.width, size.height, window.clone());
            let pixels = Pixels::new(W as u32, H as u32, surface).expect(
                "failed to create Pixels surface"
            );

            self.window = Some(window.clone());
            self.pixels = Some(pixels);

            // Kick the first redraw
            window.request_redraw();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent
    ) {
        // Route only events for our window
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
                        // Optional: bail out if resize is critical
                        // event_loop.exit();
                    }
                }
            }

            WindowEvent::RedrawRequested => {
                // Step until we hit the **start** of VBlank (LY edge 143->144) exactly once
                let mut safety = 0u32;
                let mut presented = false;

                while !presented && safety < FRAME_DOTS {
                    let cy = self.cpu.step(&mut self.bus);
                    safety = safety.saturating_add(cy);

                    let ly = self.bus.read_byte(LY_ADDR);
                    let stat = self.bus.read_byte(STAT_ADDR);
                    let mode = stat & 0b11; // STAT mode: 0=HBlank,1=VBlank,2=OAM,3=Transfer  [1](https://www.reddit.com/r/EmuDev/comments/10orf0d/question_about_the_gameboy_window_xcoordinate/)

                    // Detect **rising edge** into VBlank: LY 143->144 OR mode changed to 1
                    let vblank_now = ly == 144 || mode == 0b01;
                    if !self.in_vblank && vblank_now {
                        // We just **entered** VBlank -> present exactly once
                        self.in_vblank = true;

                        // Take stable snapshot
                        self.bus.render_frame(&mut self.fb);

                        if let Some(pixels) = &mut self.pixels {
                            let frame = pixels.frame_mut();
                            for y in 0..H {
                                for x in 0..W {
                                    let v = match self.fb[y][x] & 0b11 {
                                        0 => 255,
                                        1 => 170,
                                        2 => 85,
                                        _ => 0,
                                    };
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
                                return;
                            }
                        }
                        presented = true;
                    }

                    // Detect **exit** from VBlank for the *next* frame: LY < 144 and mode != 1
                    if ly < 144 && mode != 0b01 {
                        self.in_vblank = false;
                    }

                    self.prev_ly = ly;
                }

                if let Some(win) = &self.window {
                    win.request_redraw();
                }
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
}

fn main() -> anyhow::Result<()> {
    // Parse ROM path
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
