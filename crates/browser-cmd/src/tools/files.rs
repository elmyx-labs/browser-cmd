//! Upload: handing a page a file from inside the workspace.

use serde_json::Value;

use crate::session::{Session, after};
use crate::tools::args::args;
use crate::{Error, Result};

pub(crate) async fn upload(session: &mut Session, args: Value) -> Result<String> {
    let args = args!(args, Args { r#ref: String, path: String });
    let node = session.node(&args.r#ref)?;
    let path = inside(&session.settings().workspace, &args.path)?;
    let tab = session.current().await?;
    crate::page::dom::set_files(&tab, node, &path).await?;
    after(session, &tab, "Attached the file").await
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
