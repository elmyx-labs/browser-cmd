//! Entry point: help, the version, one command against the warm daemon, or the daemon under `--serve [ADDR]`.

use crate::api::{cli, command, help, http, server};
use crate::{Error, Host, Result, Settings};

#[tokio::main(flavor = "current_thread")]
pub async fn run() -> i32 {
    let mut argv: Vec<String> = std::env::args().skip(1).collect();
    // An output flag before the command is as good as one after it.
    let flags = argv.iter().take_while(|arg| matches!(arg.as_str(), "--brief" | "--raw")).count();
    argv.rotate_left(flags);
    if argv == ["--version"] {
        println!("browser-cmd {}", env!("CARGO_PKG_VERSION"));
        return 0;
    }
    // Nothing asked: print help rather than start a browser.
    if argv.is_empty()
        || (command::find(&argv[0]).is_none() && argv.iter().any(|arg| arg == "--help" || arg == "-h"))
    {
        println!("{}", help::help());
        return 0;
    }
    let done = tokio::task::LocalSet::new()
        .run_until(async {
            match argv.first().is_some_and(|arg| command::is_command(arg)) {
                true => cli::run(&argv).await,
                false => daemon(&argv).await,
            }
        })
        .await;
    match done {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("browser-cmd: {error}");
            1
        }
    }
}

async fn daemon(argv: &[String]) -> Result<()> {
    let address = match argv {
        [serve] if serve == "--serve" => http::address(),
        [serve, address] if serve == "--serve" => address.clone(),
        [name, ..] if !name.starts_with('-') => {
            return Err(Error::Config(format!("unknown command {name}; see --help")));
        }
        _ => return Err(Error::Config(format!("unexpected arguments: {}; see --help", argv.join(" ")))),
    };
    crate::api::client::may_start_browser()?;
    let home =
        std::env::var_os("HOME").map_or_else(|| ".".into(), std::path::PathBuf::from).join(".browser-cmd");
    // The workspace its spawner is in, not the directory within it.
    let settings = Settings::from_env(&home, &crate::workspace::root(&std::env::current_dir()?))?;
    let (limits, token) = (http::Limits::from_env()?, http::token()?);
    let listener = tokio::net::TcpListener::bind(&address).await?;
    server::serve(listener, Host::new(settings), limits, token).await
}
