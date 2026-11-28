//! Entry point: help, the version, one command against the warm daemon, or the daemon under `--serve [ADDR]`.

use crate::api::{cli, command, help, http, server};
use crate::{Error, Host, Result, Settings};
use tokio::net::TcpListener;

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

/// What the daemon says outside any command — that it started, a panic, the
/// error it ended on — is kept here: it has no terminal. One that
/// `BROWSER_CMD_ADDR` names has a log for its address, the default one its own.
pub(crate) fn log() -> std::path::PathBuf {
    match crate::env::var("ADDR") {
        Some(address) => kept(&address, "log"),
        None => default_daemon(&home(), "log"),
    }
}

/// The daemon no `BROWSER_CMD_ADDR` names, one per user beside one for a
/// person's own Chrome, writes its address to `daemon.addr` and holds
/// `daemon.lock` for as long as it serves.
fn default_daemon(home: &std::path::Path, what: &str) -> std::path::PathBuf {
    home.join(format!("{}.{what}", if http::mine() { "daemon-mine" } else { "daemon" }))
}

/// Where the default daemon said it listens, while the daemon that said so
/// still holds its lock: the system lets go of a lock when its holder dies, so
/// an address left by a daemon that crashed is no address at all.
pub(crate) fn recorded() -> Option<String> {
    recorded_in(&home())
}

fn recorded_in(home: &std::path::Path) -> Option<String> {
    let lock = std::fs::File::open(default_daemon(home, "lock")).ok()?;
    if lock.try_lock().is_ok() {
        return None;
    }
    let address = std::fs::read_to_string(default_daemon(home, "addr")).ok()?;
    Some(address.trim().to_owned()).filter(|address| !address.is_empty())
}

/// The default daemon's listener: its usual port, or one the system hands out
/// when another program has that, its address written for clients to find,
/// and the lock it holds while it serves. A second default daemon finds the
/// lock taken and leaves, as one that lost a port would.
async fn listen_default(home: &std::path::Path, usual: &str) -> Result<(TcpListener, std::fs::File)> {
    std::fs::create_dir_all(home)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(default_daemon(home, "lock"))?;
    if lock.try_lock().is_err() {
        return Err(Error::Config(format!(
            "another browser-cmd daemon is this user's default one, at {}",
            recorded_in(home).unwrap_or_else(|| "an address it has not written yet".into())
        )));
    }
    let listener = match TcpListener::bind(usual).await {
        Ok(listener) => listener,
        Err(_) => TcpListener::bind("127.0.0.1:0").await?,
    };
    let written = default_daemon(home, "addr.new");
    std::fs::write(&written, listener.local_addr()?.to_string())?;
    std::fs::rename(&written, default_daemon(home, "addr"))?;
    Ok((listener, lock))
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
    let named = match argv {
        [serve] if serve == "--serve" => crate::env::var("ADDR"),
        [serve, address] if serve == "--serve" => Some(address.clone()),
        [name, ..] if !name.starts_with('-') => {
            return Err(Error::Config(format!("unknown command {name}; see --help")));
        }
        _ => return Err(Error::Config(format!("unexpected arguments: {}; see --help", argv.join(" ")))),
    };
    crate::env::check()?;
    crate::api::client::may_start_browser()?;
    let settings = settings()?;
    let limits = http::Limits::from_env()?;
    // Held until the daemon ends: a client trusts the written address only while it is.
    let (listener, _lock) = match &named {
        Some(address) => (TcpListener::bind(address).await?, None),
        None => {
            listen_default(&home(), http::usual()).await.map(|(listener, lock)| (listener, Some(lock)))?
        }
    };
    let address = match named {
        Some(address) => address,
        None => listener.local_addr()?.to_string(),
    };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_default_daemon_takes_a_free_port_when_its_usual_one_is_another_programs() {
        let home = tempfile::tempdir().unwrap();
        let other = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let usual = other.local_addr().unwrap().to_string();
        assert_eq!(recorded_in(home.path()), None, "no daemon has written anything");
        let (listener, lock) = listen_default(home.path(), &usual).await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        assert_ne!(address, usual, "the usual port belongs to another program");
        assert_eq!(recorded_in(home.path()).as_deref(), Some(address.as_str()));
        let second = listen_default(home.path(), &usual).await;
        assert!(second.is_err_and(|error| error.to_string().contains(&address)), "one default daemon a user");
        drop((listener, lock));
        assert_eq!(recorded_in(home.path()), None, "an address whose daemon is gone is no address");
        drop(other);
        let (listener, _lock) = listen_default(home.path(), &usual).await.unwrap();
        assert_eq!(listener.local_addr().unwrap().to_string(), usual, "a free usual port is taken");
    }
}
