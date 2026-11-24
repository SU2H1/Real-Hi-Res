//! Real-Hi-Res desktop app (Rust).  `realhires-gui [PATH...]`: paths given on
//! the command line are added and scanned at once.

#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod dialogs;
mod lyrics;
mod plots;
mod style;
mod work;

const ICON: &[u8] = include_bytes!("../../../../realhires/assets/icon.png");

struct GuiApp(app::App);

impl eframe::App for GuiApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.0.ui(ui);
    }
}

fn icon() -> Option<egui::IconData> {
    let img = image::load_from_memory(ICON).ok()?.to_rgba8();
    Some(egui::IconData { width: img.width(), height: img.height(), rgba: img.into_raw() })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version") {
        println!("realhires-gui {}", realhires::VERSION);
        return;
    }
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("usage: realhires-gui [PATH...]\n\nThe Real-Hi-Res desktop app. Folders or files given here are added and scanned at once.");
        return;
    }
    let cfg = realhires::settings::load(None);
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Real-Hi-Res")
        .with_inner_size([1280.0, 880.0])
        .with_min_inner_size([900.0, 600.0])
        .with_drag_and_drop(true);
    if let Some(i) = icon() {
        viewport = viewport.with_icon(i);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    let res = eframe::run_native(
        "Real-Hi-Res",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_theme(egui::Theme::Light);
            Ok(Box::new(GuiApp(app::App::new(&cc.egui_ctx, cfg, args))))
        }),
    );
    if let Err(e) = res {
        eprintln!(
            "Real-Hi-Res cannot open its window: {e}\nThe desktop app needs a graphical desktop (on Linux X11 or Wayland). The \
             command-line tool `realhires` works without one."
        );
        std::process::exit(1);
    }
}
