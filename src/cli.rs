include!("cli.inc.rs");

pub struct Cli {
    pub command: Option<Command>,
    pub debug: bool,
    pub output: Option<String>,
    pub config: Option<String>,
    pub supervisor: bool,
    pub supervised: bool,
    pub systemd: bool,
}

pub enum Command {
    On,
    Off,
    Kill,
    Restart,
    Status,
    Outputs,
    OutputOn { output: String },
    OutputOff { output: String },
    Gui,
    __Run,
    __Supervisor,
}

impl Cli {
    pub fn parse() -> Self {
        let matches = cli().get_matches();
        let command = matches.subcommand_name().map(|name| match name {
            "on" => Command::On,
            "off" => Command::Off,
            "kill" => Command::Kill,
            "restart" => Command::Restart,
            "status" => Command::Status,
            "outputs" => Command::Outputs,
            "output-on" => Command::OutputOn {
                output: matches
                    .subcommand_matches("output-on")
                    .unwrap()
                    .get_one::<String>("output")
                    .unwrap()
                    .clone(),
            },
            "output-off" => Command::OutputOff {
                output: matches
                    .subcommand_matches("output-off")
                    .unwrap()
                    .get_one::<String>("output")
                    .unwrap()
                    .clone(),
            },
            "gui" => Command::Gui,
            "__run" => Command::__Run,
            "__supervisor" => Command::__Supervisor,
            _ => unreachable!(),
        });
        Self {
            command,
            debug: matches.get_flag("debug"),
            output: matches.get_one::<String>("output").cloned(),
            config: matches.get_one::<String>("config").cloned(),
            supervisor: matches.get_flag("supervisor"),
            supervised: matches
                .subcommand_matches("__run")
                .map(|m| m.get_flag("supervised"))
                .unwrap_or(false),
            systemd: matches.get_flag("systemd"),
        }
    }
}
