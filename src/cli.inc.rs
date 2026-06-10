pub fn cli() -> clap::Command {
    clap::Command::new(env!("CARGO_PKG_NAME"))
        .bin_name(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .about("X-Ray wallpaper engine for Wayland")
        .arg(
            clap::Arg::new("debug")
                .long("debug")
                .global(true)
                .action(clap::ArgAction::SetTrue)
                .help("Run in foreground debug mode"),
        )
        .arg(
            clap::Arg::new("output")
                .long("output")
                .global(true)
                .value_name("NAME")
                .help("Filter to a specific output"),
        )
        .arg(
            clap::Arg::new("config")
                .long("config")
                .global(true)
                .value_name("PATH")
                .help("Custom config path"),
        )
        .arg(
            clap::Arg::new("supervisor")
                .long("supervisor")
                .global(true)
                .action(clap::ArgAction::SetTrue)
                .help("Enable supervisor mode (per-output child processes)"),
        )
        .arg(
            clap::Arg::new("systemd")
                .long("systemd")
                .global(true)
                .action(clap::ArgAction::SetTrue)
                .help("Run as a systemd service (logs to journald, no daemon detach)"),
        )
        .subcommand(clap::Command::new("on").about("Start the daemon in the background"))
        .subcommand(clap::Command::new("off").about("Stop the daemon"))
        .subcommand(clap::Command::new("kill").about("Alias for off"))
        .subcommand(clap::Command::new("restart").about("Restart the daemon"))
        .subcommand(clap::Command::new("status").about("Show daemon + output status"))
        .subcommand(clap::Command::new("outputs").about("List detected runtime outputs"))
        .subcommand(
            clap::Command::new("output-on")
                .about("Enable one output in config")
                .arg(
                    clap::Arg::new("output")
                        .long("output")
                        .required(true)
                        .value_name("NAME")
                        .help("Output name to enable"),
                ),
        )
        .subcommand(
            clap::Command::new("output-off")
                .about("Disable one output in config")
                .arg(
                    clap::Arg::new("output")
                        .long("output")
                        .required(true)
                        .value_name("NAME")
                        .help("Output name to disable"),
                ),
        )
        .subcommand(clap::Command::new("gui").about("Open the configuration GUI"))
        .subcommand(
            clap::Command::new("__run")
                .about("Internal: run in foreground")
                .hide(true)
                .arg(
                    clap::Arg::new("supervised")
                        .long("supervised")
                        .action(clap::ArgAction::SetTrue)
                        .hide(true),
                ),
        )
        .subcommand(
            clap::Command::new("__supervisor")
                .about("Internal: supervisor for per-output processes")
                .hide(true),
        )
}
