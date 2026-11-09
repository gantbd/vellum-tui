#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    vellum_iced::run(std::env::args_os().skip(1).collect())
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("vellum-desktop currently requires the macOS adapter profile");
    std::process::exit(1);
}
