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
        println!("browser-cmd {}", http::BUILD);
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

/// Where browser-cmd keeps what outlives a command: `~/.browser-cmd`.
pub(crate) fn home() -> std::path::PathBuf {
    std::env::var_os("HOME").map_or_else(|| ".".into(), std::path::PathBuf::from).join(".browser-cmd")
}

/// The daemon's settings: its home under `$HOME`, its workspace the one its
/// spawner is in, not the directory within it.
pub(crate) fn settings() -> Result<Settings> {
    Settings::from_env(&home(), &crate::workspace::root(&std::env::current_dir()?))
}

/// What the daemon at `address` says outside any command — that it started, a
/// panic, the error it ended on — is kept here: it has no terminal.
pub(crate) fn log(address: &str) -> std::path::PathBuf {
    kept(address, "log")
}

/// Where the daemon at `address` keeps the token it made for itself.
pub(crate) fn token_file(address: &str) -> std::path::PathBuf {
    kept(address, "token")
}

fn kept(address: &str, what: &str) -> std::path::PathBuf {
    home().join(format!("daemon-{}.{what}", address.replace([':', '/', '\\'], "-")))
}

/// The token this daemon requires: the one set, else one of its own, written
/// where only this user reads it, so that another user of the machine cannot
/// drive this one's browser through the loopback port.
fn own_token(address: &str) -> Result<String> {
    if let Some(token) = http::token()? {
        return Ok(token);
    }
    let mut random = [0u8; 24];
    getrandom::fill(&mut random)
        .map_err(|error| Error::Tool(format!("no randomness for a token: {error}")))?;
    let token: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    let path = token_file(address);
    std::fs::create_dir_all(home())?;
    let _ = std::fs::remove_file(&path);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    std::io::Write::write_all(&mut options.open(&path)?, token.as_bytes())?;
    Ok(token)
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
    crate::env::check()?;
    crate::api::client::may_start_browser()?;
    let settings = settings()?;
    let limits = http::Limits::from_env()?;
    let listener = tokio::net::TcpListener::bind(&address).await?;
    // Made once the address is this daemon's, so one that lost the race for it leaves the winner's alone.
    let token = match listener.local_addr()?.ip().is_loopback() {
        true => Some(own_token(&address)?),
        false => http::token()?,
    };
    eprintln!(
        "browser-cmd: build {}, process {}, workspace {}",
        http::BUILD,
        std::process::id(),
        settings.workspace.display()
    );
    server::serve(listener, Host::new(settings), limits, token).await
}
