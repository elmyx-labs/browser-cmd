//! What a command hands back: its outcome, the picture it took, and the
//! numbered files pictures are kept in.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::chrome::tab::Tab;
use crate::page::world::World;
use crate::{Result, Settings};

/// `error` names the kind of failure, so "the browser never answered" is never
/// mistaken for "the browser said no".
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Outcome {
    pub ok: bool,
    #[serde(rename = "text")]
    pub output: String,
    pub images: Vec<Image>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Image {
    pub media_type: String,
    pub data: String,
}

impl Outcome {
    pub(crate) fn failed(output: String, kind: &str) -> Self {
        Self { output, error: Some(kind.to_owned()), ..Self::default() }
    }

    /// The command reached the browser and no answer came back: repeating it is
    /// how one order becomes two.
    pub(crate) fn unacknowledged(&self) -> bool {
        self.error.as_deref() == Some("unacknowledged")
    }
}

pub(crate) struct Output {
    /// A screenshot taken by the command in progress, for the model to see.
    pub(crate) picture: Option<Image>,
    pub(crate) next_shot: u64,
}

impl Output {
    pub(crate) fn new() -> Self {
        Self { picture: None, next_shot: 1 }
    }

    /// Capture the viewport into the picture the outcome will carry, best
    /// effort — used when the page has nothing the accessibility tree can show —
    /// and say where it was saved.
    pub(crate) async fn glance(
        &mut self,
        world: &mut World,
        tab: &Tab,
        settings: &Settings,
    ) -> Option<PathBuf> {
        let data =
            crate::page::view::capture(world, tab, crate::page::view::Area::Viewport, 1.0).await.ok()?;
        let saved =
            crate::text::base64::unbase64(&data).and_then(|bytes| self.save_shot(settings, &bytes).ok());
        self.picture = Some(Image { media_type: "image/png".into(), data });
        saved
    }

    /// Keep a picture of the tab beside the workspace's other media, under the
    /// next free name, and carry the counter on from there.
    pub(crate) fn save_shot(&mut self, settings: &Settings, bytes: &[u8]) -> Result<PathBuf> {
        let (path, number) = crate::workspace::spill(settings, "browser", "png", self.next_shot, bytes)?;
        self.next_shot = number + 1;
        Ok(path)
    }
}
