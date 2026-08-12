use std::path::Path;

fn main() {
    // Tell rustc's cfg checker about the custom cfg so `#[cfg(webui_dist)]` doesn't warn
    println!("cargo:rustc-check-cfg=cfg(webui_dist)");

    // If webui/dist exists at build time, set a cfg flag for conditional compilation
    if Path::new("webui/dist").exists() {
        println!("cargo:rustc-cfg=webui_dist");
    } else {
        println!(
            "cargo:warning=webui/dist not found, web-embed feature will have no embedded assets. \
             Run `cd webui && npm run build` first."
        );
    }
}
