pub fn print_help() {
    println!("cava-bg - X-Ray wallpaper engine for Wayland");
    println!();
    println!("Commands:");
    println!("  cava-bg on                         Start the daemon in the background");
    println!("  cava-bg on --debug                 Run in foreground debug mode (no detach)");
    println!("  cava-bg on --output <name>         Start filtered to a specific output");
    println!("  cava-bg off                        Stop the daemon");
    println!("  cava-bg status                     Show daemon + output status");
    println!("  cava-bg outputs                    List detected runtime outputs");
    println!("  cava-bg output-on --output <name>  Enable one output in config");
    println!("  cava-bg output-off --output <name> Disable one output in config");
    println!("  cava-bg gui                        Open the configuration GUI");
    println!("  cava-bg on --config <path>         Start with a custom config");
    println!();
    println!("Compatibility aliases:");
    println!("  cava-bg kill               Alias for 'off'");
    println!("  cava-bg --config <path>    Run in the foreground");
}
