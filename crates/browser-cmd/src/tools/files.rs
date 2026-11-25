//! Upload: handing a page a file from inside the workspace.

use std::time::Duration;

use serde_json::Value;

use crate::chrome::tab::Tab;
use crate::page::js::FILE_INPUT;
use crate::session::{Session, after};
use crate::tools::aim::aim;
use crate::tools::args::args;
use crate::{Error, Result};

pub(crate) async fn upload(session: &mut Session, args: Value) -> Result<String> {
    let args = args!(args, Args { r#ref: String, path: String });
    let node = session.node(&args.r#ref)?;
    let path = inside(&session.settings().workspace, &args.path)?;
    let tab = session.current().await?;
    let input =
        session.document.world.call_on(&tab, node, FILE_INPUT, Vec::new()).await? == Value::Bool(true);
    let asked = choose(session, &tab, node, input).await?;
    // The ref is the input itself, or the button a page puts in front of an
    // input it hides or makes on the spot: then the browser names the input.
    let Some(input) = input.then_some(node).or(asked) else {
        return Err(Error::Tool(format!(
            "cannot upload through {}: it is not a file input, and clicking it asked for no file. Name the \
             control that opens the file chooser",
            args.r#ref
        )));
    };
    crate::page::dom::set_files(&tab, input, &path).await?;
    after(session, &tab, "Attached the file").await
}

/// How long a click gets to ask for its file.
const CHOOSER: Duration = Duration::from_secs(2);

/// Click the control and take a person's moment over the chooser, where it is
/// one a person could click: a file that appears in an input nobody clicked is
/// one no person put there. The chooser itself is kept from opening. A file
/// input a person could not click is left to be handed its file directly.
/// Returns the input the browser says asked for the file.
async fn choose(session: &mut Session, tab: &Tab, node: i64, input: bool) -> Result<Option<i64>> {
    let (x, y) = match aim(session, tab, node).await {
        Ok(point) => point,
        Err(_) if input => return Ok(None),
        Err(error) => return Err(error),
    };
    crate::page::dom::intercept_chooser(tab, true).await?;
    let clicked = session.hand.click(tab, x, y).await;
    let until = tokio::time::Instant::now() + CHOOSER;
    let mut opened = None;
    while clicked.is_ok() && opened.is_none() && tokio::time::Instant::now() < until {
        opened = tab.cdp.events().chooser_opened(&tab.session);
        if opened.is_none() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    let released = crate::page::dom::intercept_chooser(tab, false).await;
    clicked?;
    released?;
    if opened.is_some() {
        session.hand.think().await;
    }
    Ok(opened.flatten())
}

/// `join` on an absolute path throws the workspace away and `..` walks out of
/// it, so the resolved path must still be under the workspace: a page whose
/// text the model reads must not pick which local file it is handed.
fn inside(workspace: &std::path::Path, path: &str) -> Result<std::path::PathBuf> {
    let refuse = |what: String| Error::Tool(format!("cannot upload {path}: {what}"));
    let full = workspace.join(path).canonicalize().map_err(|error| refuse(error.to_string()))?;
    let root = workspace.canonicalize().map_err(|error| refuse(error.to_string()))?;
    if !full.starts_with(&root) {
        return Err(refuse("a file to upload must be inside the workspace".into()));
    }
    Ok(full)
}

#[cfg(test)]
mod upload_tests {
    use super::inside;

    #[test]
    fn a_file_outside_the_workspace_is_not_uploaded_however_it_is_spelled() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("work");
        std::fs::create_dir_all(workspace.join("sub")).unwrap();
        std::fs::write(workspace.join("sub/ok.txt"), b"mine").unwrap();
        std::fs::write(temp.path().join("secret.txt"), b"not mine").unwrap();

        assert!(inside(&workspace, "sub/ok.txt").is_ok());
        // `join` on an absolute path keeps only the absolute path.
        let outside = temp.path().join("secret.txt");
        let absolute = inside(&workspace, outside.to_str().unwrap()).unwrap_err().to_string();
        assert!(absolute.contains("inside the workspace"), "{absolute}");
        // And walking out of it is the same thing spelled differently.
        let walked = inside(&workspace, "../secret.txt").unwrap_err().to_string();
        assert!(walked.contains("inside the workspace"), "{walked}");
        assert!(inside(&workspace, "sub/missing.txt").is_err());
    }
}
