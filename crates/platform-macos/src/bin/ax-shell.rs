//! Minimal macOS menu-bar shell (Gap 3).
//!
//! Runs the `NSStatusItem` shell in `hypr_rust_platform_macos::shell`. The menu
//! carries a status line, an explicit reflow command, pause/resume, a settings
//! summary, reset-to-defaults, an Accessibility status line, diagnostics and
//! quit. Auto-tiling stays off until the user picks a reflow command.
//!
//! Extra modes (no run loop):
//! - `--self-test` exercises the shell: it creates the app, status item and
//!   menu, reports the menu contents, then runs one reflow. Host evidence
//!   without a mouse click.
//! - `--reset-defaults` writes the default config to `~/.hypr-rust/config.json`.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--reset-defaults") {
        match hypr_rust_platform_macos::config::Config::default().save() {
            Ok(()) => println!("ok: wrote the default config"),
            Err(error) => {
                eprintln!("error: {error}");
                std::process::exit(1);
            }
        }
    } else if args.iter().any(|arg| arg == "--self-test") {
        hypr_rust_platform_macos::shell::self_test();
    } else {
        hypr_rust_platform_macos::shell::run();
    }
}
