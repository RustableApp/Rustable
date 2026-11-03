//! Native regression for an AppKit callback clearing the current GL context.
//! Run on macOS: cargo run --example macos_gl_context_recovery
//! The unpatched renderer aborts with "Unable to create Texture object".
#[cfg(target_os = "macos")]
fn main() -> eframe::Result {
    use eframe::egui;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[link(name = "OpenGL", kind = "framework")]
    unsafe extern "C" {
        fn CGLSetCurrentContext(context: *mut std::ffi::c_void) -> i32;
        fn CGLGetCurrentContext() -> *mut std::ffi::c_void;
    }

    struct Probe {
        frames: usize,
        texture: Option<egui::TextureHandle>,
        painted: Arc<AtomicUsize>,
    }

    impl eframe::App for Probe {
        fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
            if self.frames == 4 {
                assert_eq!(self.painted.load(Ordering::SeqCst), 4);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
            let texture = ctx.load_texture(
                format!("context-recovery-{}", self.frames),
                egui::ColorImage::filled([8, 8], egui::Color32::GREEN),
                egui::TextureOptions::NEAREST,
            );
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.image((texture.id(), egui::vec2(64.0, 64.0)));
                let painted = self.painted.clone();
                ui.painter().add(egui::PaintCallback {
                    rect: ui.max_rect(),
                    callback: Arc::new(eframe::egui_glow::CallbackFn::new(move |_, painter| {
                        use eframe::glow::HasContext as _;
                        // SAFETY: Runs synchronously on the main rendering thread.
                        unsafe {
                            assert!(!CGLGetCurrentContext().is_null());
                            assert_eq!(painter.gl().get_error(), eframe::glow::NO_ERROR);
                        }
                        painted.fetch_add(1, Ordering::SeqCst);
                    })),
                });
            });
            self.texture = Some(texture);
            self.frames += 1;
            // SAFETY: Clear only this process's current-thread context. The
            // renderer must rebind it before uploading the queued new texture.
            unsafe {
                assert_eq!(CGLSetCurrentContext(std::ptr::null_mut()), 0);
                assert!(CGLGetCurrentContext().is_null());
            }
            ctx.request_repaint();
        }
    }

    let painted = Arc::new(AtomicUsize::new(0));
    let observed = painted.clone();
    eframe::run_native(
        "GL context recovery regression",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([320.0, 160.0]),
            ..Default::default()
        },
        Box::new(move |_| {
            Ok(Box::new(Probe {
                frames: 0,
                texture: None,
                painted,
            }))
        }),
    )?;
    assert_eq!(observed.load(Ordering::SeqCst), 4);
    println!("Recovered and painted all four frames after clearing the GL context.");
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("This regression requires macOS and a graphical session.");
}
