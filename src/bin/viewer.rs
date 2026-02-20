// src/bin/viewer.rs
use gb_emulator::{ CPU, MemoryBus };
use gb_emulator::gpu::DebugOverlayConfig;

use pixels::{ Pixels, SurfaceTexture };
use std::sync::Arc;
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
        })
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
                // Step emulation (very rough pacing)
                for _ in 0..20_000 {
                    self.cpu.step(&mut self.bus);
                }
                self.bus.render_frame(&mut self.fb);

                // Upload to pixels
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
                    if pixels.render().is_err() {
                        eprintln!("pixels.render() failed; exiting");
                        event_loop.exit();
                        return;
                    }
                }

                // Request next frame (simple continuous redraw)
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
