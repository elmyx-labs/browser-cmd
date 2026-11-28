//! `doctor`: what a command would run into, checked without starting a
//! browser — the build, the Chrome it would launch, whether that Chrome gets
//! a window, and the daemon.

use crate::api::{app, client, http};
use crate::chrome::discover::chrome_to_launch;
use crate::{Error, Result};

fn chrome(settings: &crate::Settings) -> Result<String> {
    if let Some(endpoint) = &settings.attach {
        return Ok(format!("attaches to the one at {endpoint}"));
    }
    if settings.mine {
        return Ok("yours, through its Remote Debugging switch".into());
    }
    let executable = chrome_to_launch(settings.executable.as_deref())?;
    let window = match settings.headless {
        true => "headless",
        false => "with a window",
    };
    Ok(format!("{}, {window}, profile {}", executable.display(), settings.profile.display()))
}

async fn daemon(address: &str) -> Result<String> {
    Ok(match client::daemon_at(address, &http::Limits::from_env()?).await? {
        Some(build) if build == http::SERVER => format!("up at {address}, this build"),
        Some(build) if build.starts_with("browser-cmd/") => {
            format!("up at {address} as {build}; the next command replaces it with this build")
        }
        Some(_) if crate::env::var("ADDR").is_some() => {
            return Err(Error::Tool(format!(
                "{address} is taken by something that is not browser-cmd; set BROWSER_CMD_ADDR"
            )));
        }
        Some(_) => format!("down ({address} is another program's; a command starts one on a free port)"),
        None => format!("down (nothing at {address}; a command starts one)"),
    })
}

/// Every line is printed, a failed one as what to do about it; the command
/// fails if any did.
pub(crate) async fn run() -> Result<()> {
    let address = http::address();
    let executable = std::env::current_exe().map(|path| path.display().to_string()).unwrap_or_default();
    println!("build:   {} at {executable}", http::BUILD);
    let checks = [
        ("chrome", app::settings().and_then(|settings| chrome(&settings))),
        ("daemon", daemon(&address).await),
    ];
    for (name, found) in &checks {
        match found {
            Ok(found) => println!("{:<8} {found}", format!("{name}:")),
            Err(error) => println!("{:<8} PROBLEM: {error}", format!("{name}:")),
        }
    }
    println!("log:     {}", app::log().display());
    match checks.iter().filter(|(_, found)| found.is_err()).count() {
        0 => Ok(()),
        problems => Err(Error::Tool(format!("doctor found {problems} problem(s), named above"))),
    }
}
