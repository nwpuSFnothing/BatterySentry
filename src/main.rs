#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod app;
mod config;
mod drivers;
mod hid;
mod panel;
mod protocol;
mod startup;
mod taskbar;
mod theme;
mod tray;
mod ui;
mod win;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help") {
        app::console();
        println!(
            "BatterySentry\n  --background   Start on taskbar\n  --once         Read batteries and exit\n  --list         List HID interfaces\n  --quit         Stop running instance\n  --help         Show help"
        );
        return;
    }
    if args.iter().any(|a| a == "--list") {
        app::console();
        match hid::enumerate() {
            Ok(devices) => {
                for d in devices {
                    println!(
                        "{:04X}:{:04X} page={:04X} usage={:04X} in={} out={} feature={} {}\n  {}",
                        d.vid,
                        d.pid,
                        d.page,
                        d.usage,
                        d.input_len,
                        d.output_len,
                        d.feature_len,
                        d.product,
                        d.path
                    );
                }
            }
            Err(e) => eprintln!("{e}"),
        }
        return;
    }
    if args.iter().any(|a| a == "--once") {
        app::console();
        let readings = match drivers::scan() {
            Ok(readings) => readings,
            Err(e) => {
                eprintln!("{e}");
                return;
            }
        };
        for r in readings {
            println!(
                "{}: {} [{}] {}",
                r.name,
                r.value_text(),
                r.identity,
                r.detail
            );
        }
        return;
    }
    if let Err(e) = app::run(&args) {
        if !args.iter().any(|arg| arg == "--background") {
            app::show_error(&e.to_string());
        }
        std::process::exit(1);
    }
}
